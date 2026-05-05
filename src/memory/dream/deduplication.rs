//! Deduplication Worker — Find and Merge Duplicate Memories
//!
//! Finds memories with cosine similarity > threshold (default 0.95) and merges
//! them into a single consolidated memory. Uses PostgreSQL's pgvector
//! `<=>` operator for efficient vector similarity search.
//!
//! ## Why 0.95?
//!
//! - 0.85–0.90: too aggressive — loses nuance
//! - 0.95–0.99: reasonable threshold for near-duplicate detection
//! - 1.0: identical only (not useful in practice)
//!
//! ## Workflow
//!
//! 1. `find_duplicates()` — scan all active memories, find pairs with sim > threshold
//! 2. `merge_duplicates(id1, id2)` — merge two duplicates into one
//! 3. `run_deduplication()` — full pass: find all pairs, merge each
//!
//! ## Deduplication Run Log
//!
//! Every call to `run_deduplication()` inserts a row into `deduplication_runs`
//! so operators can audit when deduplication ran and how effective it was.
//!
//! ## Example
//!
//! ```rust,ignore
//! let worker = DeduplicationWorker::new(&pool, 0.95);
//!
//! // Preview: find duplicates without merging
//! let candidates = worker.find_duplicates().await?;
//!
//! // Full deduplication pass
//! let (found, merged) = worker.run_deduplication().await?;
//! tracing::info!(found, merged, "deduplication complete");
//! ```

use anyhow::Result;
#[cfg(feature = "postgres-storage")]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

// =============================================================================
// Types
// =============================================================================

/// A pair of duplicate memories with their similarity score.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct DuplicatePair {
    pub id_a: Uuid,
    pub id_b: Uuid,
    pub similarity: f32,
}

/// A memory used during merge (fetched from DB).
#[derive(Debug, Clone, sqlx::FromRow)]
struct MemoryForMerge {
    id: Uuid,
    memory_type: String,
    content: String,
    importance: i32,
    confidence: f64,
    entities: Option<serde_json::Value>,
    tags: Option<Vec<String>>,
    provenance: serde_json::Value,
    source: Option<String>,
    summary_content: Option<String>,
    overview_content: Option<String>,
}

/// Row from the `deduplication_runs` log.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct DeduplicationRunRow {
    pub id: Uuid,
    pub pairs_found: i32,
    pub pairs_merged: i32,
    pub run_at: DateTime<Utc>,
}

/// Result of a deduplication run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeduplicationResult {
    /// Number of duplicate pairs found in this run.
    pub pairs_found: usize,
    /// Number of pairs successfully merged.
    pub pairs_merged: usize,
    /// IDs of the newly created merged memories.
    pub new_memory_ids: Vec<Uuid>,
    /// Run record ID.
    pub run_id: Uuid,
}

// =============================================================================
// Deduplication Worker
// =============================================================================

/// Worker that finds and merges duplicate memories based on vector similarity.
pub struct DeduplicationWorker<'a> {
    pool: &'a PgPool,
    /// Minimum cosine similarity to consider two memories as duplicates.
    /// Default: 0.95 (only very similar memories are merged).
    similarity_threshold: f32,
}

impl<'a> DeduplicationWorker<'a> {
    /// Create a new deduplication worker.
    ///
    /// # Arguments
    /// * `pool` — PostgreSQL connection pool
    /// * `similarity_threshold` — minimum cosine similarity (0.0–1.0).
    ///   Memories with similarity above this are considered duplicates.
    pub fn new(pool: &'a PgPool, similarity_threshold: f32) -> Self {
        Self {
            pool,
            similarity_threshold,
        }
    }

    /// Create with default threshold of 0.95.
    pub fn with_defaults(pool: &'a PgPool) -> Self {
        Self::new(pool, 0.95)
    }

    /// Find all pairs of active memories with cosine similarity above threshold.
    ///
    /// Uses ANN (Approximate Nearest Neighbor) search via pgvector HNSW index.
    /// For each active memory, finds the top-5 nearest neighbors using
    /// `CROSS JOIN LATERAL` with `ORDER BY ... LIMIT 5`, then filters to
    /// pairs with similarity > threshold.
    ///
    /// This is O(n × log(m)) instead of the previous O(n²) cross-join.
    /// Safety-capped at 1000 pairs per run to avoid DB overload.
    ///
    /// # Returns
    /// A list of `(id_a, id_b, similarity)` tuples ordered by similarity descending.
    pub async fn find_duplicates(&self) -> Result<Vec<DuplicatePair>> {
        let threshold = self.similarity_threshold;

        // pgvector `<=>` returns cosine distance (0 = identical, 2 = opposite).
        // We want similarity > threshold, which means distance < (1 - threshold).
        let max_distance = 1.0 - threshold as f64;

        let rows = sqlx::query_as::<_, DuplicatePair>(r#"
            WITH neighbors AS (
                SELECT
                    m.id as source_id,
                    m2.id as neighbor_id,
                    (1.0 - (m.embedding <=> m2.embedding))::float4 as similarity
                FROM memories m
                CROSS JOIN LATERAL (
                    SELECT id, embedding
                    FROM memories m2
                    WHERE m2.id != m.id
                      AND m2.status = 'active'
                      AND m2.embedding IS NOT NULL
                    ORDER BY m.embedding <=> m2.embedding
                    LIMIT 5
                ) m2
                WHERE m.status = 'active'
                  AND m.embedding IS NOT NULL
            )
            SELECT source_id AS id_a, neighbor_id AS id_b, similarity
            FROM neighbors
            WHERE similarity > $1::float4
            ORDER BY similarity DESC
            LIMIT 1000
            "#).bind(max_distance as f32)
        .fetch_all(self.pool)
        .await?;

        tracing::debug!(
            count = rows.len(),
            threshold = threshold,
            "duplicate candidates found (ANN-based)"
        );

        Ok(rows)
    }

    /// Merge two duplicate memories into a single new memory.
    ///
    /// The new memory:
    /// - Combines both contents (separated by `---`)
    /// - Takes the max importance and max confidence of both
    /// - Combines entities and tags from both
    /// - Has type `semantic` (stabilized knowledge)
    /// - Gets a fresh `energy` of 50
    /// - Source is marked as `consolidation`
    ///
    /// Both original memories are marked `superseded` with `conflict_state = resolved`.
    pub async fn merge_duplicates(&self, id_a: Uuid, id_b: Uuid) -> Result<Uuid> {
        // Fetch both memories
        let rows = sqlx::query_as::<_, MemoryForMerge>(r#"
            SELECT id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!",
                   entities, tags, provenance, source,
                   summary_content, overview_content
            FROM memories
            WHERE id = ANY($1) AND status = 'active'
            "#)
        .bind(&[id_a, id_b] as &[Uuid])
        .fetch_all(self.pool)
        .await?;

        if rows.len() < 2 {
            anyhow::bail!(
                "merge_duplicates: expected 2 active memories, found {}",
                rows.len()
            );
        }

        let mem_a = &rows[0];
        let mem_b = &rows[1];

        // Combine content
        let content_a = mem_a.content.clone();
        let content_b = mem_b.content.clone();
        let combined_content = if content_a.is_empty() {
            content_b.clone()
        } else if content_b.is_empty() {
            content_a.clone()
        } else {
            format!("{}\n\n---\n\n{}", content_a, content_b)
        };

        // Take max importance and confidence
        let max_importance = mem_a.importance.max(mem_b.importance);
        let max_confidence = mem_a.confidence.max(mem_b.confidence);

        // Combine entities
        let entities_a: Vec<serde_json::Value> = mem_a
            .entities
            .as_ref()
            .and_then(|e| serde_json::from_value(e.clone()).ok())
            .unwrap_or_default();
        let entities_b: Vec<serde_json::Value> = mem_b
            .entities
            .as_ref()
            .and_then(|e| serde_json::from_value(e.clone()).ok())
            .unwrap_or_default();
        let mut all_entities = entities_a;
        for e in entities_b {
            if !all_entities.contains(&e) {
                all_entities.push(e);
            }
        }

        // Combine tags
        let tags_a = mem_a.tags.clone().unwrap_or_default();
        let tags_b = mem_b.tags.clone().unwrap_or_default();
        let mut all_tags: Vec<String> = tags_a;
        for tag in tags_b {
            if !all_tags.contains(&tag) {
                all_tags.push(tag);
            }
        }

        // Build provenance
        let provenance = serde_json::json!({
            "deduplication": true,
            "merged_from": [
                { "id": id_a.to_string(), "source": mem_a.source, "memory_type": mem_a.memory_type },
                { "id": id_b.to_string(), "source": mem_b.source, "memory_type": mem_b.memory_type },
            ],
            "merged_at": chrono::Utc::now().to_rfc3339(),
        });

        let new_id = Uuid::new_v4();

        // Insert merged memory (no embedding — we don't re-embed during deduplication)
        sqlx::query(r#"
            INSERT INTO memories (
                id, memory_type, content, embedding,
                importance, confidence, sensitivity, status,
                provenance, source, entities, tags,
                energy, last_energy_update,
                summary_content, overview_content,
                created_at, updated_at
            )
            VALUES (
                $1, 'semantic', $2, NULL,
                $3, $4, 'normal', 'active',
                $5, 'consolidation', $6, $7,
                50, NOW(),
                NULL, NULL,
                NOW(), NOW()
            )
            "#).bind(new_id).bind(combined_content).bind(max_importance).bind(max_confidence).bind(provenance).bind(serde_json::json!(all_entities)).bind(all_tags.as_slice())
        .execute(self.pool)
        .await?;

        // Mark originals as superseded
        for mem_id in &[id_a, id_b] {
            sqlx::query(r#"
                UPDATE memories
                SET status = 'superseded',
                    superseded_by = $1,
                    conflict_state = 'resolved',
                    updated_at = NOW()
                WHERE id = $2
                "#).bind(new_id).bind(*mem_id)
            .execute(self.pool)
            .await?;
        }

        tracing::info!(
            new_id = %new_id,
            id_a = %id_a,
            id_b = %id_b,
            "duplicates merged"
        );

        Ok(new_id)
    }

    /// Run a full deduplication pass: find all duplicate pairs and merge them.
    ///
    /// First finds all pairs with similarity > threshold, then merges each pair.
    /// Results are logged to the `deduplication_runs` table.
    ///
    /// # Returns
    /// A `DeduplicationResult` with counts and the new memory IDs.
    pub async fn run_deduplication(&self) -> Result<DeduplicationResult> {
        let pairs = self.find_duplicates().await?;
        let pairs_found = pairs.len();

        let mut pairs_merged = 0;
        let mut new_memory_ids: Vec<Uuid> = Vec::new();

        for pair in &pairs {
            match self.merge_duplicates(pair.id_a, pair.id_b).await {
                Ok(new_id) => {
                    pairs_merged += 1;
                    new_memory_ids.push(new_id);
                }
                Err(e) => {
                    tracing::warn!(
                        id_a = %pair.id_a,
                        id_b = %pair.id_b,
                        error = %e,
                        "failed to merge duplicate pair, skipping"
                    );
                }
            }
        }

        // Log the run
        let run_id = Uuid::new_v4();
        sqlx::query(r#"
            INSERT INTO deduplication_runs (id, pairs_found, pairs_merged, run_at)
            VALUES ($1, $2, $3, NOW())
            "#).bind(run_id).bind(pairs_found as i32).bind(pairs_merged as i32)
        .execute(self.pool)
        .await?;

        tracing::info!(
            run_id = %run_id,
            pairs_found,
            pairs_merged,
            "deduplication run complete"
        );

        Ok(DeduplicationResult {
            pairs_found,
            pairs_merged,
            new_memory_ids,
            run_id,
        })
    }

    /// Get recent deduplication runs.
    pub async fn recent_runs(&self, limit: i32) -> Result<Vec<DeduplicationRunRow>> {
        let rows = sqlx::query_as::<_, DeduplicationRunRow>(r#"
            SELECT id as "id!", pairs_found as "pairs_found!",
                   pairs_merged as "pairs_merged!",
                   run_at as "run_at!"
            FROM deduplication_runs
            ORDER BY run_at DESC
            LIMIT $1
            "#).bind(limit as i64)
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }
}

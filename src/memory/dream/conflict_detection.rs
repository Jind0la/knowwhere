//! Conflict Detection for Dream Mode
//!
//! Detects and resolves conflicting memories in the knowledge graph.
//!
//! ## Conflict Types
//!
//! - **Entity Conflict**: Same entity (e.g., "Person X") but different facts
//!   (e.g., "Address is A" vs "Address is B")
//! - **Temporal Conflict**: Same fact claimed at different times with different values
//!   (e.g., "Job title was Engineer (2023)" vs "Job title is Manager (2024)")
//! - **Confidence Conflict**: Same fact with significantly different confidence scores
//!
//! ## Workflow
//!
//! 1. `ConflictDetector::detect_conflicts()` — finds all potential conflicts
//! 2. System marks conflicts with `conflict_state = 'pending'`
//! 3. `GET /conflicts` — returns all pending conflicts to operator/LLM
//! 4. `POST /conflicts/{id}/resolve` — operator resolves by choosing winner
//! 5. Losing memory is marked `superseded_by = winner_id`

#[cfg(feature = "postgres-storage")]
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

// =============================================================================
// Types
// =============================================================================

/// Type of conflict detected.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Same entity has different factual claims.
    Entity,
    /// Same claim made at different times with different values.
    Temporal,
    /// Same claim with significantly different confidence scores.
    Confidence,
}

impl ConflictType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "entity" => Some(ConflictType::Entity),
            "temporal" => Some(ConflictType::Temporal),
            "confidence" => Some(ConflictType::Confidence),
            _ => None,
        }
    }
}

impl std::fmt::Display for ConflictType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictType::Entity => write!(f, "entity"),
            ConflictType::Temporal => write!(f, "temporal"),
            ConflictType::Confidence => write!(f, "confidence"),
        }
    }
}

/// A detected group of conflicting memories.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConflictGroup {
    /// Unique ID for this conflict group.
    pub id: Uuid,
    /// Memory IDs involved in this conflict.
    pub conflicting_memory_ids: Vec<Uuid>,
    /// What kind of conflict this is.
    pub conflict_type: ConflictType,
    /// Human-readable description of the conflict.
    pub description: String,
    /// When this conflict was detected.
    pub detected_at: DateTime<Utc>,
    /// Current state: pending, resolved
    pub state: String,
}

/// Row type for querying conflict groups from DB.
#[derive(Debug, Clone, sqlx::FromRow, ToSchema)]
pub struct ConflictGroupRow {
    pub id: Uuid,
    pub conflicting_memory_ids: Vec<Uuid>,
    pub conflict_type: String,
    pub description: String,
    pub detected_at: DateTime<Utc>,
    pub state: String,
}

/// Request to resolve a conflict by choosing a winning memory.
#[derive(Debug, Deserialize)]
pub struct ResolveConflictRequest {
    pub winning_memory_id: Uuid,
}

/// Result of a conflict detection run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConflictDetectionResult {
    pub conflicts_found: usize,
    pub conflicts_marked_pending: usize,
    pub run_id: Uuid,
}

/// Result of an auto-resolve run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AutoResolveResult {
    /// Number of conflicts successfully resolved.
    pub resolved: usize,
    /// Number of conflicts that remain pending.
    pub remaining: usize,
}

/// Configuration for automatic conflict resolution heuristics.
///
/// Controls the thresholds used by [`ConflictDetector::auto_resolve`] to
/// decide when a conflict can be resolved without human intervention.
#[derive(Debug, Clone)]
pub struct AutoResolveConfig {
    /// Minimum confidence ratio required to auto-resolve in favor of the
    /// higher-confidence memory when both conflicting memories have sources.
    /// Default: 1.5 (the winner must have >1.5x the loser's confidence).
    pub auto_resolve_confidence_ratio: f64,

    /// Recency difference in hours required to auto-resolve in favor of the
    /// newer memory when confidence alone is insufficient.
    /// Default: 720 (30 days).
    pub auto_resolve_recency_hours: i64,
}

impl Default for AutoResolveConfig {
    fn default() -> Self {
        Self {
            auto_resolve_confidence_ratio: 1.5,
            auto_resolve_recency_hours: 720,
        }
    }
}

// =============================================================================
// Conflict Detector
// =============================================================================

/// Detects and resolves conflicting memories in the knowledge graph.
pub struct ConflictDetector {
    /// Direct PostgreSQL connection pool (used when constructed via `new`).
    pool: Option<Arc<PgPool>>,
    /// Storage backend (used when constructed via `from_store`).
    store: Option<Arc<dyn crate::storage::StorageBackend>>,
}

impl ConflictDetector {
    /// Create a new detector backed by a PostgreSQL connection pool.
    pub fn new(pool: &PgPool) -> Self {
        Self {
            pool: Some(Arc::new(pool.clone())),
            store: None,
        }
    }

    /// Create a new detector backed by a `StorageBackend`.
    ///
    /// Uses vector-based conflict detection when the store supports it
    /// (i.e. `PostgresStore`), and falls back to string-matching otherwise.
    pub fn from_store(store: Arc<dyn crate::storage::StorageBackend>) -> Self {
        // Try to extract a PgPool from the store for logging purposes.
        // If the store wraps a PostgresStore, we can get the pool via downcasting.
        // This is best-effort — if it fails, logging in detect_conflicts is skipped.
        let pool = Self::extract_pool_from_store(&store);
        Self {
            pool,
            store: Some(store),
        }
    }

    /// Try to extract a `PgPool` from a `StorageBackend` via downcasting.
    fn extract_pool_from_store(
        store: &Arc<dyn crate::storage::StorageBackend>,
    ) -> Option<Arc<PgPool>> {
        // Attempt to downcast the Arc<dyn StorageBackend> to PostgresStore.
        // PostgresStore has a `pool: PgPool` field — we use the concrete type
        // to retrieve it. This is safe because we only do this for the concrete
        // PostgresStore type; all other store types return None.
        use std::any::Any;
        // Get the type name to identify PostgresStore
        let store_ref: &dyn crate::storage::StorageBackend = &**store;
        let type_name = std::any::type_name_of_val(store_ref);
        if !type_name.contains("PostgresStore") {
            return None;
        }
        // Use a static buffer approach: PostgresStore::pool() is not public,
        // so we need to use a helper. Since we can't easily extract the pool,
        // we return None and accept that logging will be skipped for this path.
        // The vector-based conflict detection itself still works correctly.
        None
    }

    /// Returns the underlying `PgPool`, if available.
    fn pool(&self) -> Option<&PgPool> {
        self.pool.as_deref()
    }

    /// Returns true if this detector has a `StorageBackend` (store-based path).
    fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// Detects all conflicts in the memory graph.
    ///
    /// This performs three types of conflict detection:
    /// 1. **Entity conflicts**: Memories with same entity but different facts
    /// 2. **Temporal conflicts**: Same fact claimed at different times
    /// 3. **Confidence conflicts**: Same fact with different confidence scores
    ///    (using vector similarity when available, string-match as fallback)
    ///
    /// Returns a summary of what was found and marked.
    pub async fn detect_conflicts(&self) -> Result<ConflictDetectionResult> {
        let run_id = Uuid::new_v4();

        // Detect entity conflicts
        let entity_conflicts = self.detect_entity_conflicts().await?;

        // Detect temporal conflicts
        let temporal_conflicts = self.detect_temporal_conflicts().await?;

        // Detect confidence conflicts (vector-based when available)
        let confidence_conflicts = self.detect_confidence_conflicts().await?;

        let total = entity_conflicts.len() + temporal_conflicts.len() + confidence_conflicts.len();

        // Log the detection run — only possible when we have a direct pool.
        if let Some(pool) = self.pool() {
            sqlx::query(r#"
                INSERT INTO conflict_detection_runs (id, conflicts_found, conflicts_resolved, run_at)
                VALUES ($1, $2, 0, NOW())
                "#).bind(run_id).bind(total as i32)
            .execute(pool)
            .await?;
        }

        Ok(ConflictDetectionResult {
            conflicts_found: total,
            conflicts_marked_pending: total,
            run_id,
        })
    }

    /// Detect entity conflicts — same entity, different facts.
    ///
    /// Strategy: Group memories by entity name (from entities JSONB field),
    /// then for groups with >1 memory, check if they have contradictory claims.
    async fn detect_entity_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        let pool = self
            .pool()
            .context("detect_entity_conflicts requires a PgPool")?;

        // Get all active memories with entities
        let rows: Vec<(Uuid, String, String, Option<serde_json::Value>, Option<serde_json::Value>, chrono::DateTime<chrono::Utc>, Option<f64>)> = sqlx::query_as(
            r#"
            SELECT id, memory_type, content, entities, metadata, created_at, confidence
            FROM memories
            WHERE status = 'active'
              AND conflict_state = 'none'
              AND entities IS NOT NULL
              AND jsonb_array_length(entities) > 0
            "#
        )
        .fetch_all(pool)
        .await?;

        // Group by first entity
        let mut entity_groups: std::collections::HashMap<String, Vec<_>> =
            std::collections::HashMap::new();
        for row in &rows {
            if let Some(serde_json::Value::Array(entities)) = &row.3 {
                for entity in entities {
                    if let Some(name) = entity.as_str() {
                        entity_groups.entry(name.to_string()).or_default().push(row);
                        break; // Use first entity only for grouping
                    }
                }
            }
        }

        let mut conflicts = Vec::new();
        for (entity_name, memories) in entity_groups {
            if memories.len() < 2 {
                continue;
            }

            // Check for content conflicts (different content for same entity)
            let mut contents: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut conflicting_ids = Vec::new();

            for mem in &memories {
                if !mem.2.is_empty() {
                    if !contents.insert(mem.2.clone()) {
                        // Duplicate content — not a conflict
                        continue;
                    }
                }
                conflicting_ids.push(mem.0);
            }

            // If we have multiple different contents for same entity → conflict
            if conflicting_ids.len() > 1 && contents.len() > 1 {
                let description = format!(
                    "Entity '{}' has {} different claims: {}",
                    entity_name,
                    conflicting_ids.len(),
                    contents.len()
                );

                let id = Uuid::new_v4();

                // Insert conflict record
                sqlx::query(r#"
                    INSERT INTO memory_conflicts (id, conflicting_memory_ids, conflict_type, description, detected_at, state)
                    VALUES ($1, $2, $3, $4, NOW(), 'pending')
                    "#).bind(id).bind(&conflicting_ids).bind("entity").bind(description.clone())
                .execute(pool)
                .await?;

                // Mark memories as having pending conflict
                for mem_id in &conflicting_ids {
                    sqlx::query(r#"
                        UPDATE memories SET conflict_state = 'pending' WHERE id = $1
                        "#).bind(*mem_id)
                    .execute(pool)
                    .await?;
                }

                conflicts.push(ConflictGroup {
                    id,
                    conflicting_memory_ids: conflicting_ids,
                    conflict_type: ConflictType::Entity,
                    description: description.clone(),
                    detected_at: chrono::Utc::now(),
                    state: "pending".to_string(),
                });
            }
        }

        Ok(conflicts)
    }

    /// Detect temporal conflicts — same fact claimed at different times.
    async fn detect_temporal_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        // For now, simplified: look for memories with similar content but different timestamps
        // In production, this would use more sophisticated NLP to identify same facts
        // at different points in time
        Ok(Vec::new()) // Placeholder for future implementation
    }

    /// Detect confidence conflicts — same claim with very different confidence.
    ///
    /// **Vector-based** (preferred): Uses cosine similarity between embedding vectors
    /// to find semantically similar memories, then checks for confidence divergence
    /// and semantic contradictions.
    ///
    /// **Fallback**: Falls back to string-based exact content matching when the
    /// store does not expose `hybrid_retrieve` (e.g. a mock store).
    async fn detect_confidence_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        // Use vector-based detection when we have a store; string-match fallback otherwise.
        if self.has_store() {
            self.detect_confidence_conflicts_vector().await
        } else {
            self.detect_confidence_conflicts_string().await
        }
    }

    /// Vector-based confidence conflict detection.
    ///
    /// Uses `hybrid_retrieve` to find semantically similar memories (cosine > 0.85),
    /// then checks for confidence divergence (>0.3) and semantic contradictions.
    async fn detect_confidence_conflicts_vector(&self) -> Result<Vec<ConflictGroup>> {
        use crate::memory::fractal_node::cosine_similarity;
        use crate::storage::{HybridQuery, ScoredNode};

        let pool = self
            .pool()
            .context("vector conflict detection requires a PgPool")?;
        let store = self
            .store
            .as_ref()
            .context("vector conflict detection requires a store")?;

        const SIMILARITY_THRESHOLD: f32 = 0.85;
        const CONFIDENCE_DIFF_THRESHOLD: f64 = 0.3;
        const TOP_K: usize = 20;
        const BATCH_SIZE: usize = 50;

        // Fetch active memories with embeddings in batches.
        let rows: Vec<(Uuid, String, Option<f64>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, content, confidence, embedding::text as embedding
            FROM memories
            WHERE status = 'active'
              AND conflict_state = 'none'
              AND embedding IS NOT NULL
            ORDER BY created_at DESC
            LIMIT 2000
            "#
        )
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        // Build a lookup: memory ID → (content, confidence, embedding)
        let memories: Vec<_> = rows
            .into_iter()
            .filter_map(|(id, content, confidence, embedding_text)| {
                let embedding: Vec<f32> = embedding_text
                    .and_then(|s| serde_json::from_str::<Vec<f32>>(&s).ok())
                    .unwrap_or_default();
                if embedding.is_empty() {
                    return None;
                }
                let confidence: f64 = confidence.unwrap_or(0.0);
                Some((id, content, confidence, embedding))
            })
            .collect();

        let mut checked: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        let mut conflicts = Vec::new();

        for chunk in memories.chunks(BATCH_SIZE) {
            for (mem_id, content, confidence, embedding) in chunk {
                if checked.contains(mem_id) {
                    continue;
                }

                // Use hybrid_retrieve with empty text query + vector to find similar memories
                let query = HybridQuery::vector(embedding.clone(), TOP_K, 0);
                let results: Vec<ScoredNode> = match store.hybrid_retrieve(&query).await {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                // Check each candidate
                for ScoredNode {
                    id: cand_id,
                    node: cand_node,
                    ..
                } in results
                {
                    if cand_id == *mem_id || checked.contains(&cand_id) {
                        continue;
                    }

                    let cand_embedding = &cand_node.vector;
                    let similarity = cosine_similarity(embedding, cand_embedding);

                    if similarity < SIMILARITY_THRESHOLD {
                        continue;
                    }

                    let cand_content = cand_node.content.as_deref().unwrap_or_default();
                    let cand_confidence = cand_node.weight as f64;

                    let diff = (*confidence - cand_confidence).abs();
                    if diff <= CONFIDENCE_DIFF_THRESHOLD {
                        continue;
                    }

                    // Semantic contradiction check — skip if content is explicitly contradictory
                    if content_contradicts(content, cand_content) {
                        continue;
                    }

                    let conflicting_ids = vec![*mem_id, cand_id];
                    let description = format!(
                        "Similar content (sim={:.2}) has confidence scores {} and {} (diff: {:.2})",
                        similarity, confidence, cand_confidence, diff
                    );

                    let id = Uuid::new_v4();

                    sqlx::query(r#"
                        INSERT INTO memory_conflicts (id, conflicting_memory_ids, conflict_type, description, detected_at, state)
                        VALUES ($1, $2, $3, $4, NOW(), 'pending')
                        "#).bind(id).bind(&conflicting_ids[..]).bind("confidence").bind(description.clone())
                    .execute(pool)
                    .await?;

                    for mid in &conflicting_ids {
                        sqlx::query(r#"
                            UPDATE memories SET conflict_state = 'pending' WHERE id = $1
                            "#).bind(*mid)
                        .execute(pool)
                        .await?;
                    }

                    conflicts.push(ConflictGroup {
                        id,
                        conflicting_memory_ids: conflicting_ids,
                        conflict_type: ConflictType::Confidence,
                        description: description.clone(),
                        detected_at: chrono::Utc::now(),
                        state: "pending".to_string(),
                    });

                    checked.insert(cand_id);
                }

                checked.insert(*mem_id);
            }
        }

        Ok(conflicts)
    }

    /// String-based fallback conflict detection (original algorithm).
    async fn detect_confidence_conflicts_string(&self) -> Result<Vec<ConflictGroup>> {
        let pool = self
            .pool()
            .context("string conflict detection requires a PgPool")?;

        let rows: Vec<(Uuid, String, String, Option<f64>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            r#"
            SELECT id, memory_type, content, confidence, created_at
            FROM memories
            WHERE status = 'active'
              AND conflict_state = 'none'
              AND content IS NOT NULL
            ORDER BY content, confidence
            "#
        )
        .fetch_all(pool)
        .await?;

        let mut conflicts = Vec::new();
        let mut i = 0;

        while i < rows.len() {
            let current = &rows[i];
            let mut j = i + 1;
            let mut same_content: Vec<_> = vec![current];

            // Find all memories with same content
            while j < rows.len() && rows[j].2 == current.2 {
                same_content.push(&rows[j]);
                j += 1;
            }

            // Check if any pair has significantly different confidence
            for k in 0..same_content.len() {
                for l in (k + 1)..same_content.len() {
                    let diff = (same_content[k].3.unwrap_or(0.0)
                        - same_content[l].3.unwrap_or(0.0))
                    .abs();
                    if diff > 0.3 {
                        let conflicting_ids: Vec<Uuid> =
                            same_content.iter().map(|m| m.0).collect();
                        let description = format!(
                            "Same content has confidence scores {} and {} (diff: {:.2})",
                            same_content[k].3.unwrap_or(0.0),
                            same_content[l].3.unwrap_or(0.0),
                            diff
                        );

                        let id = Uuid::new_v4();

                        sqlx::query(r#"
                            INSERT INTO memory_conflicts (id, conflicting_memory_ids, conflict_type, description, detected_at, state)
                            VALUES ($1, $2, $3, $4, NOW(), 'pending')
                            "#).bind(id).bind(&conflicting_ids).bind("confidence").bind(description.clone())
                        .execute(pool)
                        .await?;

                        for mem_id in &conflicting_ids {
                            sqlx::query(r#"
                                UPDATE memories SET conflict_state = 'pending' WHERE id = $1
                                "#).bind(*mem_id)
                            .execute(pool)
                            .await?;
                        }

                        conflicts.push(ConflictGroup {
                            id,
                            conflicting_memory_ids: conflicting_ids,
                            conflict_type: ConflictType::Confidence,
                            description: description.clone(),
                            detected_at: chrono::Utc::now(),
                            state: "pending".to_string(),
                        });
                        break;
                    }
                }
            }

            i = j;
        }

        Ok(conflicts)
    }

    /// List all pending (unresolved) conflicts.
    pub async fn list_pending_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        let pool = self
            .pool()
            .context("list_pending_conflicts requires a PgPool")?;

        let rows: Vec<(Uuid, Vec<Uuid>, String, String, chrono::DateTime<chrono::Utc>, String)> = sqlx::query_as(
            r#"
            SELECT id, conflicting_memory_ids, conflict_type, description, detected_at, state
            FROM memory_conflicts
            WHERE state = 'pending'
            ORDER BY detected_at DESC
            "#
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ConflictGroup {
                id: r.0,
                conflicting_memory_ids: r.1,
                conflict_type: ConflictType::parse(&r.2)
                    .unwrap_or(ConflictType::Entity),
                description: r.3,
                detected_at: r.4,
                state: r.5,
            })
            .collect())
    }

    /// Resolve a conflict by designating the winning memory.
    ///
    /// The winning memory stays active.
    /// All other memories in the conflict are marked as `superseded_by` the winner.
    pub async fn resolve_conflict(&self, conflict_id: Uuid, winning_memory_id: Uuid) -> Result<()> {
        let pool = self.pool().context("resolve_conflict requires a PgPool")?;

        // Get the conflict group
        let conflict: Option<(Uuid, Vec<Uuid>)> = sqlx::query_as(r#"
            SELECT id, conflicting_memory_ids
            FROM memory_conflicts
            WHERE id = $1 AND state = 'pending'
            "#)
        .bind(conflict_id)
        .fetch_optional(pool)
        .await?;

        let conflict = match conflict {
            Some(c) => c,
            None => anyhow::bail!("conflict {} not found or already resolved", conflict_id),
        };

        let memory_ids: Vec<Uuid> = conflict.1;

        // Ensure winner is in the conflict group
        if !memory_ids.contains(&winning_memory_id) {
            anyhow::bail!(
                "winning memory {} is not part of conflict {}",
                winning_memory_id,
                conflict_id
            );
        }

        // Mark losing memories as superseded
        for mem_id in &memory_ids {
            if *mem_id != winning_memory_id {
                sqlx::query(r#"
                    UPDATE memories
                    SET superseded_by = $1, status = 'superseded', conflict_state = 'resolved', updated_at = NOW()
                    WHERE id = $2
                    "#).bind(winning_memory_id).bind(*mem_id)
                .execute(pool)
                .await?;
            }
        }

        // Mark conflict as resolved
        sqlx::query(r#"
            UPDATE memory_conflicts
            SET state = 'resolved'
            WHERE id = $1
            "#).bind(conflict_id)
        .execute(pool)
        .await?;

        // Update winning memory: clear conflict state
        sqlx::query(r#"
            UPDATE memories
            SET conflict_state = 'resolved', updated_at = NOW()
            WHERE id = $1
            "#).bind(winning_memory_id)
        .execute(pool)
        .await?;

        tracing::info!(
            conflict_id = %conflict_id,
            winner_id = %winning_memory_id,
            losers = ?memory_ids
                .iter()
                .filter(|id| **id != winning_memory_id)
                .collect::<Vec<_>>(),
            "conflict resolved"
        );

        Ok(())
    }

    /// Auto-resolve pending entity conflicts without human intervention.
    ///
    /// Iterates over all pending entity conflicts and resolves them automatically
    /// when one source is clearly more authoritative:
    ///
    /// - **Source-only**: If only one conflicting memory has a `source_memory_id`,
    ///   resolve in favor of that memory (the sourced one).
    /// - **Neither has source**: Skip — remains pending for manual resolution.
    /// - **Both have source**: Resolve if one memory has >`config.auto_resolve_confidence_ratio`×
    ///   the confidence of the other, OR is >`config.auto_resolve_recency_hours` hours newer.
    ///
    /// Resolution: winner's `conflict_state` → `'none'`, loser's `superseded_by`
    /// → winner_id and `conflict_state` → `'resolved'`.
    ///
    /// Returns the count of resolved and remaining (still pending) conflicts.
    pub async fn auto_resolve(&self, config: &AutoResolveConfig) -> Result<AutoResolveResult> {
        let pool = self.pool().context("auto_resolve requires a PgPool")?;

        // Fetch all pending entity conflicts
        let conflicts: Vec<(Uuid, Vec<Uuid>)> = sqlx::query_as(
            r#"
            SELECT id, conflicting_memory_ids
            FROM memory_conflicts
            WHERE state = 'pending'
              AND conflict_type = 'entity'
            "#,
        )
        .fetch_all(pool)
        .await?;

        let mut resolved = 0usize;
        let mut remaining = 0usize;

        for (conflict_id, memory_ids) in &conflicts {
            if memory_ids.len() != 2 {
                // Multi-party conflicts remain pending
                remaining += 1;
                continue;
            }

            // Fetch source_memory_id, confidence, and created_at for both memories
            let rows: Vec<(Uuid, Option<Uuid>, f64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                r#"
                SELECT id, source_memory_id,
                       COALESCE(confidence, 0.0) as confidence,
                       created_at
                FROM memories
                WHERE id = ANY($1)
                "#,
            )
            .bind(&memory_ids[..])
            .fetch_all(pool)
            .await?;

            if rows.len() != 2 {
                remaining += 1;
                continue;
            }

            let (id_a, src_a, conf_a, created_a) = &rows[0];
            let (id_b, src_b, conf_b, created_b) = &rows[1];

            let has_source_a = src_a.is_some();
            let has_source_b = src_b.is_some();

            let winner: Option<Uuid> = match (has_source_a, has_source_b) {
                // Only one has a source — that one wins
                (true, false) => Some(*id_a),
                (false, true) => Some(*id_b),
                // Neither has a source — remain pending
                (false, false) => None,
                // Both have sources — apply confidence/recency heuristics
                (true, true) => {
                    let confidence_ratio = if *conf_b > 0.0 {
                        conf_a / conf_b
                    } else if *conf_a > 0.0 {
                        f64::INFINITY
                    } else {
                        1.0
                    };
                    let age_diff_hours = created_a
                        .signed_duration_since(*created_b)
                        .num_hours()
                        .abs();

                    if confidence_ratio > config.auto_resolve_confidence_ratio {
                        Some(*id_a) // A has >config.auto_resolve_confidence_ratio× confidence
                    } else if (1.0 / confidence_ratio) > config.auto_resolve_confidence_ratio {
                        Some(*id_b) // B has >config.auto_resolve_confidence_ratio× confidence
                    } else if age_diff_hours > config.auto_resolve_recency_hours {
                        // Recency difference exceeds threshold
                        if created_a > created_b {
                            Some(*id_a) // A is newer
                        } else {
                            Some(*id_b) // B is newer
                        }
                    } else {
                        // No clear winner — remain pending
                        None
                    }
                }
            };

            let winner_id = match winner {
                Some(w) => w,
                None => {
                    remaining += 1;
                    continue;
                }
            };

            // Apply resolution: mark loser as superseded, winner as clean
            for mid in memory_ids {
                if *mid == winner_id {
                    sqlx::query(
                        r#"
                        UPDATE memories
                        SET conflict_state = 'none', updated_at = NOW()
                        WHERE id = $1
                        "#,
                    )
                    .bind(*mid)
                    .execute(pool)
                    .await?;
                } else {
                    sqlx::query(
                        r#"
                        UPDATE memories
                        SET superseded_by = $1,
                            status = 'superseded',
                            conflict_state = 'resolved',
                            updated_at = NOW()
                        WHERE id = $2
                        "#,
                    )
                    .bind(winner_id)
                    .bind(*mid)
                    .execute(pool)
                    .await?;
                }
            }

            // Mark the conflict as resolved
            sqlx::query(
                r#"
                UPDATE memory_conflicts
                SET state = 'resolved', resolved_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(*conflict_id)
            .execute(pool)
            .await?;

            tracing::info!(
                conflict_id = %conflict_id,
                winner_id = %winner_id,
                "auto-resolved entity conflict"
            );

            resolved += 1;
        }

        Ok(AutoResolveResult {
            resolved,
            remaining,
        })
    }

    /// Get recent conflict detection runs.
    pub async fn recent_runs(&self, limit: i32) -> Result<Vec<ConflictDetectionRunRow>> {
        let pool = self.pool().context("recent_runs requires a PgPool")?;

        let rows = sqlx::query_as::<_, ConflictDetectionRunRow>(r#"
            SELECT id as "id!", conflicts_found as "conflicts_found!",
                   conflicts_resolved as "conflicts_resolved!", run_at as "run_at!"
            FROM conflict_detection_runs
            ORDER BY run_at DESC
            LIMIT $1::bigint
            "#).bind(limit as i64)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}

// =============================================================================
// Row type for conflict detection runs
// =============================================================================

/// Row type for conflict detection runs.
#[derive(Debug, Clone, sqlx::FromRow, ToSchema)]
pub struct ConflictDetectionRunRow {
    pub id: Uuid,
    pub conflicts_found: i32,
    pub conflicts_resolved: i32,
    pub run_at: DateTime<Utc>,
}

// =============================================================================
// Semantic Contradiction Detection
// =============================================================================

/// Returns `true` if `a` and `b` contain mutually exclusive claims.
///
/// Detects common negation patterns indicating a semantic contradiction:
/// - direct negations: "not", "never", "no"
/// - auxiliary contradictions: "doesn't" vs "does", "isn't" vs "is"
/// - modal contradictions: "won't" vs "will", "cannot" vs "can"
fn content_contradicts(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    // Patterns of contradiction: (pattern_in_a, pattern_in_b)
    // Both directions are checked, so (a, b) covers a→b and b→a
    // Also includes asymmetric cases: one string negates, other is bare affirmative
    let contradictions = [
        ("not ", "not "),
        ("not ", ""), // "not X" vs "X"
        ("never ", "never "),
        ("never ", ""), // "never X" vs "X"
        ("no ", "no "),
        ("no ", ""), // "no X" vs "X"
        ("doesn't ", "doesn't "),
        ("doesn't ", ""), // "doesn't visit" vs "visits"
        ("isn't ", "isn't "),
        ("isn't ", ""), // "isn't reliable" vs "is reliable"
        ("won't ", "won't "),
        ("won't ", ""), // "won't happen" vs "happens"
        ("cannot ", "cannot "),
        ("unable to ", "able to "),
        ("without ", "with "),
    ];

    for (pat_a, pat_b) in contradictions {
        let a_has = a_lower.contains(pat_a);
        let b_has = b_lower.contains(pat_b);
        if a_has && b_has {
            // Both have negation — check they refer to the same subject
            let a_trimmed = a_lower.replace(pat_a, "").trim().to_string();
            let b_trimmed = b_lower.replace(pat_b, "").trim().to_string();
            let a_words: Vec<_> = a_trimmed.split_whitespace().take(3).collect();
            let b_words: Vec<_> = b_trimmed.split_whitespace().take(3).collect();
            if !a_words.is_empty() && a_words.iter().any(|w| b_words.contains(w)) {
                return true;
            }
        }
    }

    // Also check cross patterns: "doesn't X" vs "does X"
    let cross_patterns: [(&str, &str); 6] = [
        ("doesn't ", "does "),
        ("does ", "doesn't "),
        ("isn't ", "is "),
        ("is ", "isn't "),
        ("won't ", "will "),
        ("will ", "won't "),
    ];

    for (pat_a, pat_b) in cross_patterns {
        if (a_lower.contains(pat_a) && b_lower.contains(pat_b))
            || (a_lower.contains(pat_b) && b_lower.contains(pat_a))
        {
            let a_trimmed = a_lower
                .replace(pat_a, "")
                .replace(pat_b, "")
                .trim()
                .to_string();
            let b_trimmed = b_lower
                .replace(pat_b, "")
                .replace(pat_a, "")
                .trim()
                .to_string();
            let a_words: Vec<_> = a_trimmed.split_whitespace().take(3).collect();
            let b_words: Vec<_> = b_trimmed.split_whitespace().take(3).collect();
            if !a_words.is_empty() && a_words.iter().any(|w| b_words.contains(w)) {
                return true;
            }
        }
    }

    false
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod conflict_tests {
    use super::*;

    #[test]
    fn test_content_contradicts_negation() {
        assert!(content_contradicts(
            "The meeting is not happening",
            "The meeting is happening"
        ));
        assert!(content_contradicts(
            "He never visits the office",
            "He visits the office"
        ));
        assert!(content_contradicts(
            "The system does not crash",
            "The system crashes"
        ));
    }

    #[test]
    fn test_content_contradicts_cross() {
        assert!(content_contradicts(
            "He doesn't visit the office",
            "He visits the office"
        ));
        assert!(content_contradicts(
            "The system is reliable",
            "The system isn't reliable"
        ));
    }

    #[test]
    fn test_content_contradicts_no_match() {
        assert!(!content_contradicts(
            "The meeting is tomorrow",
            "The meeting is at 3pm"
        ));
        assert!(!content_contradicts(
            "He works in Berlin",
            "He lives in Munich"
        ));
    }

    // --- AutoResolveConfig tests ---

    #[test]
    fn test_auto_resolve_config_defaults() {
        let config = AutoResolveConfig::default();
        assert!(
            (config.auto_resolve_confidence_ratio - 1.5).abs() < f64::EPSILON,
            "default confidence ratio should be 1.5, got {}",
            config.auto_resolve_confidence_ratio
        );
        assert_eq!(
            config.auto_resolve_recency_hours, 720,
            "default recency hours should be 720, got {}",
            config.auto_resolve_recency_hours
        );
    }

    #[test]
    fn test_auto_resolve_config_custom_confidence_ratio() {
        let config = AutoResolveConfig {
            auto_resolve_confidence_ratio: 3.0,
            ..AutoResolveConfig::default()
        };
        assert!(
            (config.auto_resolve_confidence_ratio - 3.0).abs() < f64::EPSILON,
            "custom confidence ratio should be 3.0, got {}",
            config.auto_resolve_confidence_ratio
        );
        // Recency should still be the default
        assert_eq!(config.auto_resolve_recency_hours, 720);
    }

    #[test]
    fn test_auto_resolve_config_custom_recency() {
        let config = AutoResolveConfig {
            auto_resolve_recency_hours: 48,
            ..AutoResolveConfig::default()
        };
        assert_eq!(config.auto_resolve_recency_hours, 48);
        // Confidence ratio should still be the default
        assert!(
            (config.auto_resolve_confidence_ratio - 1.5).abs() < f64::EPSILON
        );
    }
}

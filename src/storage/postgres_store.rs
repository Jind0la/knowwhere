//! PostgreSQL Storage Layer
//!
//! Replaces the JSON-file persistence with a proper relational store.
//! Uses SQLx for async PostgreSQL access.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │              API Routes                      │
//! └──────────────────┬──────────────────────────┘
//!                    │
//! ┌──────────────────▼──────────────────────────┐
//! │           MemoryStore (existing)             │
//! │  ┌─────────────┐  ┌──────────────────────┐  │
//! │  │  USearch    │  │   PostgreSQL Store   │  │
//! │  │  (vectors)  │  │   (metadata, edges)  │  │
//! │  └─────────────┘  └──────────────────────┘  │
//! └─────────────────────────────────────────────┘
//! ```

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use uuid::Uuid;

use crate::memory::fractal_node::FractalNode;
use crate::memory::types::{ConflictState, ContextTier, MemorySource, MemoryStatus, Sensitivity};
use crate::memory::MemoryType;
use crate::memory::conversation::TurnRow;
use crate::storage::backend::{HybridQuery, ScoredNode, StorageBackend, UpdateOperation};

/// Hex-encoded BLAKE3 of the plaintext API key — stored in `auth_api_keys.key_hash` for O(1) lookup.
#[must_use]
pub fn stored_api_key_fingerprint(plaintext: &str) -> String {
    blake3::hash(plaintext.as_bytes()).to_hex().to_string()
}

/// PostgreSQL-backed storage layer.
/// Wraps the existing in-memory USearch index with persistent PostgreSQL storage.
pub struct PostgresStore {
    pub pool: PgPool,
}

/// SQLx row type for turn-level vector-similarity queries.
/// Returned by `search_turns()`. Maps raw DB columns to a lightweight struct.
#[derive(Debug, sqlx::FromRow)]
pub struct TurnWithScore {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    pub external_session_id: Option<String>,
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub similarity: f32,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub embedding_type: Option<String>,
    pub embedding_dim: Option<i32>,
}

/// Internal row type for turn retrieval that includes the full embedding vector.
/// Used by ranking/scoring pipeline when turn-level embedding access is needed.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct TurnWithVector {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    pub external_session_id: Option<String>,
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub similarity: f32,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub embedding_type: Option<String>,
    pub embedding_dim: Option<i32>,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, sqlx::FromRow)]
struct SessionTurnRow {
    pub turn_id: Uuid,
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub content_preview: String,
    pub token_count: Option<i32>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub embedding_type: Option<String>,
    pub embedding_dim: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct AdjacentTurnRow {
    pub turn_id: Uuid,
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub embedding_type: Option<String>,
    pub embedding_dim: Option<i32>,
}

fn allow_internal_meta(filter: Option<MemoryType>) -> bool {
    filter == Some(MemoryType::Meta)
}

fn is_internal_meta_artifact(node: &FractalNode) -> bool {
    if node.memory_type != MemoryType::Meta {
        return false;
    }
    let derivation = node
        .metadata
        .get(FractalNode::DERIVATION_KEY)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(derivation.as_str(), "instruction" | "reflected")
        || node
            .metadata
            .get(FractalNode::RETRIEVAL_VISIBILITY_KEY)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| v.eq_ignore_ascii_case(FractalNode::INTERNAL_VISIBILITY))
}

impl PostgresStore {
    /// Connect to PostgreSQL.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Expose pool for internal integration tests and maintenance workers.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Returns dominant embedding dimension in active memories, if any.
    pub async fn active_embedding_dimension(&self) -> anyhow::Result<Option<usize>> {
        let dim = sqlx::query_scalar::<_, Option<i32>>(
            r#"
            SELECT vector_dims(embedding)
            FROM memories
            WHERE status = 'active'
              AND embedding IS NOT NULL
            LIMIT 1
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(dim.map(|d| d as usize))
    }

    fn align_query_vector_dim(mut vector: Vec<f32>, target_dim: usize) -> Vec<f32> {
        if vector.len() > target_dim {
            vector.truncate(target_dim);
            return vector;
        }
        if vector.len() < target_dim {
            vector.resize(target_dim, 0.0);
        }
        vector
    }

    /// Max nodes added by [`StorageBackend::expand_fractal`] beyond the initial hit list.
    pub const PG_EXPAND_FRACTAL_MAX_EXTRA: usize = 100;

    /// Batch-load fractal nodes (active only). One round-trip for UUID fan-out.
    pub async fn get_fractal_nodes_any(
        &self,
        ids: &[Uuid],
    ) -> anyhow::Result<HashMap<Uuid, FractalNode>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<MemoryRow> = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT 
                id , memory_type ,
                content , content_preview,
                importance , confidence ,
                sensitivity , status ,
                superseded_by, conflict_state ,
                source , source_id, provenance ,
                parent_id, depth , access_count ,
                last_accessed, created_at , updated_at ,
                deleted_at, metadata , entities ,
                COALESCE(tags, ARRAY[]::TEXT[]) AS tags ,
                embedding::float4[] ,
                context_tier::text, parent_tier_id,
                COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                summary_content, overview_content
            FROM memories
            WHERE id = ANY($1) AND status != 'deleted'
            "#,
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let id = r.id;
                (id, memory_row_to_fractal_node(r))
            })
            .collect())
    }

    /// Iterative fractal child expansion (matches `MemoryStore::expand_children` semantics).
    async fn expand_children_pg(
        &self,
        child_ids: &[Uuid],
        query_vector: &[f32],
        depth: usize,
        threshold: f32,
        visited: &mut HashSet<Uuid>,
        expanded: &mut Vec<ScoredNode>,
        max_total: usize,
    ) -> anyhow::Result<()> {
        let mut stack: Vec<(Vec<Uuid>, usize)> = vec![(child_ids.to_vec(), depth)];

        while let Some((ids, d)) = stack.pop() {
            if d == 0 || expanded.len() >= max_total {
                continue;
            }

            let fetch_ids: Vec<Uuid> = ids
                .into_iter()
                .filter(|id| !visited.contains(id))
                .collect();
            if fetch_ids.is_empty() {
                continue;
            }

            let loaded = self.get_fractal_nodes_any(&fetch_ids).await?;

            for id in fetch_ids {
                if expanded.len() >= max_total {
                    break;
                }
                if visited.contains(&id) {
                    continue;
                }
                visited.insert(id);

                let Some(child) = loaded.get(&id) else {
                    tracing::debug!(node_id = %id, "expand_fractal: child id missing or deleted");
                    continue;
                };

                let sim =
                    crate::memory::fractal_node::cosine_similarity(&child.vector, query_vector);

                expanded.push(ScoredNode {
                    id: child.id,
                    score: sim,
                    distribution_scores: None,
                    debug: None,
                    node: child.clone(),
                });

                if sim >= threshold && !child.children_tier_ids.is_empty() {
                    stack.push((child.children_tier_ids.clone(), d.saturating_sub(1)));
                }
            }
        }

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Event Log (Layer 0 — immutable)
    // -------------------------------------------------------------------------

    /// Append an event to the immutable event log.
    pub async fn append_event(
        &self,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO events (id, event_type, payload, created_at)
            VALUES ($1, $2, $3, NOW())
            "#,
        )
        .bind(id)
        .bind(event_type)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Read events for replay (used for rebuilding state from event log).
    pub async fn read_events(&self, after_id: Option<Uuid>, limit: i64) -> Result<Vec<Event>> {
        let rows = if let Some(after) = after_id {
            sqlx::query_as::<_, Event>(
                r#"
                SELECT id, event_type, payload, created_at
                FROM events
                WHERE id > $1
                ORDER BY created_at ASC
                LIMIT $2::bigint
                "#,
            )
            .bind(after)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Event>(
                r#"
                SELECT id, event_type, payload, created_at
                FROM events
                ORDER BY created_at ASC
                LIMIT $1::bigint
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Memory CRUD
    // -------------------------------------------------------------------------

    /// Store a session memory node.
    ///
    /// DEPRECATED: This method's name is misleading — it stores individual
    /// memory nodes (`FractalNode`) in the `memories` table, **not** session-level
    /// aggregate embeddings. Session-level embeddings on `conversation_sessions`
    /// were removed in migration 015. All nodes inserted here carry their own
    /// per-node embedding vector.
    ///
    /// This is the canonical write path for all memory nodes in Postgres mode
    /// (used by the [`StorageBackend::insert`] implementation), regardless of
    /// whether they originated from a session, import, consolidation, or external source.
    #[allow(deprecated)]
    pub async fn store_session(
        &self,
        content: String,
        embedding: Vec<f32>,
        memory_type: MemoryType,
        provenance: serde_json::Value,
        source: &str,
        source_id: Option<&str>,
        entities: Vec<String>,
        tags: Vec<String>,
        importance: i32,
        confidence: f64,
        sensitivity: &str,
        metadata: serde_json::Value,
        context_tier: &str,
        parent_tier_id: Option<Uuid>,
        children_tier_ids: Vec<Uuid>,
        summary_content: Option<&str>,
        overview_content: Option<&str>,
        created_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let memory_type_str = match memory_type {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Preference => "preference",
            MemoryType::Procedural => "procedural",
            MemoryType::Meta => "meta",
            MemoryType::Decision => "decision",
        };

        sqlx::query(r#"
            INSERT INTO memories (
                id, memory_type, content, embedding, entities, tags,
                provenance, source, source_id, importance, confidence,
                sensitivity, metadata, status, access_count, created_at, updated_at,
                context_tier, parent_tier_id, children_tier_ids,
                summary_content, overview_content
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'active', 0, COALESCE($19, NOW()), NOW(), $14::context_tier, $15, $16, $17, $18)
            "#)
        .bind(id)
        .bind(memory_type_str)
        .bind(content)
        .bind(embedding)
        .bind(serde_json::to_value(&entities).unwrap_or(serde_json::json!([])))
        .bind(&tags)
        .bind(provenance)
        .bind(source)
        .bind(source_id)
        .bind(importance)
        .bind(confidence)
        .bind(sensitivity)
        .bind(metadata)
        .bind(context_tier)
        .bind(parent_tier_id)
        .bind(&children_tier_ids)
        .bind(summary_content)
        .bind(overview_content)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        // Append event to Layer 0
        self.append_event(
            "session_stored",
            &serde_json::json!({
                "memory_id": id.to_string(),
                "memory_type": memory_type_str,
                "source": source,
            }),
        )
        .await?;

        Ok(id)
    }

    /// Retrieve a single memory by ID.
    pub async fn get_memory(&self, id: Uuid) -> Result<Option<MemoryRow>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT 
                id , memory_type ,
                content , content_preview,
                importance , confidence ,
                sensitivity , status ,
                superseded_by, conflict_state ,
                source , source_id, provenance ,
                parent_id, depth , access_count ,
                last_accessed, created_at , updated_at ,
                deleted_at, metadata , entities ,
                COALESCE(tags, ARRAY[]::TEXT[]) AS tags ,
                embedding::float4[] ,
                context_tier::text, parent_tier_id,
                COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                summary_content, overview_content
            FROM memories
            WHERE id = $1 AND status != 'deleted'
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Update access statistics.
    pub async fn record_access(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memories
            SET access_count = access_count + 1, last_accessed = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark a memory as superseded by another.
    pub async fn supersede(&self, memory_id: Uuid, superseded_by_id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memories
            SET status = 'superseded', superseded_by = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(0)
        .bind(superseded_by_id)
        .execute(&self.pool)
        .await?;

        self.append_event(
            "memory_superseded",
            &serde_json::json!({
                "memory_id": memory_id.to_string(),
                "superseded_by": superseded_by_id.to_string(),
            }),
        )
        .await?;

        Ok(())
    }

    /// Update memory status.
    pub async fn update_status(&self, id: Uuid, status: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memories
            SET status = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(0)
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Soft delete a memory.
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memories
            SET status = 'deleted', deleted_at = NOW(), updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        self.append_event(
            "memory_deleted",
            &serde_json::json!({ "memory_id": id.to_string() }),
        )
        .await?;

        Ok(())
    }

    /// Update a memory's embedding vector.
    pub async fn update_vector(&self, id: Uuid, new_embedding: Vec<f32>) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE memories
            SET embedding = $2, updated_at = NOW()
            WHERE id = $1 AND status != 'deleted'
            "#,
        )
        .bind(id)
        .bind(new_embedding)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Full-text search using PostgreSQL ts_rank (BM25-like approximation).
    /// Returns (memory_id, rank_score) pairs ordered by relevance.
    pub async fn search_bm25(&self, query_text: &str, top_k: i32) -> Result<Vec<(Uuid, f32)>> {
        if query_text.trim().is_empty() {
            return Ok(vec![]);
        }

        #[derive(Debug, sqlx::FromRow)]
        struct Bm25Row {
            id: Uuid,
            rank: f64,
        }

        let rows: Vec<Bm25Row> = sqlx::query_as::<_, Bm25Row>(r#"
            SELECT id, COALESCE(ts_rank(to_tsvector('english', content), plainto_tsquery('english', $1)), 0.0)::float8 AS rank
            FROM memories
            WHERE status = 'active'
              AND to_tsvector('english', content) @@ plainto_tsquery('english', $1)
            ORDER BY ts_rank(to_tsvector('english', content), plainto_tsquery('english', $1)) DESC
            LIMIT $2::bigint
            "#)
        .bind(query_text)
        .bind(top_k as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| (r.id, r.rank as f32)).collect())
    }

    /// Purge memories with null or all-zero embedding vectors.
    /// Returns the number of purged memories.
    pub async fn purge_dummy_vectors(&self) -> Result<usize> {
        let result = sqlx::query(
            r#"
            DELETE FROM memories
            WHERE status = 'active'
              AND (embedding IS NULL
                   OR embedding = '{}'::vector)
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as usize)
    }

    /// Vector similarity search.
    pub async fn vector_search(
        &self,
        embedding: &[f32],
        limit: i32,
        memory_type: Option<&str>,
        min_importance: Option<i32>,
    ) -> Result<Vec<MemoryWithScore>> {
        let embedding_dim = embedding.len() as i32;
        // Build dynamic query with optional filters
        let rows = if let Some(mt) = memory_type {
            if let Some(mi) = min_importance {
                let embedding_str = format!(
                    "[{}]",
                    embedding
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                sqlx::query_as::<_, MemoryWithScore>(
                    r#"
                    SELECT id, memory_type,
                           content, importance,
                           COALESCE(confidence, 0.0::double precision) as confidence,
                           sensitivity,
                           status, source,
                           access_count,
                           created_at, updated_at,
                           source_id, provenance, last_accessed,
                           content_preview,
                           COALESCE((1 - (embedding <=> $1::vector))::float, 0.0) AS similarity,
                           embedding::float4[] as embedding,
                           context_tier::text, parent_tier_id,
                           COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                           summary_content, overview_content
                    FROM (
                        SELECT *
                        FROM memories
                        WHERE status = 'active'
                          AND embedding IS NOT NULL
                          AND memory_type = $3
                          AND importance >= $4
                          AND vector_dims(embedding) = $5
                    ) filtered
                    ORDER BY embedding <=> $1::vector
                    LIMIT $2::bigint
                    "#,
                )
                .bind(&embedding_str)
                .bind(limit as i64)
                .bind(mt)
                .bind(mi)
                .bind(embedding_dim)
                .fetch_all(&self.pool)
                .await?
            } else {
                let embedding_str = format!(
                    "[{}]",
                    embedding
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                sqlx::query_as::<_, MemoryWithScore>(r#"
                    SELECT id, memory_type,
                           content, importance,
                           COALESCE(confidence, 0.0::double precision)::double precision AS confidence,
                           sensitivity,
                           status, source,
                           access_count,
                           created_at, updated_at,
                           source_id, provenance,
                           last_accessed,
                           content_preview,
                           COALESCE((1 - (embedding <=> $1::vector))::float, 0.0) AS similarity,
                           embedding::float4[] as embedding,
                           context_tier::text, parent_tier_id,
                           COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                           summary_content, overview_content
                    FROM (
                        SELECT *
                        FROM memories
                        WHERE status = 'active'
                          AND embedding IS NOT NULL
                          AND memory_type = $3
                          AND vector_dims(embedding) = $4
                    ) filtered
                    ORDER BY embedding <=> $1::vector
                    LIMIT $2::bigint
                    "#)
                .bind(&embedding_str)
                .bind(limit as i64)
                .bind(mt)
                .bind(embedding_dim)
                .fetch_all(&self.pool)
                .await?
            }
        } else {
            let embedding_str = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            sqlx::query_as::<_, MemoryWithScore>(
                r#"
                SELECT id, memory_type,
                       content, importance,
                       COALESCE(confidence, 0.0::double precision) as confidence,
                       sensitivity,
                       status, source,
                       access_count,
                       created_at, updated_at,
                       source_id, provenance,
                       last_accessed,
                       content_preview,
                       COALESCE((1 - (embedding <=> $1::vector))::float, 0.0) AS similarity,
                       embedding::float4[] as embedding,
                       context_tier::text, parent_tier_id,
                       COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                       summary_content, overview_content
                FROM (
                    SELECT *
                    FROM memories
                    WHERE status = 'active'
                      AND embedding IS NOT NULL
                      AND vector_dims(embedding) = $3
                ) filtered
                ORDER BY embedding <=> $1::vector
                LIMIT $2::bigint
                "#,
            )
            .bind(&embedding_str)
            .bind(limit as i64)
            .bind(embedding_dim)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    /// Recent memories.
    pub async fn recent_memories(&self, limit: i32) -> Result<Vec<MemoryRow>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT 
                id , memory_type ,
                content , content_preview,
                importance , confidence ,
                sensitivity , status ,
                superseded_by, conflict_state ,
                source , source_id, provenance ,
                parent_id, depth , access_count ,
                last_accessed, created_at , updated_at ,
                deleted_at, metadata , entities ,
                COALESCE(tags, ARRAY[]::TEXT[]) AS tags ,
                embedding::float4[] ,
                context_tier::text, parent_tier_id,
                COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                summary_content, overview_content
            FROM memories
            WHERE status = 'active'
            ORDER BY created_at DESC
            LIMIT $1::bigint
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List all active memories (for list_all).
    pub async fn list_memories(&self) -> Result<Vec<MemoryRow>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT 
                id , memory_type ,
                content , content_preview,
                importance , confidence ,
                sensitivity , status ,
                superseded_by, conflict_state ,
                source , source_id, provenance ,
                parent_id, depth , access_count ,
                last_accessed, created_at , updated_at ,
                deleted_at, metadata , entities ,
                COALESCE(tags, ARRAY[]::TEXT[]) AS tags ,
                embedding::float4[] ,
                context_tier::text, parent_tier_id,
                COALESCE(children_tier_ids, ARRAY[]::uuid[]) AS children_tier_ids,
                summary_content, overview_content
            FROM memories
            WHERE status = 'active'
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count total active memories.
    pub async fn count(&self, memory_type: Option<&str>) -> Result<i64> {
        let row = if let Some(mt) = memory_type {
            sqlx::query(
                r#"
                SELECT COUNT(*)::bigint as "total:i64"
                FROM memories
                WHERE status = 'active' AND memory_type = $1
                "#,
            )
            .bind(mt)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT COUNT(*)::bigint as "total:i64"
                FROM memories
                WHERE status = 'active'
                "#,
            )
            .fetch_one(&self.pool)
            .await?
        };
        let total: i64 = row.try_get(0)?;
        Ok(total)
    }

    // -------------------------------------------------------------------------
    // Knowledge Edges
    // -------------------------------------------------------------------------

    /// Create a knowledge edge.
    pub async fn create_edge(
        &self,
        from_node_id: Uuid,
        to_node_id: Uuid,
        edge_type: &str,
        strength: f64,
        confidence: f64,
        bidirectional: bool,
        reason: Option<&str>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(r#"
            INSERT INTO knowledge_edges
                (id, from_node_id, to_node_id, edge_type, strength, confidence, bidirectional, reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
            ON CONFLICT (from_node_id, to_node_id, edge_type) DO NOTHING
            "#)
        .bind(id)
        .bind(from_node_id)
        .bind(to_node_id)
        .bind(edge_type)
        .bind(strength)
        .bind(confidence)
        .bind(0)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get edges from a node.
    pub async fn get_edges(&self, from_node_id: Uuid) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            r#"
            SELECT id , from_node_id as "from_node_id!", to_node_id as "to_node_id!",
                   edge_type as "edge_type!", strength as "strength!",
                   confidence , causality as "causality!",
                   bidirectional as "bidirectional!",
                   reason, created_at , metadata
            FROM knowledge_edges
            WHERE from_node_id = $1 OR to_node_id = $1
            "#,
        )
        .bind(from_node_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Delete an edge.
    pub async fn delete_edge(&self, id: Uuid) -> Result<()> {
        sqlx::query!("DELETE FROM knowledge_edges WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Fractal Structure (parent-child for zoom retrieval)
    // -------------------------------------------------------------------------

    /// Set parent (for fractal zoom).
    pub async fn set_parent(&self, memory_id: Uuid, parent_id: Uuid) -> Result<()> {
        sqlx::query(r#"
            UPDATE memories
            SET parent_id = $2, depth = (SELECT depth + 1 FROM memories WHERE id = $2), updated_at = NOW()
            WHERE id = $1
            "#)
        .bind(0)
        .bind(parent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get fractal children (for zoom retrieval).
    /// Uses a properly structured CTE with column aliases defined once in the CTE header.
    pub async fn get_children(&self, memory_id: Uuid, max_depth: i32) -> Result<Vec<MemoryRow>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            WITH RECURSIVE fractal_tree(
                id, memory_type, content, importance, confidence, sensitivity,
                status, conflict_state, source, depth, access_count,
                created_at, updated_at, superseded_by, source_id, provenance,
                parent_id, last_accessed, deleted_at, metadata, entities,
                tags, content_preview, embedding, level
            ) AS (
                SELECT id, memory_type, content, importance, confidence, sensitivity,
                       status, conflict_state, source, depth, access_count,
                       created_at, updated_at, superseded_by, source_id, provenance,
                       parent_id, last_accessed, deleted_at, metadata, entities,
                       tags, content_preview, embedding, 1
                FROM memories
                WHERE parent_id = $1 AND status = 'active'

                UNION ALL

                SELECT m.id, m.memory_type, m.content, m.importance, m.confidence,
                       m.sensitivity, m.status, m.conflict_state, m.source, m.depth,
                       m.access_count, m.created_at, m.updated_at, m.superseded_by,
                       m.source_id, m.provenance, m.parent_id, m.last_accessed,
                       m.deleted_at, m.metadata, m.entities, m.tags,
                       m.content_preview, m.embedding, ft.level + 1
                FROM memories m
                INNER JOIN fractal_tree ft ON m.parent_id = ft.id
                WHERE m.status = 'active' AND ft.level < $2
            )
            SELECT id , memory_type ,
                   content , importance ,
                   confidence , sensitivity ,
                   status , conflict_state ,
                   source , depth ,
                   access_count ,
                   created_at , updated_at ,
                   superseded_by, source_id, provenance, parent_id,
                   last_accessed, deleted_at, metadata, entities,
                   COALESCE(tags, ARRAY[]::TEXT[]) AS tags ,
                   content_preview,
                   embedding::float4[] 
            FROM fractal_tree
            ORDER BY level
            "#,
        )
        .bind(0)
        .bind(max_depth)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Consolidation History
    // -------------------------------------------------------------------------

    /// Log a consolidation run.
    pub async fn log_consolidation(
        &self,
        session_id: Option<&str>,
        conversation_id: Option<&str>,
        memories_processed: i32,
        new_memories_created: i32,
        edges_created: i32,
        processing_time_ms: i32,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO consolidation_history (
                id, consolidation_date, session_id, conversation_id,
                memories_processed, new_memories_created, edges_created,
                processing_time_ms, status, error_message, created_at
            )
            VALUES ($1, CURRENT_DATE, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
            "#,
        )
        .bind(id)
        .bind(session_id)
        .bind(conversation_id)
        .bind(memories_processed)
        .bind(new_memories_created)
        .bind(edges_created)
        .bind(processing_time_ms)
        .bind(status)
        .bind(error_message)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get recent consolidation runs.
    pub async fn recent_consolidations(&self, limit: i32) -> Result<Vec<ConsolidationRow>> {
        let rows = sqlx::query_as::<_, ConsolidationRow>(
            r#"
            SELECT id , consolidation_date,
                   session_id, conversation_id,
                   memories_processed as "memories_processed!",
                   new_memories_created as "new_memories_created!",
                   edges_created as "edges_created!",
                   processing_time_ms as "processing_time_ms!",
                   status ,
                   error_message, created_at 
            FROM consolidation_history
            ORDER BY created_at DESC
            LIMIT $1::bigint
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Audit Log
    // -------------------------------------------------------------------------

    /// Log an audit finding.
    pub async fn log_audit(
        &self,
        run_id: Uuid,
        issue_type: &str,
        memory_id: Uuid,
        severity: &str,
        description: &str,
        action_taken: Option<&str>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(r#"
            INSERT INTO audit_log (id, run_id, issue_type, memory_id, severity, description, action_taken, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            "#)
        .bind(id)
        .bind(run_id)
        .bind(issue_type)
        .bind(memory_id)
        .bind(severity)
        .bind(0)
        .bind(action_taken)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    // -------------------------------------------------------------------------
    // Auth: Users (beta user accounts)
    // -------------------------------------------------------------------------

    /// Create a new user account.
    pub async fn create_user(
        &self,
        username: &str,
        email: &str,
        password_hash: &str,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO auth_users (id, username, email, password_hash, created_at)
            VALUES ($1, $2, $3, $4, NOW())
            "#,
        )
        .bind(id)
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get a user by username.
    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<AuthUserRow>> {
        let row = sqlx::query_as(
            r#"
            SELECT id, username, email, password_hash, created_at
            FROM auth_users
            WHERE username = $1
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Delete a beta user and cascading API keys (`ON DELETE CASCADE`).
    pub async fn delete_auth_user(&self, user_id: Uuid) -> Result<()> {
        sqlx::query(r#"DELETE FROM auth_users WHERE id = $1"#)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get a user by ID.
    pub async fn get_user_by_id(&self, id: Uuid) -> Result<Option<AuthUserRow>> {
        let row = sqlx::query_as(
            r#"
            SELECT id, username, email, password_hash, created_at
            FROM auth_users
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // -------------------------------------------------------------------------
    // Auth: API Keys
    // -------------------------------------------------------------------------

    /// Create an API key for a user. Returns the key ID.
    ///
    /// `key_hash` must be [`stored_api_key_fingerprint`] of the plaintext key (BLAKE3 hex).
    pub async fn create_api_key(&self, user_id: Uuid, key_hash: &str, name: &str) -> Result<Uuid> {
        self.create_api_key_with_expiry(user_id, key_hash, name, None)
            .await
    }

    /// Create an API key for a user with optional expiration.
    pub async fn create_api_key_with_expiry(
        &self,
        user_id: Uuid,
        key_hash: &str,
        name: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO auth_api_keys (id, user_id, key_hash, name, created_at, expires_at, revoked_at)
            VALUES ($1, $2, $3, $4, NOW(), $5, NULL)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(key_hash)
        .bind(name)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Look up an API key by stored fingerprint (`key_hash` column).
    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<AuthApiKeyRow>> {
        let row = sqlx::query_as(
            r#"
            SELECT id, user_id, key_hash, name, created_at, last_used_at, expires_at, revoked_at
            FROM auth_api_keys
            WHERE key_hash = $1
              AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Resolve an API key from the raw Bearer secret (BLAKE3 lookup, with legacy bcrypt migration).
    pub async fn find_api_key_by_plaintext(
        &self,
        plaintext: &str,
    ) -> Result<Option<AuthApiKeyRow>> {
        let digest = stored_api_key_fingerprint(plaintext);
        if let Some(row) = self.get_api_key_by_hash(&digest).await? {
            return Ok(Some(row));
        }

        let legacy = sqlx::query_as::<_, AuthApiKeyRow>(
            r#"
            SELECT id, user_id, key_hash, name, created_at, last_used_at, expires_at, revoked_at
            FROM auth_api_keys
            WHERE key_hash LIKE '$2%'
              AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        for row in legacy {
            if bcrypt::verify(plaintext, &row.key_hash).unwrap_or(false) {
                sqlx::query(r#"UPDATE auth_api_keys SET key_hash = $1 WHERE id = $2"#)
                    .bind(&digest)
                    .bind(row.id)
                    .execute(&self.pool)
                    .await?;
                tracing::info!(key_id = %row.id, "migrated API key row from bcrypt to BLAKE3 fingerprint");
                let mut upgraded = row;
                upgraded.key_hash = digest;
                return Ok(Some(upgraded));
            }
        }

        Ok(None)
    }

    /// Atomically remove one API key row and insert a new fingerprint for the same user/name.
    pub async fn replace_api_key(
        &self,
        old_key_id: Uuid,
        user_id: Uuid,
        key_name: &str,
        new_fingerprint: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let deleted = sqlx::query(r#"DELETE FROM auth_api_keys WHERE id = $1 AND user_id = $2"#)
            .bind(old_key_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        if deleted.rows_affected() != 1 {
            anyhow::bail!("api key row missing or user mismatch");
        }
        let new_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO auth_api_keys (id, user_id, key_hash, name, created_at, expires_at, revoked_at)
            VALUES ($1, $2, $3, $4, NOW(), $5, NULL)
            "#,
        )
        .bind(new_id)
        .bind(user_id)
        .bind(new_fingerprint)
        .bind(key_name)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Update last_used_at when an API key is used.
    pub async fn record_api_key_usage(&self, key_id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE auth_api_keys
            SET last_used_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(key_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Run auth schema migrations (create tables if they don't exist).
    /// Called on server startup when postgres-storage feature is enabled.
    pub async fn run_auth_migrations(&self) -> Result<()> {
        // Create auth_users table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS auth_users (
                id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                username        VARCHAR(255) UNIQUE NOT NULL,
                email           VARCHAR(255) UNIQUE NOT NULL,
                password_hash   VARCHAR(255) NOT NULL,
                created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Create indexes on auth_users
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_auth_users_username ON auth_users(username)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_auth_users_email ON auth_users(email)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Create auth_api_keys table (key_hash: BLAKE3 hex of plaintext key; legacy bcrypt rows start with $2)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS auth_api_keys (
                id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                user_id         UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
                key_hash        VARCHAR(255) NOT NULL,
                name            VARCHAR(255) DEFAULT 'default',
                created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                last_used_at    TIMESTAMPTZ,
                expires_at      TIMESTAMPTZ,
                revoked_at      TIMESTAMPTZ
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            ALTER TABLE auth_api_keys
            ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            ALTER TABLE auth_api_keys
            ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Create indexes on auth_api_keys
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_auth_api_keys_key_hash ON auth_api_keys(key_hash)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_auth_api_keys_user ON auth_api_keys(user_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        tracing::info!("auth schema migrations completed");
        Ok(())
    }

    // =========================================================================
    // Turn-Level Storage (per-turn embedding pipeline)
    // =========================================================================

    /// Find an existing conversation session by external_id, or create one.
    pub async fn find_or_create_session(&self, external_id: &str) -> Result<Uuid> {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM conversation_sessions WHERE external_id = $1",
        )
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO conversation_sessions (id, external_id, started_at) VALUES ($1, $2, NOW())",
        )
        .bind(id)
        .bind(external_id)
        .execute(&self.pool)
        .await?;
        tracing::info!(%id, external_id, "created conversation session");
        Ok(id)
    }

    /// Store a single conversational turn with its embedding.
    pub async fn store_turn(
        &self,
        external_session_id: &str,
        turn_index: i32,
        speaker_role: &str,
        content: &str,
        embedding: Vec<f32>,
        metadata: Option<serde_json::Value>,
        embedding_type: &str,
        embedding_dim: i32,
    ) -> Result<Uuid> {
        let session_id = self.find_or_create_session(external_session_id).await?;
        let turn_id = Uuid::new_v4();
        let meta = metadata.unwrap_or(serde_json::json!({}));
        sqlx::query(
            r#"INSERT INTO conversation_turns (id, session_id, turn_index, speaker_role, content, embedding, metadata, embedding_type, embedding_dim)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (session_id, turn_index) DO UPDATE SET
                content = EXCLUDED.content, embedding = EXCLUDED.embedding,
                speaker_role = EXCLUDED.speaker_role, metadata = EXCLUDED.metadata,
                embedding_type = EXCLUDED.embedding_type, embedding_dim = EXCLUDED.embedding_dim"#,
        )
        .bind(turn_id).bind(session_id).bind(turn_index).bind(speaker_role)
        .bind(content).bind(&embedding).bind(&meta)
        .bind(embedding_type).bind(embedding_dim)
        .execute(&self.pool).await?;
        sqlx::query(
            "UPDATE conversation_sessions SET turn_count = (SELECT COUNT(*) FROM conversation_turns WHERE session_id = $1), updated_at = NOW() WHERE id = $1",
        )
        .bind(session_id).execute(&self.pool).await?;
        tracing::info!(%turn_id, %session_id, turn_index, speaker_role, "turn stored");
        Ok(turn_id)
    }

    /// Store multiple turns in a single batch.
    pub async fn store_turns_batch(
        &self,
        external_session_id: &str,
        turns: &[crate::api::turns::BatchTurnItem],
        embeddings: Vec<Vec<f32>>,
        embedding_type: &str,
        embedding_dim: i32,
    ) -> Result<(Uuid, Vec<Uuid>)> {
        let session_id = self.find_or_create_session(external_session_id).await?;
        let mut turn_ids = Vec::with_capacity(turns.len());
        for (item, emb) in turns.iter().zip(embeddings.iter()) {
            let turn_id = Uuid::new_v4();
            let meta = item.metadata.clone().unwrap_or(serde_json::json!({}));
            sqlx::query(
                r#"INSERT INTO conversation_turns (id, session_id, turn_index, speaker_role, content, embedding, metadata, embedding_type, embedding_dim)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (session_id, turn_index) DO UPDATE SET
                    content = EXCLUDED.content, embedding = EXCLUDED.embedding,
                    speaker_role = EXCLUDED.speaker_role, metadata = EXCLUDED.metadata,
                    embedding_type = EXCLUDED.embedding_type, embedding_dim = EXCLUDED.embedding_dim"#,
            )
            .bind(turn_id).bind(session_id).bind(item.turn_index).bind(&item.speaker_role)
            .bind(&item.content).bind(emb).bind(&meta)
            .bind(embedding_type).bind(embedding_dim)
            .execute(&self.pool).await?;
            turn_ids.push(turn_id);
        }
        sqlx::query(
            "UPDATE conversation_sessions SET turn_count = (SELECT COUNT(*) FROM conversation_turns WHERE session_id = $1), updated_at = NOW() WHERE id = $1",
        )
        .bind(session_id).execute(&self.pool).await?;
        tracing::info!(%session_id, count = turn_ids.len(), "turns batch stored");
        Ok((session_id, turn_ids))
    }

    /// Retrieve turns by vector similarity search, returning API-ready ScoredTurn records.
    pub async fn retrieve_turns(
        &self,
        query_vector: &[f32],
        top_k: usize,
        speaker_filter: Option<&str>,
        session_id_filter: Option<Uuid>,
    ) -> Result<Vec<crate::api::turns::ScoredTurn>> {
        if query_vector.is_empty() {
            return Ok(vec![]);
        }
        let k = top_k as i64;
        let rows = if let Some(sid) = session_id_filter {
            if let Some(speaker) = speaker_filter {
                sqlx::query_as::<_, TurnWithScore>(
                    r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                    ct.embedding_type, ct.embedding_dim,
                    (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                    FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                    WHERE ct.session_id = $2 AND ct.speaker_role = $3 AND ct.embedding IS NOT NULL
                    ORDER BY ct.embedding <=> $1 LIMIT $4"#,
                ).bind(query_vector).bind(sid).bind(speaker).bind(k).fetch_all(&self.pool).await?
            } else {
                sqlx::query_as::<_, TurnWithScore>(
                    r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                    ct.embedding_type, ct.embedding_dim,
                    (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                    FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                    WHERE ct.session_id = $2 AND ct.embedding IS NOT NULL
                    ORDER BY ct.embedding <=> $1 LIMIT $3"#,
                ).bind(query_vector).bind(sid).bind(k).fetch_all(&self.pool).await?
            }
        } else if let Some(speaker) = speaker_filter {
            sqlx::query_as::<_, TurnWithScore>(
                r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                ct.embedding_type, ct.embedding_dim,
                (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                WHERE ct.speaker_role = $2 AND ct.embedding IS NOT NULL
                ORDER BY ct.embedding <=> $1 LIMIT $3"#,
            ).bind(query_vector).bind(speaker).bind(k).fetch_all(&self.pool).await?
        } else {
            sqlx::query_as::<_, TurnWithScore>(
                r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                ct.embedding_type, ct.embedding_dim,
                (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                WHERE ct.embedding IS NOT NULL ORDER BY ct.embedding <=> $1 LIMIT $2"#,
            ).bind(query_vector).bind(k).fetch_all(&self.pool).await?
        };
        Ok(rows.into_iter().map(|r| {
            let embedding_info = r.embedding_type.map(|provider| crate::memory::conversation::EmbeddingInfo {
                vector: vec![], // vector not included in scored turn responses (too large)
                provider,
                dimension: r.embedding_dim.unwrap_or(0) as usize,
                metadata: None,
            });
            crate::api::turns::ScoredTurn {
                turn_id: r.turn_id, session_id: r.session_id,
                external_session_id: r.external_session_id, turn_index: r.turn_index,
                speaker_role: r.speaker_role, content: r.content, similarity: r.similarity,
                metadata: r.metadata, created_at: r.created_at, embedding_info, adjacent_turns: None,
            }
        }).collect())
    }

    /// Retrieve turns by vector similarity, returning internal results with full embedding vectors.
    /// Used by the ranking/scoring pipeline (`retrieve_fractal` augmentation) so turn nodes
    /// carry their vectors for downstream processing (fractal zoom, reranker, session scoring).
    pub async fn retrieve_turns_internal(
        &self,
        query_vector: &[f32],
        top_k: usize,
        speaker_filter: Option<&str>,
        session_id_filter: Option<Uuid>,
    ) -> Result<Vec<TurnWithVector>> {
        if query_vector.is_empty() {
            return Ok(vec![]);
        }
        let k = top_k as i64;
        if let Some(sid) = session_id_filter {
            if let Some(speaker) = speaker_filter {
                sqlx::query_as::<_, TurnWithVector>(
                    r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                    ct.embedding_type, ct.embedding_dim, ct.embedding,
                    (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                    FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                    WHERE ct.session_id = $2 AND ct.speaker_role = $3 AND ct.embedding IS NOT NULL
                    ORDER BY ct.embedding <=> $1 LIMIT $4"#,
                ).bind(query_vector).bind(sid).bind(speaker).bind(k).fetch_all(&self.pool).await
            } else {
                sqlx::query_as::<_, TurnWithVector>(
                    r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                    ct.embedding_type, ct.embedding_dim, ct.embedding,
                    (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                    FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                    WHERE ct.session_id = $2 AND ct.embedding IS NOT NULL
                    ORDER BY ct.embedding <=> $1 LIMIT $3"#,
                ).bind(query_vector).bind(sid).bind(k).fetch_all(&self.pool).await
            }
        } else if let Some(speaker) = speaker_filter {
            sqlx::query_as::<_, TurnWithVector>(
                r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                ct.embedding_type, ct.embedding_dim, ct.embedding,
                (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                WHERE ct.speaker_role = $2 AND ct.embedding IS NOT NULL
                ORDER BY ct.embedding <=> $1 LIMIT $3"#,
            ).bind(query_vector).bind(speaker).bind(k).fetch_all(&self.pool).await
        } else {
            sqlx::query_as::<_, TurnWithVector>(
                r#"SELECT ct.id AS turn_id, ct.session_id, ct.turn_index, ct.speaker_role, ct.content, ct.metadata, ct.created_at,
                ct.embedding_type, ct.embedding_dim, ct.embedding,
                (1 - (ct.embedding <=> $1))::FLOAT4 AS similarity, cs.external_id AS external_session_id
                FROM conversation_turns ct JOIN conversation_sessions cs ON ct.session_id = cs.id
                WHERE ct.embedding IS NOT NULL ORDER BY ct.embedding <=> $1 LIMIT $2"#,
            ).bind(query_vector).bind(k).fetch_all(&self.pool).await
        }
        .map_err(|e| anyhow::anyhow!("retrieve_turns_internal: {e}"))
    }

    /// Get all turns for a session, ordered by turn_index.
    pub async fn get_session_turns(&self, session_id: Uuid) -> Result<crate::api::turns::SessionTurnsResponse> {
        let session_row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT external_id FROM conversation_sessions WHERE id = $1",
        ).bind(session_id).fetch_optional(&self.pool).await?;
        let external_session_id = session_row.and_then(|r| r.0);
        let turns: Vec<crate::api::turns::SessionTurn> = sqlx::query_as::<_, SessionTurnRow>(
            r#"SELECT id AS turn_id, turn_index, speaker_role, content, LEFT(content, 500) AS content_preview,
            token_count, metadata, created_at, embedding_type, embedding_dim
            FROM conversation_turns WHERE session_id = $1 ORDER BY turn_index"#,
        ).bind(session_id).fetch_all(&self.pool).await?.into_iter().map(|r| {
            let embedding_info = r.embedding_type.map(|provider| crate::memory::conversation::EmbeddingInfo {
                vector: vec![],
                provider,
                dimension: r.embedding_dim.unwrap_or(0) as usize,
                metadata: None,
            });
            crate::api::turns::SessionTurn {
                turn_id: r.turn_id, turn_index: r.turn_index, speaker_role: r.speaker_role,
                content: r.content, content_preview: r.content_preview,
                token_count: r.token_count, metadata: r.metadata, created_at: r.created_at,
                embedding_info,
            }
        }).collect();
        Ok(crate::api::turns::SessionTurnsResponse { session_id, external_session_id, turns })
    }

    /// List turns for a session with pagination and ordering.
    /// Returns (turns, total_count) — total_count is the number of turns in the session.
    pub async fn list_turns_by_session(
        &self,
        session_id: Uuid,
        offset: i64,
        limit: i64,
        order_desc: bool,
    ) -> Result<(Vec<crate::api::turns::SessionTurn>, i64)> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_turns WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;

        let order_clause = if order_desc { "DESC" } else { "ASC" };
        let sql = format!(
            r#"SELECT id AS turn_id, turn_index, speaker_role, content, LEFT(content, 500) AS content_preview,
            token_count, metadata, created_at, embedding_type, embedding_dim
            FROM conversation_turns WHERE session_id = $1
            ORDER BY turn_index {} LIMIT $2 OFFSET $3"#,
            order_clause,
        );
        let turns: Vec<crate::api::turns::SessionTurn> = sqlx::query_as::<_, SessionTurnRow>(&sql)
            .bind(session_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| {
                let embedding_info = r.embedding_type.map(|provider| crate::memory::conversation::EmbeddingInfo {
                    vector: vec![],
                    provider,
                    dimension: r.embedding_dim.unwrap_or(0) as usize,
                    metadata: None,
                });
                crate::api::turns::SessionTurn {
                    turn_id: r.turn_id,
                    turn_index: r.turn_index,
                    speaker_role: r.speaker_role,
                    content: r.content,
                    content_preview: r.content_preview,
                    token_count: r.token_count,
                    metadata: r.metadata,
                    created_at: r.created_at,
                    embedding_info,
                }
            })
            .collect();
        Ok((turns, total))
    }

    /// Get adjacent turns for context expansion.
    pub async fn get_adjacent_turns(
        &self, session_id: Uuid, turn_index: i32, window: i32,
    ) -> Result<Vec<crate::api::turns::TurnContext>> {
        let rows: Vec<AdjacentTurnRow> = sqlx::query_as::<_, AdjacentTurnRow>(
            r#"SELECT id AS turn_id, turn_index, speaker_role, content, metadata, embedding_type, embedding_dim FROM conversation_turns
            WHERE session_id = $1 AND turn_index BETWEEN ($2 - $3) AND ($2 + $3) AND turn_index != $2 ORDER BY turn_index"#,
        ).bind(session_id).bind(turn_index).bind(window).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|r| crate::api::turns::TurnContext {
            turn_id: r.turn_id, turn_index: r.turn_index, speaker_role: r.speaker_role,
            content: r.content, metadata: r.metadata,
        }).collect())
    }

    // -------------------------------------------------------------------------
    // Turn CRUD (read, update, delete) — complementing store_turn / retrieve
    // -------------------------------------------------------------------------

    /// Read a single turn by its UUID.
    pub async fn get_turn(&self, turn_id: Uuid) -> Result<Option<TurnRow>> {
        let row = sqlx::query_as::<_, TurnRow>(
            r#"SELECT id, session_id, turn_index, speaker_role, content,
                      content_preview, embedding::float4[], embedding_type, embedding_dim,
                      token_count, metadata, created_at
               FROM conversation_turns
               WHERE id = $1"#,
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Update an existing turn's content, metadata, embedding, and/or speaker_role.
    /// When `embedding` is provided, `embedding_type` and `embedding_dim` should
    /// also be provided to keep embedding metadata consistent with the vector.
    /// Returns true if a row was updated.
    pub async fn update_turn(
        &self,
        turn_id: Uuid,
        content: Option<&str>,
        metadata: Option<serde_json::Value>,
        embedding: Option<Vec<f32>>,
        embedding_type: Option<&str>,
        embedding_dim: Option<i32>,
        speaker_role: Option<&str>,
    ) -> Result<bool> {
        // Build dynamic SET clause — only update fields that are Some
        let mut set_parts: Vec<String> = Vec::new();
        let mut idx: usize = 1;

        if content.is_some() {
            set_parts.push(format!("content = ${idx}", idx = idx));
            idx += 1;
        }
        if metadata.is_some() {
            set_parts.push(format!("metadata = ${idx}", idx = idx));
            idx += 1;
        }
        if embedding.is_some() {
            set_parts.push(format!("embedding = ${idx}", idx = idx));
            idx += 1;
        }
        if embedding_type.is_some() {
            set_parts.push(format!("embedding_type = ${idx}", idx = idx));
            idx += 1;
        }
        if embedding_dim.is_some() {
            set_parts.push(format!("embedding_dim = ${idx}", idx = idx));
            idx += 1;
        }
        if speaker_role.is_some() {
            set_parts.push(format!("speaker_role = ${idx}", idx = idx));
            idx += 1;
        }

        if set_parts.is_empty() {
            return Ok(false);
        }

        let sql = format!(
            "UPDATE conversation_turns SET {} WHERE id = ${idx}",
            set_parts.join(", "),
        );

        let mut query = sqlx::query(&sql);
        if let Some(c) = content {
            query = query.bind(c);
        }
        if let Some(m) = metadata {
            query = query.bind(m);
        }
        if let Some(e) = embedding.as_ref() {
            query = query.bind(e);
        }
        if let Some(t) = embedding_type {
            query = query.bind(t);
        }
        if let Some(d) = embedding_dim {
            query = query.bind(d);
        }
        if let Some(s) = speaker_role {
            query = query.bind(s);
        }
        query = query.bind(turn_id);

        let result = query.execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete a turn by its UUID (hard delete).
    /// Returns true if a row was deleted.
    /// Side effects: denormalized turn_count is refreshed.
    /// Linked memories (via turn_id FK) have their turn_id set to NULL (ON DELETE SET NULL).
    pub async fn delete_turn(&self, turn_id: Uuid) -> Result<bool> {
        // Capture session_id before deletion for side-effect maintenance
        let session_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT session_id FROM conversation_turns WHERE id = $1",
        )
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await?;

        let result = sqlx::query("DELETE FROM conversation_turns WHERE id = $1")
            .bind(turn_id)
            .execute(&self.pool)
            .await?;

        let deleted = result.rows_affected() > 0;
        if deleted {
            if let Some(sid) = session_id {
                // Refresh denormalized turn_count
                sqlx::query(
                    "UPDATE conversation_sessions SET turn_count = (SELECT COUNT(*) FROM conversation_turns WHERE session_id = $1), updated_at = NOW() WHERE id = $1",
                )
                .bind(sid)
                .execute(&self.pool)
                .await?;
            }
            tracing::info!(%turn_id, "turn deleted");
        }
        Ok(deleted)
    }

}

// =============================================================================
// StorageBackend implementation
// =============================================================================

#[async_trait]
impl StorageBackend for PostgresStore {
    // --- CRUD ---

    async fn insert(&self, node: FractalNode) -> anyhow::Result<Uuid> {
        let content = node.content.clone().unwrap_or_default();
        let embedding = node.vector.clone();
        let memory_type = node.memory_type;
        let provenance = node.provenance.clone();
        let source = match node.source {
            MemorySource::Conversation => "conversation",
            MemorySource::Document => "document",
            MemorySource::Import => "import",
            MemorySource::Manual => "manual",
            MemorySource::Consolidation => "consolidation",
            MemorySource::AiSelfImprovement => "ai_self_improvement",
        };
        let source_id = None;
        let entities: Vec<String> = vec![];
        let tags = vec![];
        let importance = node.importance;
        let confidence = node.confidence;
        let sensitivity = match node.sensitivity {
            Sensitivity::Normal => "normal",
            Sensitivity::Low => "low",
            Sensitivity::High => "high",
            Sensitivity::Restricted => "restricted",
        };
        let metadata = serde_json::to_value(&node.metadata).unwrap_or(serde_json::json!({}));

        let context_tier = node.context_tier.label();
        let parent_tier_id = node.parent_tier_id;
        let children_tier_ids = node.children_tier_ids.clone();
        let summary_content = node.summary_content.as_deref();
        let overview_content = node.overview_content.as_deref();
        let created_at = node.created_at;

        self.store_session(
            content,
            embedding,
            memory_type,
            provenance,
            source,
            source_id,
            entities,
            tags,
            importance,
            confidence,
            sensitivity,
            metadata,
            context_tier,
            parent_tier_id,
            children_tier_ids,
            summary_content,
            overview_content,
            Some(created_at),
        )
        .await
    }

    async fn get(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>> {
        let row = self.get_memory(*id).await?;
        Ok(row.map(|r| memory_row_to_fractal_node(r)))
    }

    async fn delete(&self, id: &Uuid) -> anyhow::Result<bool> {
        self.delete(*id).await?;
        Ok(true)
    }

    async fn update_vector(&self, id: &Uuid, new_vector: Vec<f32>) -> anyhow::Result<bool> {
        self.update_vector(*id, new_vector).await
    }

    // --- Query ---

    async fn hybrid_retrieve(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
        let include_internal_meta = allow_internal_meta(query.memory_type_filter);
        let mut owned_vector = query.query_vector.clone().unwrap_or_default();
        if !owned_vector.is_empty() {
            if let Some(db_dim) = self.active_embedding_dimension().await? {
                if db_dim != owned_vector.len() {
                    tracing::warn!(
                        query_dim = owned_vector.len(),
                        db_dim,
                        "aligning query vector dimension to active postgres embedding dimension"
                    );
                    owned_vector = Self::align_query_vector_dim(owned_vector, db_dim);
                }
            }
        }
        let vector = owned_vector.as_slice();
        let fetch_k = query.profile.fetch_k(query.top_k);

        // If only text is provided, fall back to BM25
        if query.query_text.is_some() && query.query_vector.is_none() {
            let text = query.query_text.as_ref().unwrap();
            let bm25_results = self.search_bm25(text, fetch_k as i32).await?;
            // Convert BM25 results to ScoredNodes by fetching full nodes
            let mut scored_nodes = Vec::new();
            for (id, score) in bm25_results {
                if let Some(node) = self.get(&id).await? {
                    let type_ok = query
                        .memory_type_filter
                        .map_or(true, |mt| node.memory_type == mt);
                    let internal_ok = include_internal_meta || !is_internal_meta_artifact(&node);
                    if query.profile.allows(&node) && type_ok && internal_ok {
                        scored_nodes.push(query.profile.score_node(score, node, query.source_type_weights));
                    }
                }
            }
            scored_nodes.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored_nodes.truncate(query.top_k);
            return Ok(scored_nodes);
        }

        // Vector search (with optional BM25 boost)
        let rows = match self.vector_search(vector, fetch_k as i32, None, None).await {
            Ok(rows) => rows,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("different vector dimensions") {
                    tracing::warn!(
                        query_dim = vector.len(),
                        "vector dimension mismatch in Postgres, falling back to BM25-only retrieval"
                    );
                    if let Some(text) = query.query_text.as_ref() {
                        let bm25_results = self.search_bm25(text, fetch_k as i32).await?;
                        let mut scored_nodes = Vec::new();
                        for (id, score) in bm25_results {
                            if let Some(node) = self.get(&id).await? {
                                let type_ok = query
                                    .memory_type_filter
                                    .map_or(true, |mt| node.memory_type == mt);
                                let internal_ok =
                                    include_internal_meta || !is_internal_meta_artifact(&node);
                                if query.profile.allows(&node) && type_ok && internal_ok {
                                    scored_nodes.push(query.profile.score_node(score, node, query.source_type_weights));
                                }
                            }
                        }
                        scored_nodes.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        scored_nodes.truncate(query.top_k);
                        return Ok(scored_nodes);
                    }
                    return Ok(vec![]);
                }
                return Err(e);
            }
        };

        // If no text query, return pure vector results
        if query.query_text.is_none() {
            let mut scored_nodes: Vec<ScoredNode> = Vec::new();
            for row in rows {
                if let Some(node) = self.get(&row.id).await? {
                    let type_ok = query
                        .memory_type_filter
                        .map_or(true, |mt| node.memory_type == mt);
                    let internal_ok = include_internal_meta || !is_internal_meta_artifact(&node);
                    if query.profile.allows(&node) && type_ok && internal_ok {
                        let row_vector = row.embedding.clone().unwrap_or_default();
                        let sim =
                            crate::memory::fractal_node::cosine_similarity(&row_vector, vector);
                        scored_nodes.push(query.profile.score_node(sim, node, query.source_type_weights));
                    }
                }
            }
            scored_nodes.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored_nodes.truncate(query.top_k);
            // Temporal recency boost
            if let Some(boost) = query.recency_boost {
                let _boosted = apply_temporal_boost_scored(&mut scored_nodes, boost);
            }
            return Ok(scored_nodes);
        }

        // Hybrid: combine vector + BM25 via RRF
        let bm25_text = query.query_text.as_ref().unwrap();
        let bm25_results = self.search_bm25(bm25_text, fetch_k as i32).await?;
        let bm25_ids: Vec<(Uuid, f32)> = bm25_results;

        let vector_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();

        let fused = rrf_fuse(&vector_ids, &bm25_ids, 60.0);

        let mut scored_nodes = Vec::new();
        for (id, score) in fused {
            if let Some(node) = self.get(&id).await? {
                let type_ok = query
                    .memory_type_filter
                    .map_or(true, |mt| node.memory_type == mt);
                let internal_ok = include_internal_meta || !is_internal_meta_artifact(&node);
                if query.profile.allows(&node) && type_ok && internal_ok {
                    scored_nodes.push(query.profile.score_node(score, node, query.source_type_weights));
                }
            }
        }
        scored_nodes.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored_nodes.truncate(query.top_k);

        // Temporal recency boost (legacy)
        if let Some(boost) = query.recency_boost {
            let _boosted = apply_temporal_boost_scored(&mut scored_nodes, boost);
        }

        // NEW: Hybrid temporal + semantic scoring (WP1)
        if let Some(w) = query.temporal_weight {
            apply_hybrid_temporal_scoring(&mut scored_nodes, w);
        }

        Ok(scored_nodes)
    }

    async fn retrieve_fractal(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
        // PostgreSQL doesn't have a fractal tree structure.
        // Fall back to hybrid_retrieve for the top-k results.
        let results = self.hybrid_retrieve(query).await?;

        // Take top-k, score defaults to 1.0 for fractal mode
        Ok(results
            .into_iter()
            .take(query.top_k)
            .map(|mut r| {
                r.score = 1.0;
                r
            })
            .collect())
    }

    async fn expand_fractal(
        &self,
        nodes: Vec<ScoredNode>,
        query_vector: &[f32],
        max_depth: usize,
        pruning_threshold: f32,
    ) -> anyhow::Result<Vec<ScoredNode>> {
        if max_depth == 0 || query_vector.is_empty() {
            return Ok(nodes);
        }

        let max_total = nodes
            .len()
            .saturating_add(Self::PG_EXPAND_FRACTAL_MAX_EXTRA);
        let mut expanded: Vec<ScoredNode> = Vec::with_capacity(nodes.len().saturating_mul(2));

        for scored in nodes {
            if expanded.len() >= max_total {
                break;
            }
            expanded.push(scored.clone());

            let node = &scored.node;
            let mut visited: HashSet<Uuid> = HashSet::new();
            visited.insert(node.id);

            if !node.children_tier_ids.is_empty() {
                self.expand_children_pg(
                    &node.children_tier_ids,
                    query_vector,
                    max_depth.saturating_sub(1),
                    pruning_threshold,
                    &mut visited,
                    &mut expanded,
                    max_total,
                )
                .await?;
            } else if let Some(pid) = node.parent_tier_id {
                let loaded = self.get_fractal_nodes_any(&[pid]).await?;
                if let Some(parent) = loaded.get(&pid) {
                    visited.insert(parent.id);
                    let parent_sim = crate::memory::fractal_node::cosine_similarity(
                        &parent.vector,
                        query_vector,
                    );
                    if parent_sim >= pruning_threshold && expanded.len() < max_total {
                        expanded.push(ScoredNode {
                            id: parent.id,
                            score: parent_sim,
                            distribution_scores: None,
                            debug: None,
                            node: parent.clone(),
                        });
                    }

                    if !parent.children_tier_ids.is_empty() {
                        self.expand_children_pg(
                            &parent.children_tier_ids,
                            query_vector,
                            max_depth.saturating_sub(1),
                            pruning_threshold,
                            &mut visited,
                            &mut expanded,
                            max_total,
                        )
                        .await?;
                    }
                }
            }
        }

        expanded.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(expanded)
    }

    async fn search_bm25(
        &self,
        query_text: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<(Uuid, f32)>> {
        self.search_bm25(query_text, top_k as i32).await
    }

    // --- Enumeration ---

    async fn list_all(&self) -> anyhow::Result<Vec<FractalNode>> {
        let rows = self.list_memories().await?;
        Ok(rows.into_iter().map(memory_row_to_fractal_node).collect())
    }

    async fn recent(&self, limit: usize) -> anyhow::Result<Vec<FractalNode>> {
        let rows = self.recent_memories(limit as i32).await?;
        Ok(rows.into_iter().map(memory_row_to_fractal_node).collect())
    }

    async fn count(&self) -> usize {
        self.count(None).await.unwrap_or(0) as usize
    }

    // --- Maintenance ---

    async fn purge_dummy_vectors(&self) -> usize {
        self.purge_dummy_vectors().await.unwrap_or(0)
    }

    async fn update(&self, id: &Uuid, op: UpdateOperation) -> anyhow::Result<()> {
        use crate::storage::UpdateOperation;
        match op {
            UpdateOperation::MultiplyWeight(factor) => {
                let query = sqlx::query_scalar::<_, f64>(
                    "UPDATE memories SET weight = weight * $1 WHERE id = $2 RETURNING id",
                )
                .bind(factor)
                .bind(*id);
                query.fetch_one(&self.pool).await?;
            }
            UpdateOperation::SetWeight(w) => {
                let query = sqlx::query("UPDATE memories SET weight = $1 WHERE id = $2")
                    .bind(w)
                    .bind(*id);
                query.execute(&self.pool).await?;
            }
            UpdateOperation::SetParentTierId(parent_id) => {
                let query = sqlx::query(
                    "UPDATE memories SET parent_tier_id = $1 WHERE id = $2 AND parent_tier_id IS NULL",
                )
                .bind(parent_id)
                .bind(*id);
                query.execute(&self.pool).await?;
            }
            UpdateOperation::SetOverviewContent(content) => {
                let query = sqlx::query("UPDATE memories SET overview_content = $1 WHERE id = $2")
                    .bind(content)
                    .bind(*id);
                query.execute(&self.pool).await?;
            }
            UpdateOperation::SetSummaryContent(content) => {
                let query = sqlx::query("UPDATE memories SET summary_content = $1 WHERE id = $2")
                    .bind(content)
                    .bind(*id);
                query.execute(&self.pool).await?;
            }
            UpdateOperation::SetStatus(status) => {
                let status_str = match status {
                    crate::memory::types::MemoryStatus::Active => "active",
                    crate::memory::types::MemoryStatus::Draft => "draft",
                    crate::memory::types::MemoryStatus::Archived => "archived",
                    crate::memory::types::MemoryStatus::Deleted => "deleted",
                    crate::memory::types::MemoryStatus::Superseded => "superseded",
                    crate::memory::types::MemoryStatus::Stale => "stale",
                };
                let query = sqlx::query("UPDATE memories SET status = $1 WHERE id = $2")
                    .bind(status_str)
                    .bind(*id);
                query.execute(&self.pool).await?;
            }
            UpdateOperation::ApplyAudit { weight, status } => {
                let status_str = status.map(|s| match s {
                    crate::memory::types::MemoryStatus::Active => "active",
                    crate::memory::types::MemoryStatus::Draft => "draft",
                    crate::memory::types::MemoryStatus::Archived => "archived",
                    crate::memory::types::MemoryStatus::Deleted => "deleted",
                    crate::memory::types::MemoryStatus::Superseded => "superseded",
                    crate::memory::types::MemoryStatus::Stale => "stale",
                });
                let query = sqlx::query(
                    "UPDATE memories SET weight = $1, status = COALESCE($2, status) WHERE id = $3",
                )
                .bind(weight)
                .bind(status_str)
                .bind(*id);
                query.execute(&self.pool).await?;
            }
            UpdateOperation::AddChildTierId(child_id) => {
                let query = sqlx::query(
                    "UPDATE memories SET children_tier_ids = array_append(COALESCE(children_tier_ids, ARRAY[]::uuid[]), $1) WHERE id = $2",
                )
                .bind(child_id)
                .bind(*id);
                query.execute(&self.pool).await?;
            }
        }
        Ok(())
    }
}

// =============================================================================
// Helper functions
// =============================================================================

/// Convert a MemoryRow into a FractalNode.
fn memory_row_to_fractal_node(row: MemoryRow) -> FractalNode {
    let metadata: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_value(row.metadata.clone().unwrap_or(serde_json::json!({})))
            .unwrap_or_default();

    // Parse structured fields from strings stored in the DB
    let memory_type = MemoryType::parse(&row.memory_type).unwrap_or(MemoryType::Episodic);
    let source = MemorySource::parse(&row.source).unwrap_or(MemorySource::Conversation);
    let sensitivity = Sensitivity::parse(&row.sensitivity).unwrap_or(Sensitivity::Normal);
    let status = MemoryStatus::parse(&row.status).unwrap_or(MemoryStatus::Active);
    let conflict_state = ConflictState::parse(&row.conflict_state).unwrap_or(ConflictState::None);

    // Parse tier fields from DB — now properly stored and retrieved
    let context_tier = row
        .context_tier
        .and_then(|t| ContextTier::parse(&t))
        .unwrap_or(ContextTier::Raw);
    let parent_tier_id = row.parent_tier_id;
    let children_tier_ids = row.children_tier_ids.unwrap_or_default();
    let summary_content = row.summary_content;
    let overview_content = row.overview_content;
    let children: Vec<FractalNode> = vec![];
    let relations: Vec<crate::memory::fractal_node::Relation> = vec![];
    let original_pointer: Option<String> = None;
    let multimodal: Option<crate::multimodal::MultimodalData> = None;

    FractalNode {
        id: row.id,
        memory_type,
        source,
        vector: row.embedding.unwrap_or_default(),
        content: if row.content.is_empty() {
            None
        } else {
            Some(row.content)
        },
        original_pointer,
        metadata,
        weight: 1.0,
        multimodal,
        children,
        children_tier_ids,
        relations,
        created_at: row.created_at,
        last_accessed: row.last_accessed.unwrap_or(row.created_at),
        confidence: row.confidence,
        sensitivity,
        superseded_by: row.superseded_by,
        conflict_state,
        provenance: row.provenance,
        importance: row.importance,
        status,
        access_count: row.access_count,
        context_tier,
        parent_tier_id,
        summary_content,
        overview_content,
        r_m: row.last_accessed.unwrap_or(row.created_at),
        n_m: 0,
    }
}

/// Convert a MemoryWithScore into a FractalNode.
fn memory_with_score_to_fractal_node(row: MemoryWithScore) -> Option<FractalNode> {
    let provenance = row.provenance.clone();

    let memory_type = MemoryType::parse(&row.memory_type).unwrap_or(MemoryType::Episodic);
    let source = MemorySource::parse(&row.source).unwrap_or(MemorySource::Conversation);
    let sensitivity = Sensitivity::parse(&row.sensitivity).unwrap_or(Sensitivity::Normal);
    let status = MemoryStatus::parse(&row.status).unwrap_or(MemoryStatus::Active);

    Some(FractalNode {
        id: row.id,
        memory_type,
        source,
        vector: row.embedding.unwrap_or_default(),
        content: if row.content.is_empty() {
            None
        } else {
            Some(row.content)
        },
        original_pointer: None,
        metadata: std::collections::HashMap::new(),
        weight: 1.0,
        multimodal: None,
        children: vec![],
        children_tier_ids: vec![],
        relations: vec![],
        created_at: row.created_at,
        last_accessed: row.last_accessed.unwrap_or(row.created_at),
        confidence: row.confidence.unwrap_or(0.0),
        sensitivity,
        superseded_by: None,
        conflict_state: ConflictState::None,
        provenance,
        importance: row.importance.unwrap_or(0),
        status,
        access_count: row.access_count.unwrap_or(0),
        context_tier: ContextTier::Raw,
        parent_tier_id: None,
        summary_content: None,
        overview_content: None,
        r_m: row.last_accessed.unwrap_or(row.created_at),
        n_m: 0,
    })
}

/// Reciprocal Rank Fusion — combines two result sets with rank scores.
fn rrf_fuse(vector_ids: &[Uuid], bm25_results: &[(Uuid, f32)], k: f32) -> Vec<(Uuid, f32)> {
    use std::collections::HashMap;

    let mut scores: HashMap<Uuid, f32> = HashMap::new();

    // Vector results get score 1/(k + rank)
    for (rank, id) in vector_ids.iter().enumerate() {
        let score = 1.0 / (k + (rank as f32 + 1.0));
        *scores.entry(*id).or_insert(0.0) += score;
    }

    // BM25 results get score 1/(k + rank)
    for (rank, (id, bm25_score)) in bm25_results.iter().enumerate() {
        let score = 1.0 / (k + (rank as f32 + 1.0));
        // Normalize BM25 score (typically 0-20 range) to 0-1
        let normalized_bm25 = (bm25_score / 20.0).min(1.0);
        *scores.entry(*id).or_insert(0.0) += score * (normalized_bm25 + 0.1);
    }

    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

/// Apply temporal recency boost to close-scoring ScoredNode results.
///
/// Same semantics as MemoryStore::apply_temporal_boost but operates on
/// ScoredNode slices (used by PostgresStore).
/// Improved hybrid temporal + semantic scoring.
/// Applies configurable weight between semantic similarity and global recency (exponential decay).
/// This is the core of Work Package 1.
fn apply_hybrid_temporal_scoring(results: &mut [ScoredNode], temporal_weight: f32) {
    if results.is_empty() || temporal_weight <= 0.0 {
        tracing::info!(
            results_empty = results.is_empty(),
            temporal_weight,
            "hybrid_temporal_scoring SKIPPED (empty or weight <= 0)"
        );
        return;
    }
    let w = temporal_weight.clamp(0.0, 0.8);
    let now = chrono::Utc::now();

    // Snapshot pre-scoring state
    let original_scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let original_top3_ids: Vec<String> = results.iter().take(3).map(|r| r.id.to_string()).collect();

    // Collect recency factors for diagnostics
    let mut recency_factors: Vec<f32> = Vec::with_capacity(results.len());
    
    for item in results.iter_mut() {
        let age_days = (now - item.node.created_at).num_days() as f32;
        // Half-life of 7 days chosen for typical conversational memory use cases.
// Note: There are currently two temporal mechanisms:
//   1. temporal_weight (hybrid semantic + global recency, this function)
//   2. recency_boost (legacy close-score boost, kept for backward compat)
// These are independent and can be combined.
        // With 21-day half-life, 0-2 day old data produced almost no variance (range ~0.064).
        // 7 days gives ~3x more differentiation for recent memories while still allowing
        // older relevant memories to compete when semantically strong.
        let recency_factor = 0.5f32.powf(age_days / 7.0).max(0.05);
        recency_factors.push(recency_factor);
        
        // Store debug info
        if let Some(debug) = &mut item.debug {
            debug.recency_factor = Some(recency_factor);
            debug.temporal_weight = Some(w);
            debug.explanation = Some(format!(
                "Hybrid score: semantic×{:.2} + recency({:.1}d)×{:.2}",
                1.0 - w, age_days, w
            ));
        }
        
        // Hybrid score: semantic * (1-w) + recency * w
        item.score = item.score * (1.0 - w) + recency_factor * w;
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Post-scoring snapshot
    let new_scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let new_top3_ids: Vec<String> = results.iter().take(3).map(|r| r.id.to_string()).collect();

    // Compute deltas
    let max_delta = original_scores.iter().zip(new_scores.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let mean_delta = original_scores.iter().zip(new_scores.iter())
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>() / results.len() as f32;
    let recency_min = recency_factors.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let recency_max = recency_factors.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let recency_mean = recency_factors.iter().sum::<f32>() / recency_factors.len() as f32;
    let recency_range = recency_max - recency_min;
    let reorder_happened = original_top3_ids != new_top3_ids;
    
    tracing::info!(
        weight = w,
        nodes = results.len(),
        score_range_before = format!("{:.4}-{:.4}", 
            original_scores.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
            original_scores.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))),
        score_range_after = format!("{:.4}-{:.4}",
            new_scores.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
            new_scores.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))),
        max_delta = format!("{:.4}", max_delta),
        mean_delta = format!("{:.4}", mean_delta),
        recency_factor_range = format!("{:.4}-{:.4}", recency_min, recency_max),
        recency_factor_mean = format!("{:.4}", recency_mean),
        recency_range = format!("{:.4}", recency_range),
        top3_reordered = reorder_happened,
        "hybrid_temporal_scoring applied (WP1) — diagnostics"
    );
}

/// Legacy close-score recency boost (kept for backward compat)
fn apply_temporal_boost_scored(results: &mut [ScoredNode], recency_boost: f32) -> usize {
    let mut boosted = 0usize;
    if results.is_empty() {
        return boosted;
    }
    let newest = results.iter().map(|n| n.node.created_at).max();
    let Some(newest) = newest else { return boosted };

    let oldest = results
        .iter()
        .map(|n| n.node.created_at)
        .min()
        .unwrap_or(newest);
    let time_range = (newest - oldest).num_seconds() as f32;
    if time_range < 1.0 {
        return boosted;
    }

    let max_score = results
        .iter()
        .map(|n| n.score)
        .fold(f32::NEG_INFINITY, f32::max);
    let closeness_threshold = recency_boost * 0.5;

    for item in results.iter_mut() {
        if (max_score - item.score).abs() <= closeness_threshold {
            let age_seconds = (newest - item.node.created_at).num_seconds() as f32;
            let recency_factor = 1.0 - (age_seconds / time_range).clamp(0.0, 1.0);
            item.score += recency_boost * recency_factor;
            boosted += 1;
        }
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    tracing::info!(
        boosted,
        total = results.len(),
        boost_factor = recency_boost,
        time_range_s = time_range,
        "temporal_boost_scored applied"
    );
    boosted
}

// =============================================================================
// Row Types (matching the SQL schema)
// =============================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemoryRow {
    pub id: Uuid,
    pub memory_type: String,
    pub content: String,
    pub content_preview: Option<String>,
    pub importance: i32,
    pub confidence: f64,
    pub sensitivity: String,
    pub status: String,
    pub superseded_by: Option<Uuid>,
    pub conflict_state: String,
    pub source: String,
    pub source_id: Option<String>,
    pub provenance: serde_json::Value,
    pub parent_id: Option<Uuid>,
    pub depth: i32,
    pub access_count: i32,
    pub last_accessed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
    pub entities: Option<serde_json::Value>,
    pub tags: Vec<String>,
    /// Dense vector embedding (stored as f32 array, deserialized from PostgreSQL vector type).
    pub embedding: Option<Vec<f32>>,
    // -- Fractal tier fields --
    pub context_tier: Option<String>,
    pub parent_tier_id: Option<Uuid>,
    pub children_tier_ids: Option<Vec<Uuid>>,
    pub summary_content: Option<String>,
    pub overview_content: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemoryWithScore {
    pub id: Uuid,
    pub memory_type: String,
    pub content: String,
    pub content_preview: Option<String>,
    pub importance: Option<i32>,
    pub confidence: Option<f64>,
    pub sensitivity: String,
    pub status: String,
    pub source: String,
    pub source_id: Option<String>,
    pub provenance: serde_json::Value,
    pub access_count: Option<i32>,
    pub last_accessed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub similarity: Option<f64>,
    /// Dense vector embedding (stored as f32 array, deserialized from PostgreSQL vector type).
    pub embedding: Option<Vec<f32>>,
    // -- Fractal tier fields --
    pub context_tier: Option<String>,
    pub parent_tier_id: Option<Uuid>,
    pub children_tier_ids: Option<Vec<Uuid>>,
    pub summary_content: Option<String>,
    pub overview_content: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EdgeRow {
    pub id: Uuid,
    pub from_node_id: Uuid,
    pub to_node_id: Uuid,
    pub edge_type: String,
    pub strength: f64,
    pub confidence: f64,
    pub causality: bool,
    pub bidirectional: bool,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConsolidationRow {
    pub id: Uuid,
    pub consolidation_date: chrono::NaiveDate,
    pub session_id: Option<String>,
    pub conversation_id: Option<String>,
    pub memories_processed: i32,
    pub new_memories_created: i32,
    pub edges_created: i32,
    pub processing_time_ms: i32,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Row type for auth_users table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuthUserRow {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Row type for auth_api_keys table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuthApiKeyRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub key_hash: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

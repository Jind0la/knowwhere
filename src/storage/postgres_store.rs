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

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::fractal_node::{FractalNode, NodeType};
use crate::memory::types::{ConflictState, ContextTier, MemorySource, MemoryStatus, Sensitivity};
use crate::memory::MemoryType;
use crate::storage::backend::{HybridQuery, ScoredNode, StorageBackend, UpdateOperation};

/// PostgreSQL-backed storage layer.
/// Wraps the existing in-memory USearch index with persistent PostgreSQL storage.
pub struct PostgresStore {
    pool: PgPool,
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

    // -------------------------------------------------------------------------
    // Event Log (Layer 0 — immutable)
    // -------------------------------------------------------------------------

    /// Append an event to the immutable event log.
    pub async fn append_event(&self, event_type: &str, payload: &serde_json::Value) -> Result<Uuid> {
        let id = Uuid::new_v4();
        sqlx::query!(
            r#"
            INSERT INTO events (id, event_type, payload, created_at)
            VALUES ($1, $2, $3, NOW())
            "#,
            id,
            event_type,
            payload,
        )
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Read events for replay (used for rebuilding state from event log).
    pub async fn read_events(
        &self,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<Event>> {
        let rows = if let Some(after) = after_id {
            sqlx::query_as!(
                Event,
                r#"
                SELECT id, event_type, payload, created_at
                FROM events
                WHERE id > $1
                ORDER BY created_at ASC
                LIMIT $2
                "#,
                after,
                limit
            )
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as!(
                Event,
                r#"
                SELECT id, event_type, payload, created_at
                FROM events
                ORDER BY created_at ASC
                LIMIT $1
                "#,
                limit
            )
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Memory CRUD
    // -------------------------------------------------------------------------

    /// Store a session memory node.
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
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let memory_type_str = match memory_type {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Preference => "preference",
            MemoryType::Procedural => "procedural",
            MemoryType::Meta => "meta",
        };

        sqlx::query!(
            r#"
            INSERT INTO memories (
                id, memory_type, content, embedding, entities, tags,
                provenance, source, source_id, importance, confidence,
                sensitivity, status, access_count, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active', 0, NOW(), NOW())
            "#,
            id,
            memory_type_str,
            content,
            embedding as _,
            serde_json::json!(entities),
            &tags,
            provenance,
            source,
            source_id,
            importance as _,
            confidence,
            sensitivity,
        )
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
        let row = sqlx::query_as!(
            MemoryRow,
            r#"
            SELECT id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!", sensitivity as "sensitivity!",
                   status as "status!", conflict_state as "conflict_state!",
                   source as "source!", depth as "depth!",
                   access_count as "access_count!",
                   created_at as "created_at!", updated_at as "updated_at!",
                   superseded_by, source_id, provenance, parent_id,
                   last_accessed, deleted_at, metadata, entities,
                   COALESCE(tags, ARRAY[]::TEXT[]) as tags,
                   content_preview,
                   embedding as "embedding: _"
            FROM memories
            WHERE id = $1 AND status != 'deleted'
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Update access statistics.
    pub async fn record_access(&self, id: Uuid) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE memories
            SET access_count = access_count + 1, last_accessed = NOW()
            WHERE id = $1
            "#,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark a memory as superseded by another.
    pub async fn supersede(&self, memory_id: Uuid, superseded_by_id: Uuid) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE memories
            SET status = 'superseded', superseded_by = $2, updated_at = NOW()
            WHERE id = $1
            "#,
            memory_id,
            superseded_by_id
        )
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
        sqlx::query!(
            r#"
            UPDATE memories
            SET status = $2, updated_at = NOW()
            WHERE id = $1
            "#,
            id,
            status
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Soft delete a memory.
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE memories
            SET status = 'deleted', deleted_at = NOW(), updated_at = NOW()
            WHERE id = $1
            "#,
            id
        )
        .execute(&self.pool)
        .await?;

        self.append_event("memory_deleted", &serde_json::json!({ "memory_id": id.to_string() }))
            .await?;

        Ok(())
    }

    /// Update a memory's embedding vector.
    pub async fn update_vector(&self, id: Uuid, new_embedding: Vec<f32>) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            UPDATE memories
            SET embedding = $2, updated_at = NOW()
            WHERE id = $1 AND status != 'deleted'
            "#,
            id,
            new_embedding as _
        )
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

        let rows: Vec<Bm25Row> = sqlx::query_as!(
            Bm25Row,
            r#"
            SELECT id, COALESCE(ts_rank(to_tsvector('english', content), plainto_tsquery('english', $1)), 0.0)::float8 AS "rank!"
            FROM memories
            WHERE status = 'active'
              AND to_tsvector('english', content) @@ plainto_tsquery('english', $1)
            ORDER BY rank DESC
            LIMIT $2
            "#,
            query_text,
            top_k
        )
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
            "#
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
        // Build dynamic query with optional filters
        let rows = if let Some(mt) = memory_type {
            if let Some(mi) = min_importance {
                sqlx::query_as!(
                    MemoryWithScore,
                    r#"
                    SELECT id as "id!", memory_type as "memory_type!",
                           content as "content!", importance as "importance!",
                           COALESCE(confidence, 0.0) as "confidence!",
                           sensitivity as "sensitivity!",
                           status as "status!", source as "source!",
                           access_count as "access_count!",
                           created_at as "created_at!", updated_at as "updated_at!",
                           source_id, provenance, last_accessed,
                           content_preview,
                           COALESCE((1 - (embedding <=> $1::vector))::float, 0.0) AS "similarity: f64",
                           embedding as "embedding: _"
                    FROM memories
                    WHERE status = 'active'
                      AND embedding IS NOT NULL
                      AND memory_type = $4
                      AND importance >= $5
                    ORDER BY embedding <=> $1::vector
                    LIMIT $2
                    "#,
                    embedding as _,
                    limit,
                    mt,
                    mi as _
                )
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query_as!(
                    MemoryWithScore,
                    r#"
                    SELECT id as "id!", memory_type as "memory_type!",
                           content as "content!", importance as "importance!",
                           COALESCE(confidence, 0.0) as "confidence!",
                           sensitivity as "sensitivity!",
                           status as "status!", source as "source!",
                           access_count as "access_count!",
                           created_at as "created_at!", updated_at as "updated_at!",
                           source_id, provenance,
                           last_accessed,
                           content_preview,
                           COALESCE((1 - (embedding <=> $1::vector))::float, 0.0) AS "similarity: f64",
                           embedding as "embedding: _"
                    FROM memories
                    WHERE status = 'active'
                      AND embedding IS NOT NULL
                      AND memory_type = $3
                    ORDER BY embedding <=> $1::vector
                    LIMIT $2
                    "#,
                    embedding as _,
                    limit,
                    mt
                )
                .fetch_all(&self.pool)
                .await?
            }
        } else {
            sqlx::query_as!(
                MemoryWithScore,
                r#"
                SELECT id as "id!", memory_type as "memory_type!",
                       content as "content!", importance as "importance!",
                       COALESCE(confidence, 0.0) as "confidence!",
                       sensitivity as "sensitivity!",
                       status as "status!", source as "source!",
                       access_count as "access_count!",
                       created_at as "created_at!", updated_at as "updated_at!",
                       source_id, provenance,
                       last_accessed,
                       content_preview,
                       COALESCE((1 - (embedding <=> $1::vector))::float, 0.0) AS "similarity: f64",
                       embedding as "embedding: _"
                FROM memories
                WHERE status = 'active'
                  AND embedding IS NOT NULL
                ORDER BY embedding <=> $1::vector
                LIMIT $2
                "#,
                embedding as _,
                limit
            )
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    /// Recent memories.
    pub async fn recent_memories(&self, limit: i32) -> Result<Vec<MemoryRow>> {
        let rows = sqlx::query_as!(
            MemoryRow,
            r#"
            SELECT id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!", sensitivity as "sensitivity!",
                   status as "status!", conflict_state as "conflict_state!",
                   source as "source!", depth as "depth!",
                   access_count as "access_count!",
                   created_at as "created_at!", updated_at as "updated_at!",
                   superseded_by, source_id, provenance, parent_id,
                   last_accessed, deleted_at, metadata, entities,
                   COALESCE(tags, ARRAY[]::TEXT[]) as tags,
                   content_preview,
                   embedding as "embedding: _"
            FROM memories
            WHERE status = 'active'
            ORDER BY created_at DESC
            LIMIT $1
            "#,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List all active memories (for list_all).
    pub async fn list_memories(&self) -> Result<Vec<MemoryRow>> {
        let rows = sqlx::query_as!(
            MemoryRow,
            r#"
            SELECT id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!", sensitivity as "sensitivity!",
                   status as "status!", conflict_state as "conflict_state!",
                   source as "source!", depth as "depth!",
                   access_count as "access_count!",
                   created_at as "created_at!", updated_at as "updated_at!",
                   superseded_by, source_id, provenance, parent_id,
                   last_accessed, deleted_at, metadata, entities,
                   COALESCE(tags, ARRAY[]::TEXT[]) as tags,
                   content_preview,
                   embedding as "embedding: _"
            FROM memories
            WHERE status = 'active'
            ORDER BY created_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Count total active memories.
    pub async fn count(&self, memory_type: Option<&str>) -> Result<i64> {
        let count: (i64,) = if let Some(mt) = memory_type {
            sqlx::query_as!(
                _,
                r#"
                SELECT COUNT(*)::bigint
                FROM memories
                WHERE status = 'active' AND memory_type = $1
                "#,
                mt
            )
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_as!(
                _,
                r#"
                SELECT COUNT(*)::bigint
                FROM memories
                WHERE status = 'active'
                "#
            )
            .fetch_one(&self.pool)
            .await?
        };
        Ok(count.0)
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
        sqlx::query!(
            r#"
            INSERT INTO knowledge_edges
                (id, from_node_id, to_node_id, edge_type, strength, confidence, bidirectional, reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
            ON CONFLICT (from_node_id, to_node_id, edge_type) DO NOTHING
            "#,
            id,
            from_node_id,
            to_node_id,
            edge_type,
            strength,
            confidence,
            bidirectional,
            reason
        )
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get edges from a node.
    pub async fn get_edges(&self, from_node_id: Uuid) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as!(
            EdgeRow,
            r#"
            SELECT id as "id!", from_node_id as "from_node_id!", to_node_id as "to_node_id!",
                   edge_type as "edge_type!", strength as "strength!",
                   confidence as "confidence!", causality as "causality!",
                   bidirectional as "bidirectional!",
                   reason, created_at as "created_at!", metadata
            FROM knowledge_edges
            WHERE from_node_id = $1 OR to_node_id = $1
            "#,
            from_node_id
        )
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
        sqlx::query!(
            r#"
            UPDATE memories
            SET parent_id = $2, depth = (SELECT depth + 1 FROM memories WHERE id = $2), updated_at = NOW()
            WHERE id = $1
            "#,
            memory_id,
            parent_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get fractal children (for zoom retrieval).
    pub async fn get_children(&self, memory_id: Uuid, max_depth: i32) -> Result<Vec<MemoryRow>> {
        let rows = sqlx::query_as!(
            MemoryRow,
            r#"
            WITH RECURSIVE fractal_tree AS (
                SELECT id as "id!", memory_type as "memory_type!",
                       content as "content!", importance as "importance!",
                       confidence as "confidence!", sensitivity as "sensitivity!",
                       status as "status!", conflict_state as "conflict_state!",
                       source as "source!", depth as "depth!",
                       access_count as "access_count!",
                       created_at as "created_at!", updated_at as "updated_at!",
                       superseded_by, source_id, provenance, parent_id,
                       last_accessed, deleted_at, metadata, entities,
                       COALESCE(tags, ARRAY[]::TEXT[]) as tags,
                       embedding as "embedding: _",
                       content_preview,
                       1 AS level
                FROM memories
                WHERE parent_id = $1 AND status = 'active'

                UNION ALL

                SELECT m.id as "id!", m.memory_type as "memory_type!",
                       m.content as "content!", m.importance as "importance!",
                       m.confidence as "confidence!", m.sensitivity as "sensitivity!",
                       m.status as "status!", m.conflict_state as "conflict_state!",
                       m.source as "source!", m.depth as "depth!",
                       m.access_count as "access_count!",
                       m.created_at as "created_at!", m.updated_at as "updated_at!",
                       m.superseded_by, m.source_id, m.provenance, m.parent_id,
                       m.last_accessed, m.deleted_at, m.metadata, m.entities,
                       COALESCE(m.tags, ARRAY[]::TEXT[]) as tags,
                       m.embedding as "embedding: _",
                       m.content_preview,
                       ft.level + 1 AS level
                FROM memories m
                INNER JOIN fractal_tree ft ON m.parent_id = ft.id
                WHERE m.status = 'active' AND ft.level < $2
            )
            SELECT
                   -- Non-nullable primitives: use COALESCE to satisfy sqlx
                   id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!", sensitivity as "sensitivity!",
                   status as "status!", conflict_state as "conflict_state!",
                   source as "source!", depth as "depth!",
                   access_count as "access_count!",
                   created_at as "created_at!", updated_at as "updated_at!",
                   -- Nullable but should have values
                   COALESCE(content_preview, ''::text) as content_preview,
                   COALESCE(superseded_by, '00000000-0000-0000-0000-000000000000'::uuid) as superseded_by,
                   COALESCE(source_id, ''::text) as source_id,
                   provenance, parent_id, last_accessed, deleted_at,
                   metadata, entities,
                   COALESCE(tags, ARRAY[]::TEXT[]) as tags,
                   embedding as "embedding: _"
            FROM fractal_tree
            ORDER BY level
            "#,
            memory_id,
            max_depth
        )
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
        sqlx::query!(
            r#"
            INSERT INTO consolidation_history (
                id, consolidation_date, session_id, conversation_id,
                memories_processed, new_memories_created, edges_created,
                processing_time_ms, status, error_message, created_at
            )
            VALUES ($1, CURRENT_DATE, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
            "#,
            id,
            session_id,
            conversation_id,
            memories_processed as _,
            new_memories_created as _,
            edges_created as _,
            processing_time_ms,
            status,
            error_message
        )
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get recent consolidation runs.
    pub async fn recent_consolidations(&self, limit: i32) -> Result<Vec<ConsolidationRow>> {
        let rows = sqlx::query_as!(
            ConsolidationRow,
            r#"
            SELECT id as "id!", consolidation_date,
                   session_id, conversation_id,
                   memories_processed as "memories_processed!",
                   new_memories_created as "new_memories_created!",
                   edges_created as "edges_created!",
                   processing_time_ms as "processing_time_ms!",
                   status as "status!",
                   error_message, created_at as "created_at!"
            FROM consolidation_history
            ORDER BY created_at DESC
            LIMIT $1::bigint
            "#,
            limit as i64
        )
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
        sqlx::query!(
            r#"
            INSERT INTO audit_log (id, run_id, issue_type, memory_id, severity, description, action_taken, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            "#,
            id,
            run_id,
            issue_type,
            memory_id,
            severity,
            description,
            action_taken
        )
        .execute(&self.pool)
        .await?;
        Ok(id)
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
        let metadata = serde_json::to_value(&node.metadata)
            .unwrap_or(serde_json::json!({}));

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
        let vector = query
            .query_vector
            .as_deref()
            .unwrap_or(&[]);

        // If only text is provided, fall back to BM25
        if query.query_text.is_some() && query.query_vector.is_none() {
            let text = query.query_text.as_ref().unwrap();
            let bm25_results = self.search_bm25(text, query.top_k as i32).await?;
            // Convert BM25 results to ScoredNodes by fetching full nodes
            let mut scored_nodes = Vec::new();
            for (id, score) in bm25_results {
                if let Some(node) = self.get(&id).await? {
                    scored_nodes.push(ScoredNode { id, score, node });
                }
            }
            return Ok(scored_nodes);
        }

        // Vector search (with optional BM25 boost)
        let rows = self
            .vector_search(vector, query.top_k as i32, None, None)
            .await?;

        // If no text query, return pure vector results
        if query.query_text.is_none() {
            return Ok(rows
                .into_iter()
                .filter_map(|row| {
                    let row_vector = row.embedding.clone().unwrap_or_default();
                    let node = memory_with_score_to_fractal_node(row)?;
                    let sim = crate::memory::fractal_node::cosine_similarity(&row_vector, vector);
                    Some(ScoredNode {
                        id: node.id,
                        score: sim,
                        node,
                    })
                })
                .collect());
        }

        // Hybrid: combine vector + BM25 via RRF
        let bm25_text = query.query_text.as_ref().unwrap();
        let bm25_results = self.search_bm25(bm25_text, query.top_k as i32).await?;
        let bm25_ids: Vec<(Uuid, f32)> = bm25_results;

        let vector_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();

        let fused = rrf_fuse(&vector_ids, &bm25_ids, 60.0);

        let mut scored_nodes = Vec::new();
        for (id, score) in fused {
            if let Some(node) = self.get(&id).await? {
                scored_nodes.push(ScoredNode { id, score, node });
            }
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

    async fn search_bm25(&self, query_text: &str, top_k: usize) -> anyhow::Result<Vec<(Uuid, f32)>> {
        self.search_bm25(query_text, top_k as i32).await
    }

    // --- Enumeration ---

    async fn list_all(&self) -> anyhow::Result<Vec<FractalNode>> {
        let rows = self.list_memories().await?;
        Ok(rows
            .into_iter()
            .map(memory_row_to_fractal_node)
            .collect())
    }

    async fn recent(&self, limit: usize) -> anyhow::Result<Vec<FractalNode>> {
        let rows = self.recent_memories(limit as i32).await?;
        Ok(rows
            .into_iter()
            .map(memory_row_to_fractal_node)
            .collect())
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
                let query = sqlx::query(
                    "UPDATE memories SET weight = $1 WHERE id = $2",
                )
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
            UpdateOperation::SetStatus(status) => {
                let status_str = match status {
                    crate::memory::types::MemoryStatus::Active => "active",
                    crate::memory::types::MemoryStatus::Draft => "draft",
                    crate::memory::types::MemoryStatus::Archived => "archived",
                    crate::memory::types::MemoryStatus::Deleted => "deleted",
                    crate::memory::types::MemoryStatus::Superseded => "superseded",
                    crate::memory::types::MemoryStatus::Stale => "stale",
                };
                let query = sqlx::query(
                    "UPDATE memories SET status = $1 WHERE id = $2",
                )
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
        serde_json::from_value(row.metadata.clone()).unwrap_or_default();

    // Parse structured fields from strings stored in the DB
    let memory_type =
        MemoryType::from_str(&row.memory_type).unwrap_or(MemoryType::Episodic);
    let source = MemorySource::from_str(&row.source).unwrap_or(MemorySource::Conversation);
    let sensitivity =
        Sensitivity::from_str(&row.sensitivity).unwrap_or(Sensitivity::Normal);
    let status = MemoryStatus::from_str(&row.status).unwrap_or(MemoryStatus::Active);
    let conflict_state =
        ConflictState::from_str(&row.conflict_state).unwrap_or(ConflictState::None);

    // Fields not stored per-row — use sensible defaults
    let context_tier = ContextTier::Raw;
    let parent_tier_id: Option<Uuid> = None;
    let summary_content: Option<String> = None;
    let overview_content: Option<String> = None;
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
    }
}

/// Convert a MemoryWithScore into a FractalNode.
fn memory_with_score_to_fractal_node(row: MemoryWithScore) -> Option<FractalNode> {
    let provenance = row.provenance.clone();

    let memory_type =
        MemoryType::from_str(&row.memory_type).unwrap_or(MemoryType::Episodic);
    let source = MemorySource::from_str(&row.source).unwrap_or(MemorySource::Conversation);
    let sensitivity =
        Sensitivity::from_str(&row.sensitivity).unwrap_or(Sensitivity::Normal);
    let status = MemoryStatus::from_str(&row.status).unwrap_or(MemoryStatus::Active);

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
        relations: vec![],
        created_at: row.created_at,
        last_accessed: row.last_accessed.unwrap_or(row.created_at),
        confidence: row.confidence,
        sensitivity,
        superseded_by: None,
        conflict_state: ConflictState::None,
        provenance,
        importance: row.importance,
        status,
        access_count: row.access_count,
        context_tier: ContextTier::Raw,
        parent_tier_id: None,
        summary_content: None,
        overview_content: None,
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
    pub metadata: serde_json::Value,
    pub entities: serde_json::Value,
    pub tags: Vec<String>,
    /// Dense vector embedding (stored as f32 array, deserialized from PostgreSQL vector type).
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemoryWithScore {
    pub id: Uuid,
    pub memory_type: String,
    pub content: String,
    pub content_preview: Option<String>,
    pub importance: i32,
    pub confidence: f64,
    pub sensitivity: String,
    pub status: String,
    pub source: String,
    pub source_id: Option<String>,
    pub provenance: serde_json::Value,
    pub access_count: i32,
    pub last_accessed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub similarity: f64,
    /// Dense vector embedding (stored as f32 array, deserialized from PostgreSQL vector type).
    pub embedding: Option<Vec<f32>>,
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

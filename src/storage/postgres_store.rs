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
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::fractal_node::{FractalNode, NodeType};
use crate::memory::MemoryType;

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
            SELECT id, memory_type, content, content_preview,
                   importance, confidence, sensitivity, status,
                   superseded_by, conflict_state, source, source_id,
                   provenance, parent_id, depth,
                   access_count, last_accessed,
                   created_at, updated_at, deleted_at, metadata,
                   entities, tags
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
                    SELECT id, memory_type, content, content_preview,
                           importance, confidence, sensitivity, status,
                           source, source_id, provenance,
                           access_count, last_accessed,
                           created_at, updated_at,
                           (1 - (embedding <=> $1::vector))::float AS similarity
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
                    embedding as _,
                    mt,
                    mi as _
                )
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query_as!(
                    MemoryWithScore,
                    r#"
                    SELECT id, memory_type, content, content_preview,
                           importance, confidence, sensitivity, status,
                           source, source_id, provenance,
                           access_count, last_accessed,
                           created_at, updated_at,
                           (1 - (embedding <=> $1::vector))::float AS similarity
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
                SELECT id, memory_type, content, content_preview,
                       importance, confidence, sensitivity, status,
                       source, source_id, provenance,
                       access_count, last_accessed,
                       created_at, updated_at,
                       (1 - (embedding <=> $1::vector))::float AS similarity
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
            SELECT id, memory_type, content, content_preview,
                   importance, confidence, sensitivity, status,
                   superseded_by, conflict_state, source, source_id,
                   provenance, parent_id, depth,
                   access_count, last_accessed,
                   created_at, updated_at, deleted_at, metadata,
                   entities, tags
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
            SELECT id, from_node_id, to_node_id, edge_type, strength,
                   confidence, causality, bidirectional, reason, created_at, metadata
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
                SELECT id, memory_type, content, content_preview,
                       importance, confidence, sensitivity, status,
                       superseded_by, conflict_state, source, source_id,
                       provenance, parent_id, depth,
                       access_count, last_accessed,
                       created_at, updated_at, deleted_at, metadata,
                       entities, tags,
                       1 AS level
                FROM memories
                WHERE parent_id = $1 AND status = 'active'

                UNION ALL

                SELECT m.id, m.memory_type, m.content, m.content_preview,
                       m.importance, m.confidence, m.sensitivity, m.status,
                       m.superseded_by, m.conflict_state, m.source, m.source_id,
                       m.provenance, m.parent_id, m.depth,
                       m.access_count, m.last_accessed,
                       m.created_at, m.updated_at, m.deleted_at, m.metadata,
                       m.entities, m.tags,
                       ft.level + 1
                FROM memories m
                INNER JOIN fractal_tree ft ON m.parent_id = ft.id
                WHERE m.status = 'active' AND ft.level < $2
            )
            SELECT id, memory_type, content, content_preview,
                   importance, confidence, sensitivity, status,
                   superseded_by, conflict_state, source, source_id,
                   provenance, parent_id, depth,
                   access_count, last_accessed,
                   created_at, updated_at, deleted_at, metadata,
                   entities, tags
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
            SELECT id, consolidation_date, session_id, conversation_id,
                   memories_processed, new_memories_created, edges_created,
                   processing_time_ms, status, error_message, created_at
            FROM consolidation_history
            ORDER BY created_at DESC
            LIMIT $1
            "#,
            limit
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
// Row Types (matching the SQL schema)
// =============================================================================

sqlx::impl_for!(Event in "postgres");
sqlx::impl_for!(MemoryRow in "postgres");
sqlx::impl_for!(MemoryWithScore in "postgres");
sqlx::impl_for!(EdgeRow in "postgres");
sqlx::impl_for!(ConsolidationRow in "postgres");

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

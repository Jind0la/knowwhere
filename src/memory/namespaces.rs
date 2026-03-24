//! Directory Namespace — hierarchical address space for memories.
//!
//! Namespaces organize memories by kind (user, agent, resources, session)
//! using a path-based hierarchy similar to `viking://` URIs.
//!
//! Example paths:
//! - `user/preferences` — user settings
//! - `agent/skills` — agent capabilities
//! - `resources/docs` — external document references

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
use utoipa::ToSchema;

use super::types::MemoryType;

/// A named namespace for organizing memories.
#[cfg(feature = "postgres-storage")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MemoryNamespace {
    /// Unique identifier.
    pub id: Uuid,
    /// Hierarchical path, e.g. `user/preferences`.
    pub path: String,
    /// Depth in the path hierarchy (e.g. `a/b/c` has depth 3).
    pub depth: i32,
    /// Parent namespace ID (null for root-level namespaces).
    pub parent_id: Option<Uuid>,
    /// Human-readable description of what this namespace contains.
    pub description: Option<String>,
    /// Optional hint about the type of memories typically stored here.
    pub memory_type_hint: Option<String>,
}

impl MemoryNamespace {
    /// Returns the last component of the path (e.g. `preferences` from `user/preferences`).
    pub fn name(&self) -> &str {
        self.path.rsplit_once('/').map(|(_, n)| n).unwrap_or(&self.path)
    }

    /// Returns the parent path, e.g. `user` from `user/preferences`.
    pub fn parent_path(&self) -> Option<&str> {
        self.path.rsplit_once('/').map(|(p, _)| p)
    }
}

/// Database-backed store for memory namespaces.
#[cfg(feature = "postgres-storage")]
pub struct NamespaceStore<'a> {
    pool: &'a PgPool,
}

#[cfg(feature = "postgres-storage")]
impl<'a> NamespaceStore<'a> {
    /// Create a new NamespaceStore backed by the given connection pool.
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Look up a namespace by its path.
    pub async fn find_by_path(&self, path: &str) -> anyhow::Result<Option<MemoryNamespace>> {
        let row = sqlx::query_as!(
            MemoryNamespaceRow,
            r#"
            SELECT id as "id!", path as "path!", depth as "depth!",
                   parent_id, description, memory_type_hint
            FROM memory_namespaces
            WHERE path = $1
            "#,
            path,
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// List all namespaces ordered by path.
    pub async fn list_all(&self) -> anyhow::Result<Vec<MemoryNamespace>> {
        let rows = sqlx::query_as!(
            MemoryNamespaceRow,
            r#"
            SELECT id as "id!", path as "path!", depth as "depth!",
                   parent_id, description, memory_type_hint
            FROM memory_namespaces
            ORDER BY path ASC
            "#,
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Create a new namespace.
    /// Returns the assigned UUID.
    pub async fn create(&self, ns: &MemoryNamespace) -> anyhow::Result<Uuid> {
        let id = sqlx::query_scalar!(
            r#"
            INSERT INTO memory_namespaces (id, path, depth, parent_id, description, memory_type_hint)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (path) DO UPDATE SET
                description = EXCLUDED.description,
                memory_type_hint = EXCLUDED.memory_type_hint
            RETURNING id
            "#,
            ns.id,
            ns.path,
            ns.depth,
            ns.parent_id,
            ns.description,
            ns.memory_type_hint,
        )
        .fetch_one(self.pool)
        .await?;

        Ok(id)
    }

    /// List direct child namespaces of a parent.
    pub async fn children(&self, parent_id: Uuid) -> anyhow::Result<Vec<MemoryNamespace>> {
        let rows = sqlx::query_as!(
            MemoryNamespaceRow,
            r#"
            SELECT id as "id!", path as "path!", depth as "depth!",
                   parent_id, description, memory_type_hint
            FROM memory_namespaces
            WHERE parent_id = $1
            ORDER BY path ASC
            "#,
            parent_id,
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// List memories within a namespace, ordered by creation time (most recent first).
    ///
    /// Returns `MemoryRow` objects that contain the basic memory fields.
    /// Use this for browsing memories inside a namespace.
    pub async fn memories_in_namespace(
        &self,
        namespace_id: Uuid,
        limit: i32,
    ) -> anyhow::Result<Vec<MemoryRow>> {
                let rows = sqlx::query_as!(
            MemoryRow,
            r#"
            SELECT
                id as "id!",
                memory_type as "memory_type!",
                content,
                embedding as "embedding: _",
                (COALESCE(importance, 0)::float4) as "importance!: f32",
                (COALESCE(confidence, 0)::float4) as "confidence!: f32",
                sensitivity,
                status,
                access_count as "access_count!",
                created_at as "created_at!",
                updated_at as "updated_at!",
                namespace_id,
                parent_tier_id,
                context_tier::text AS context_tier,
                energy,
                content_hash,
                semantic_thumbnail,
                provenance,
                entities,
                COALESCE(tags, ARRAY[]::TEXT[]) as tags,
                source,
                source_id
            FROM memories
            WHERE namespace_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            "#,
            namespace_id,
            limit as i64,
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows)
    }

    /// Get a namespace and its direct children (one level).
    pub async fn with_children(&self, namespace_id: Uuid) -> anyhow::Result<(MemoryNamespace, Vec<MemoryNamespace>)> {
        let ns = sqlx::query_as!(
            MemoryNamespaceRow,
            r#"
            SELECT id as "id!", path as "path!", depth as "depth!",
                   parent_id, description, memory_type_hint
            FROM memory_namespaces
            WHERE id = $1
            "#,
            namespace_id,
        )
        .fetch_optional(self.pool)
        .await?
        .map(|r| r.into())
        .ok_or_else(|| anyhow::anyhow!("namespace {namespace_id} not found"))?;

        let children = self.children(namespace_id).await?;
        Ok((ns, children))
    }
}

// Internal row type matching the SQLx query shape.
#[cfg(feature = "postgres-storage")]
#[derive(Debug, sqlx::FromRow)]
struct MemoryNamespaceRow {
    id: Uuid,
    path: String,
    depth: i32,
    parent_id: Option<Uuid>,
    description: Option<String>,
    memory_type_hint: Option<String>,
}

impl From<MemoryNamespaceRow> for MemoryNamespace {
    fn from(row: MemoryNamespaceRow) -> Self {
        MemoryNamespace {
            id: row.id,
            path: row.path,
            depth: row.depth,
            parent_id: row.parent_id,
            description: row.description,
            memory_type_hint: row.memory_type_hint,
        }
    }
}

// Partial memory row used for namespace-scoped browsing.
#[cfg(feature = "postgres-storage")]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct MemoryRow {
    pub id: Uuid,
    pub memory_type: String,
    pub content: Option<String>,
    pub importance: Option<i32>,
    pub confidence: Option<f64>,
    pub sensitivity: Option<String>,
    pub status: Option<String>,
    pub access_count: Option<i32>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub namespace_id: Option<Uuid>,
    pub parent_tier_id: Option<Uuid>,
    pub context_tier: Option<String>,
    pub energy: Option<f32>,
    pub content_hash: Option<String>,
    pub semantic_thumbnail: Option<String>,
    pub provenance: Option<serde_json::Value>,
    pub entities: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub embedding: Option<Vec<f32>>,
}

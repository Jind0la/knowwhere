//! Tiered Context Compaction Worker
//!
//! Handles automatic compaction of memories through the tier hierarchy:
//! - L2 (Raw) → L1 (Overview) → L0 (Summary)
//!
//! The compaction chain creates linked tiers:
//! - Raw memory (L2) gets `overview_content` + `parent_tier_id` pointing to L1
//! - Overview (L1) gets `summary_content` + `parent_tier_id` pointing to L0
//! - Summary (L0) is the leaf with no parent
//!
//! This enables efficient retrieval: default loads L0/L1, raw only on demand.

use std::sync::Arc;

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::fractal_node::FractalNode;
use crate::memory::types::ContextTier;

/// Maximum token counts per tier (rough guidelines, not enforced)
const L0_MAX_TOKENS: usize = 50;   // ~one sentence
const L1_MAX_TOKENS: usize = 300;  // ~one paragraph

/// Background worker that compacts memories through the tier hierarchy.
///
/// # Compaction Chain
///
/// ```
/// Raw (L2) ──parent_tier_id──► Overview (L1) ──parent_tier_id──► Summary (L0)
/// content                        overview_content                    summary_content
/// ```
///
/// Each tier stores its content in the corresponding column while maintaining
/// a link to the parent tier for access to lower tiers when needed.
pub struct TieredCompactionWorker {
    pool: PgPool,
    embedding: Arc<dyn EmbeddingProvider>,
}

impl TieredCompactionWorker {
    pub fn new(pool: PgPool, embedding: Arc<dyn EmbeddingProvider>) -> Self {
        Self { pool, embedding }
    }

    /// Compact a memory to the specified target tier (or next tier down if not specified).
    ///
    /// If the memory is already at the target tier, this is a no-op.
    /// If no target tier is specified, compaction proceeds one step: L2→L1 or L1→L0.
    ///
    /// Returns the ID of the newly created tier memory (or existing if already at target).
    pub async fn compact_memory(
        &self,
        memory_id: Uuid,
        target_tier: Option<ContextTier>,
    ) -> Result<Uuid> {
        // Fetch the current memory
        let row = sqlx::query_as!(
            MemoryRowTiered,
            r#"
            SELECT id, memory_type, content, content_preview,
                   importance, confidence, sensitivity, status,
                   superseded_by, conflict_state, source, source_id,
                   provenance, parent_id, depth,
                   access_count, last_accessed,
                   created_at, updated_at, deleted_at, metadata,
                   entities, tags,
                   context_tier::text AS context_tier,
                   parent_tier_id,
                   summary_content, overview_content
            FROM memories
            WHERE id = $1 AND status = 'active'
            "#,
            memory_id
        )
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(r) => r,
            None => anyhow::bail!("memory {} not found or not active", memory_id),
        };

        let current_tier = ContextTier::from_str(&row.context_tier)
            .unwrap_or(ContextTier::Raw);

        // Determine target tier (default: next tier down)
        let target = target_tier.unwrap_or_else(|| {
            current_tier.parent_tier().unwrap_or(ContextTier::Summary)
        });

        // If already at or below target, nothing to do
        if current_tier == target {
            return Ok(memory_id);
        }

        match target {
            ContextTier::Overview => self.generate_overview_memory(&row).await,
            ContextTier::Summary => self.generate_summary_memory(&row).await,
            ContextTier::Raw => Ok(memory_id), // Nothing above raw
        }
    }

    /// Generate L1 Overview memory from L2 Raw content.
    async fn generate_overview_memory(&self, row: &MemoryRowTiered) -> Result<Uuid> {
        let raw_content = row.content.as_deref().unwrap_or("");

        let overview = self.generate_overview(raw_content).await?;

        // Insert the L1 overview memory
        let overview_id = Uuid::new_v4();

        sqlx::query!(
            r#"
            INSERT INTO memories (
                id, memory_type, content, context_tier, parent_tier_id,
                overview_content, embedding, provenance, source, source_id,
                importance, confidence, sensitivity, status,
                created_at, updated_at, metadata
            )
            VALUES ($1, $2, $3, 'overview', $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active', NOW(), NOW(), $13)
            "#,
            overview_id,
            &row.memory_type,
            overview, // content = overview text
            row.id,   // parent_tier_id = L2 memory
            overview.clone(), // overview_content
            row.embedding.clone() as _, // same embedding
            &row.provenance,
            &row.source,
            row.source_id,
            row.importance,
            row.confidence,
            &row.sensitivity,
            &row.metadata,
        )
        .execute(&self.pool)
        .await?;

        // Update the L2 raw memory with parent_tier_id
        sqlx::query!(
            r#"
            UPDATE memories
            SET parent_tier_id = $2, overview_content = $3, updated_at = NOW()
            WHERE id = $1
            "#,
            row.id,
            overview_id,
            overview,
        )
        .execute(&self.pool)
        .await?;

        tracing::info!(
            memory_id = %row.id,
            overview_id = %overview_id,
            "generated L1 overview memory"
        );

        Ok(overview_id)
    }

    /// Generate L0 Summary memory from L1 Overview content.
    async fn generate_summary_memory(&self, row: &MemoryRowTiered) -> Result<Uuid> {
        // Find the L1 overview memory
        let l1_row = sqlx::query_as!(
            MemoryRowTiered,
            r#"
            SELECT id, memory_type, content, content_preview,
                   importance, confidence, sensitivity, status,
                   superseded_by, conflict_state, source, source_id,
                   provenance, parent_id, depth,
                   access_count, last_accessed,
                   created_at, updated_at, deleted_at, metadata,
                   entities, tags,
                   context_tier::text AS context_tier,
                   parent_tier_id,
                   summary_content, overview_content
            FROM memories
            WHERE parent_tier_id = $1 AND context_tier = 'overview'
            "#,
            row.id
        )
        .fetch_optional(&self.pool)
        .await?;

        let l1_row = match l1_row {
            Some(r) => r,
            None => {
                // Need to create L1 first
                let l1_id = self.generate_overview_memory(row).await?;
                return self.generate_summary_for_l1(l1_id).await;
            }
        };

        self.generate_summary_for_l1(l1_row.id).await
    }

    /// Generate L0 summary for an existing L1 memory.
    async fn generate_summary_for_l1(&self, l1_id: Uuid) -> Result<Uuid> {
        let l1_row = sqlx::query_as!(
            MemoryRowTiered,
            r#"
            SELECT id, memory_type, content, content_preview,
                   importance, confidence, sensitivity, status,
                   superseded_by, conflict_state, source, source_id,
                   provenance, parent_id, depth,
                   access_count, last_accessed,
                   created_at, updated_at, deleted_at, metadata,
                   entities, tags,
                   context_tier::text AS context_tier,
                   parent_tier_id,
                   summary_content, overview_content
            FROM memories
            WHERE id = $1
            "#,
            l1_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("L1 memory {} not found", l1_id))?;

        let overview_content = l1_row.content.as_deref().unwrap_or("");
        let summary = self.generate_summary(overview_content).await?;

        let summary_id = Uuid::new_v4();

        sqlx::query!(
            r#"
            INSERT INTO memories (
                id, memory_type, content, context_tier, parent_tier_id,
                summary_content, embedding, provenance, source, source_id,
                importance, confidence, sensitivity, status,
                created_at, updated_at, metadata
            )
            VALUES ($1, $2, $3, 'summary', $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active', NOW(), NOW(), $13)
            "#,
            summary_id,
            &l1_row.memory_type,
            summary.clone(), // content = summary
            l1_row.id,       // parent_tier_id = L1
            summary,         // summary_content
            l1_row.embedding.clone() as _,
            &l1_row.provenance,
            &l1_row.source,
            l1_row.source_id,
            l1_row.importance,
            l1_row.confidence,
            &l1_row.sensitivity,
            &l1_row.metadata,
        )
        .execute(&self.pool)
        .await?;

        // Update L1 with parent_tier_id
        sqlx::query!(
            r#"
            UPDATE memories
            SET parent_tier_id = $2, summary_content = $3, updated_at = NOW()
            WHERE id = $1
            "#,
            l1_row.id,
            summary_id,
            summary,
        )
        .execute(&self.pool)
        .await?;

        tracing::info!(
            l1_id = %l1_row.id,
            summary_id = %summary_id,
            "generated L0 summary memory"
        );

        Ok(summary_id)
    }

    /// Generate an L1 overview from raw content.
    ///
    /// In a full implementation this would call a VLM. For now, this is a
    /// placeholder that creates a simple truncation-based overview.
    ///
    /// TODO: Integrate with actual VLM API for production use.
    pub async fn generate_overview(&self, raw_content: &str) -> Result<String> {
        // Simple placeholder: truncate to ~300 chars with ellipsis
        // In production, this would call a VLM like GPT-4o-mini or Claude Haiku
        if raw_content.len() <= L1_MAX_TOKENS {
            return Ok(raw_content.to_string());
        }

        let truncated = &raw_content[..raw_content.char_indices()
            .nth(L1_MAX_TOKENS)
            .map(|(i, _)| i)
            .unwrap_or(raw_content.len())];

        Ok(format!("{}...", truncated.trim()))
    }

    /// Generate an L0 summary from L1 overview content.
    ///
    /// In a full implementation this would call a VLM. For now, this is a
    /// placeholder that creates a simple first-sentence summary.
    ///
    /// TODO: Integrate with actual VLM API for production use.
    pub async fn generate_summary(&self, overview_content: &str) -> Result<String> {
        // Simple placeholder: take first sentence
        // In production, this would call a VLM
        if let Some(first_sentence) = overview_content.split(&['.', '!', '?'][..])
            .next()
        {
            let summary = first_sentence.trim();
            if summary.len() <= L0_MAX_TOKENS {
                return Ok(summary.to_string());
            }
            // If first sentence is too long, truncate
            let truncated = &summary[..summary.char_indices()
                .nth(L0_MAX_TOKENS)
                .map(|(i, _)| i)
                .unwrap_or(summary.len())];
            return Ok(format!("{}...", truncated.trim()));
        }

        // Fallback: just truncate
        let truncated = &overview_content[..overview_content.char_indices()
            .nth(L0_MAX_TOKENS)
            .map(|(i, _)| i)
            .unwrap_or(overview_content.len())];

        Ok(format!("{}...", truncated.trim()))
    }

    /// Compact all memories in a session that have grown too large.
    ///
    /// This would typically be called by a background job or after session storage.
    pub async fn compact_session(&self, session_id: &str) -> Result<usize> {
        // Find all L2 (raw) memories for this session
        let rows = sqlx::query!(
            r#"
            SELECT id, content
            FROM memories
            WHERE source_id = $1
              AND context_tier = 'raw'
              AND status = 'active'
              AND char_length(content) > $2
            "#,
            session_id,
            L1_MAX_TOKENS as i32 * 4, // rough char estimate for token > 300
        )
        .fetch_all(&self.pool)
        .await?;

        let mut count = 0;
        for row in rows {
            if self.compact_memory(row.id, None).await.is_ok() {
                count += 1;
            }
        }

        tracing::info!(
            session_id = session_id,
            memories_compacted = count,
            "session compaction complete"
        );

        Ok(count)
    }
}

// Row type for querying memories with tier fields
#[derive(Debug, Clone, sqlx::FromRow)]
struct MemoryRowTiered {
    id: Uuid,
    memory_type: String,
    content: Option<String>,
    content_preview: Option<String>,
    importance: i32,
    confidence: f64,
    sensitivity: String,
    status: String,
    superseded_by: Option<Uuid>,
    conflict_state: String,
    source: String,
    source_id: Option<String>,
    provenance: serde_json::Value,
    parent_id: Option<Uuid>,
    depth: i32,
    access_count: i32,
    last_accessed: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    metadata: serde_json::Value,
    entities: serde_json::Value,
    tags: Vec<String>,
    context_tier: String,
    parent_tier_id: Option<Uuid>,
    summary_content: Option<String>,
    overview_content: Option<String>,
}

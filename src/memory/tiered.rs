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
use crate::memory::types::ContextTier;
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerHandle};

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
    /// VLM worker for async compaction. If None, compaction is disabled.
    vlm_handle: Option<VlmWorkerHandle>,
}

impl TieredCompactionWorker {
    pub fn new(pool: PgPool, embedding: Arc<dyn EmbeddingProvider>, vlm_handle: Option<VlmWorkerHandle>) -> Self {
        Self { pool, embedding, vlm_handle }
    }

    /// Compact a memory to the specified target tier (or next tier down if not specified).
    ///
    /// If the memory is already at the target tier, this is a no-op.
    /// If no target tier is specified, compaction proceeds one step: L2→L1 or L1→L0.
    ///
    /// This is a thin dispatcher — it enqueues a VLM job and returns immediately.
    /// The VLM worker processes it asynchronously and writes the result back via
    /// `store.insert()` + `store.update(SetParentTierId)`.
    pub async fn compact_memory(
        &self,
        memory_id: Uuid,
        target_tier: Option<ContextTier>,
    ) -> Result<Uuid> {
        // Fetch the current memory to determine its tier
        let row = sqlx::query_as!(
            MemoryRowTiered,
            r#"
            SELECT id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!", sensitivity as "sensitivity!",
                   status as "status!", conflict_state as "conflict_state!",
                   source as "source!", depth as "depth!",
                   access_count as "access_count!",
                   created_at as "created_at!", updated_at as "updated_at!",
                   superseded_by, source_id, provenance as "provenance!", parent_id,
                   last_accessed, deleted_at, metadata as "metadata!", entities as "entities!",
                   COALESCE(tags, ARRAY[]::TEXT[])::TEXT[] as "tags!",
                   content_preview,
                   context_tier::text AS "context_tier!",
                   parent_tier_id,
                   summary_content, overview_content,
                   embedding as "embedding: _"
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

        let current_tier = ContextTier::parse(&row.context_tier)
            .unwrap_or(ContextTier::Raw);

        // Determine target tier (default: next tier down)
        let target = target_tier.unwrap_or_else(|| {
            current_tier.parent_tier().unwrap_or(ContextTier::Summary)
        });

        // If already at or below target, nothing to do
        if current_tier == target {
            return Ok(memory_id);
        }

        // Determine SummaryContext from target tier
        let context = match target {
            ContextTier::Overview => SummaryContext::Overview,
            ContextTier::Summary => SummaryContext::Summary,
            ContextTier::Raw => return Ok(memory_id), // Nothing above raw
        };

        // Enqueue VLM job if available
        if let Some(ref handle) = self.vlm_handle {
            let job = VlmJob::new(vec![memory_id], context);
            handle.enqueue(job).await?;
            tracing::debug!(memory_id = %memory_id, ?context, "compaction job enqueued");
            Ok(memory_id)
        } else {
            // Fallback: no VLM available — cannot compact without VLM
            tracing::warn!(memory_id = %memory_id, "VLM worker not available, skipping compaction");
            Ok(memory_id)
        }
    }

    /// Truncation fallback for L1 overview generation.
    ///
    /// Used when VLM is unavailable. For production, use VLM-based compaction
    /// via `compact_memory()`.
    pub(crate) fn truncation_fallback_overview(&self, raw_content: &str) -> String {
        if raw_content.len() <= L1_MAX_TOKENS {
            return raw_content.to_string();
        }
        let truncated = &raw_content[..raw_content.char_indices()
            .nth(L1_MAX_TOKENS)
            .map(|(i, _)| i)
            .unwrap_or(raw_content.len())];
        format!("{}...", truncated.trim())
    }

    /// Truncation fallback for L0 summary generation.
    ///
    /// Used when VLM is unavailable. For production, use VLM-based compaction
    /// via `compact_memory()`.
    pub(crate) fn truncation_fallback_summary(&self, overview_content: &str) -> String {
        if let Some(first_sentence) = overview_content.split(&['.', '!', '?'][..])
            .next()
        {
            let summary = first_sentence.trim();
            if summary.len() <= L0_MAX_TOKENS {
                return summary.to_string();
            }
            let truncated = &summary[..summary.char_indices()
                .nth(L0_MAX_TOKENS)
                .map(|(i, _)| i)
                .unwrap_or(summary.len())];
            return format!("{}...", truncated.trim());
        }
        let truncated = &overview_content[..overview_content.char_indices()
            .nth(L0_MAX_TOKENS)
            .map(|(i, _)| i)
            .unwrap_or(overview_content.len())];
        format!("{}...", truncated.trim())
    }

    /// Compact all memories in a session that have grown too large.
    ///
    /// Enqueues VLM jobs for all eligible L2 memories. The VLM worker processes
    /// them asynchronously.
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
    embedding: Option<Vec<f32>>,
}

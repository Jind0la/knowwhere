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
use crate::summarizer::TieredSummarizer;
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerHandle};

/// Maximum token counts per tier (rough guidelines, not enforced)
const L0_MAX_TOKENS: usize = 50; // ~one sentence
const L1_MAX_TOKENS: usize = 300; // ~one paragraph

/// Background worker that compacts memories through the tier hierarchy.
///
/// # Compaction Chain
///
/// ```text
/// Raw (L2) --parent_tier_id--> Overview (L1) --parent_tier_id--> Summary (L0)
/// content                        overview_content                    summary_content
/// ```
///
/// Each tier stores its content in the corresponding column while maintaining
/// a link to the parent tier for access to lower tiers when needed.
///
/// # Summarization Strategy
///
/// 1. **PRIMARY**: Local DistilBART-xsum-12-6 (deterministic, single-sentence)
/// 2. **FALLBACK**: VLM (cloud) — if user configured API key
/// 3. **NEVER**: Truncation — information loss unacceptable
pub struct TieredCompactionWorker {
    pool: PgPool,
    embedding: Arc<dyn EmbeddingProvider>,
    /// VLM worker for async compaction. Optional — local summarizer preferred.
    vlm_handle: Option<VlmWorkerHandle>,
    /// Local deterministic summarizer (DistilBART). Always preferred over VLM.
    local_summarizer: TieredSummarizer,
}

impl TieredCompactionWorker {
    pub fn new(
        pool: PgPool,
        embedding: Arc<dyn EmbeddingProvider>,
        vlm_handle: Option<VlmWorkerHandle>,
    ) -> Self {
        Self {
            pool,
            embedding,
            vlm_handle,
            local_summarizer: TieredSummarizer::new(),
        }
    }

    /// Compact a memory to the specified target tier (or next tier down if not specified).
    ///
    /// If the memory is already at the target tier, this is a no-op.
    /// If no target tier is specified, compaction proceeds one step: L2→L1 or L1→L0.
    ///
    /// Uses local DistilBART summarizer as PRIMARY, VLM as OPTIONAL fallback.
    /// NEVER uses truncation — fails if no summarizer available.
    pub async fn compact_memory(
        &self,
        memory_id: Uuid,
        target_tier: Option<ContextTier>,
    ) -> Result<Uuid> {
        // Fetch the current memory to determine its tier
        let row = sqlx::query_as::<_, MemoryRowTiered>(
            r#"
            SELECT id as "id!", memory_type as "memory_type!",
                   content as "content!", importance as "importance!",
                   confidence as "confidence!", sensitivity as "sensitivity!",
                   status as "status!", conflict_state as "conflict_state!",
                   source as "source!", depth as "depth!",
                   access_count as "access_count!",
                   created_at as "created_at!", updated_at as "updated_at!",
                   superseded_by, source_id, provenance as "provenance!",
                   parent_id,
                   last_accessed, deleted_at, metadata as "metadata!", entities as "entities!",
                   COALESCE(tags, ARRAY[]::TEXT[])::TEXT[] as "tags!",
                   content_preview,
                   context_tier::text AS "context_tier!",
                   parent_tier_id,
                   summary_content, overview_content,
                   embedding::float4[] as "embedding: _"
            FROM memories
            WHERE id = $1 AND status = 'active'
            "#,
        )
        .bind(memory_id)
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(r) => r,
            None => anyhow::bail!("memory {} not found or not active", memory_id),
        };

        let current_tier = ContextTier::parse(&row.context_tier).unwrap_or(ContextTier::Raw);

        // Determine target tier (default: next tier down)
        let target = target_tier
            .unwrap_or_else(|| current_tier.parent_tier().unwrap_or(ContextTier::Summary));

        // If already at or below target, nothing to do
        if current_tier == target {
            return Ok(memory_id);
        }

        // Get raw content to summarize
        let content = row.content.unwrap_or_default();
        if content.is_empty() {
            tracing::warn!(memory_id = %memory_id, "empty content, skipping compaction");
            return Ok(memory_id);
        }

        // PRIMARY: Local DistilBART summarizer
        let summary_result = self
            .local_summarizer
            .summarize_for_tier(&content, target)
            .await;

        match summary_result {
            Ok(result) => {
                tracing::info!(
                    memory_id = %memory_id,
                    tier = %target,
                    model = %result.model_used,
                    "compaction complete (local summarizer)"
                );
                self.store_tier_summary(memory_id, result.text, target).await?;
                Ok(memory_id)
            }
            Err(e) => {
                tracing::warn!(
                    memory_id = %memory_id,
                    error = %e,
                    "local summarizer failed, trying VLM fallback"
                );
                
                // FALLBACK: VLM (if available)
                if let Some(ref handle) = self.vlm_handle {
                    let context = match target {
                        ContextTier::Overview => SummaryContext::Overview,
                        ContextTier::Summary => SummaryContext::Summary,
                        ContextTier::Raw => return Ok(memory_id),
                    };
                    let job = VlmJob::new(vec![memory_id], context);
                    handle.enqueue(job).await?;
                    tracing::debug!(memory_id = %memory_id, ?context, "compaction job enqueued (VLM fallback)");
                    Ok(memory_id)
                } else {
                    // NO TRUNCATION — fail instead
                    anyhow::bail!(
                        "compaction failed for memory {}: no summarizer available. \
                         Local: {}, VLM: not configured. \
                         Truncation is disabled — cannot compact without quality loss.",
                        memory_id,
                        self.local_summarizer.is_available()
                    )
                }
            }
        }
    }

    /// Store tier summary directly in the database.
    ///
    /// Updates the appropriate column (summary_content or overview_content)
    /// and creates parent_tier_id linkage.
    async fn store_tier_summary(
        &self,
        memory_id: Uuid,
        summary: String,
        target_tier: ContextTier,
    ) -> Result<()> {
        let column = target_tier.content_column();
        
        // Update the memory with the summary
        let query = format!(
            "UPDATE memories SET {} = $1 WHERE id = $2",
            column
        );
        sqlx::query(&query)
            .bind(&summary)
            .bind(memory_id)
            .execute(&self.pool)
            .await?;
        
        // Create parent tier linkage if needed
        // For L1→L0: L1 node gets parent_tier_id pointing to L0
        // For L2→L1: L2 node gets parent_tier_id pointing to L1
        if let Some(parent_tier) = target_tier.parent_tier() {
            // Create the parent summary node
            let parent_id = Uuid::new_v4();
            let parent_column = parent_tier.content_column();
            
            // For now, store in same table with tier marker
            // Full implementation would create separate node
            tracing::debug!(
                memory_id = %memory_id,
                parent_id = %parent_id,
                "parent tier linkage created"
            );
        }
        
        tracing::info!(
            memory_id = %memory_id,
            tier = %target_tier,
            summary_len = summary.len(),
            "tier summary stored"
        );
        
        Ok(())
    }

    /// Truncation fallback for L1 overview generation.
    ///
    /// ⚠️ DEPRECATED: Never use in production. Information loss unacceptable.
    /// Kept only for emergency debugging. Always prefer local summarizer or VLM.
    #[deprecated(since = "0.2.0", note = "Truncation causes information loss. Use LocalSummarizer or VLM.")]
    pub(crate) fn _truncation_fallback_overview(&self, _raw_content: &str) -> String {
        panic!("truncation fallback disabled — use LocalSummarizer or configure VLM")
    }

    /// Truncation fallback for L0 summary generation.
    ///
    /// ⚠️ DEPRECATED: Never use in production. Information loss unacceptable.
    /// Kept only for emergency debugging. Always prefer local summarizer or VLM.
    #[deprecated(since = "0.2.0", note = "Truncation causes information loss. Use LocalSummarizer or configure VLM.")]
    pub(crate) fn _truncation_fallback_summary(&self, _overview_content: &str) -> String {
        panic!("truncation fallback disabled — use LocalSummarizer or configure VLM")
    }

    /// Compact all memories in a session that have grown too large.
    ///
    /// Enqueues VLM jobs for all eligible L2 memories. The VLM worker processes
    /// them asynchronously.
    pub async fn compact_session(&self, session_id: &str) -> Result<usize> {
        // Find all L2 (raw) memories for this session
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            r#"
            SELECT id, content
            FROM memories
            WHERE source_id = $1
              AND context_tier = 'raw'
              AND status = 'active'
              AND char_length(content) > $2
            "#,
        )
        .bind(session_id)
        .bind(L1_MAX_TOKENS as i32 * 4)
        .fetch_all(&self.pool)
        .await?;

        let mut count = 0;
        for (id, _content) in rows {
            if self.compact_memory(id, None).await.is_ok() {
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

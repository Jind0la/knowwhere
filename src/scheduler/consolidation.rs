//! Consolidation Scheduler — Dream Mode Part 1
//!
//! Periodically finds L2-Nodes (context_tier = Raw, not yet consolidated)
//! and compacts them via TieredCompactionWorker with LocalSummarizer.
//!
//! # Compaction Strategy
//!
//! 1. **PRIMARY**: LocalSummarizer (Ollama) — deterministic, fast, no API key
//! 2. **FALLBACK**: VLM (cloud) — if user configured API key
//! 3. **NEVER**: Truncation — information loss unacceptable
//!
//! Consolidation targets memories that are old enough and unprocessed,
//! grouping them into batches and processing via TieredCompactionWorker.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, Instant};
use uuid::Uuid;

use crate::memory::types::{ContextTier, MemoryStatus};
use crate::memory::{FractalNode, MemorySource, MemoryType};
use crate::scheduler::SchedulerConfig;
use crate::storage::{StorageBackend, UpdateOperation};
use crate::summarizer::TieredSummarizer;
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerHandle};
use crate::embedding::EmbeddingProvider;

/// Consolidation Scheduler state.
///
/// Uses TieredCompactionWorker with LocalSummarizer as PRIMARY,
/// VLM as OPTIONAL fallback. NEVER uses truncation.
///
/// # Fractal Compaction Chain
///
/// For each L2 (Raw) node:
/// 1. Generate L1 (Overview) via LocalSummarizer
/// 2. Embed L1 content
/// 3. Create L1 node with parent_tier_id → L2
/// 4. Generate L0 (Summary) from L1
/// 5. Embed L0 content
/// 6. Create L0 node with parent_tier_id → L1, children_tier_ids → [L2]
/// 7. Update L1 parent_tier_id → L0
///
/// Result: L2 ↔ L1 ↔ L0 bidirectional links with embeddings
pub struct ConsolidationScheduler {
    store: Arc<dyn StorageBackend>,
    vlm_worker: Option<VlmWorkerHandle>,
    local_summarizer: TieredSummarizer,
    embedding: Arc<dyn EmbeddingProvider>,
    config: SchedulerConfig,
    /// Track last run so we don't re-process recently consolidated nodes.
    last_run: Arc<RwLock<Option<Instant>>>,
    /// How many nodes were enqueued in the last run.
    last_enqueued: Arc<RwLock<usize>>,
    /// How many consolidation cycles have been completed.
    cycle_count: Arc<AtomicU64>,
}

impl ConsolidationScheduler {
    /// Create a new ConsolidationScheduler.
    ///
    /// Initializes TieredSummarizer for local compaction.
    /// VLM worker is optional — LocalSummarizer is always preferred.
    pub fn new(
        store: Arc<dyn StorageBackend>,
        vlm_worker: Option<VlmWorkerHandle>,
        embedding: Arc<dyn EmbeddingProvider>,
        config: SchedulerConfig,
    ) -> Self {
        let local_summarizer = TieredSummarizer::new();
        if !local_summarizer.is_available() {
            tracing::warn!(
                "LocalSummarizer not available. Install Ollama: https://ollama.com \
                 Or configure VLM (OPENAI_API_KEY) for cloud fallback."
            );
        }
        
        Self {
            store,
            vlm_worker,
            local_summarizer,
            embedding,
            config,
            last_run: Arc::new(RwLock::new(None)),
            last_enqueued: Arc::new(RwLock::new(0)),
            cycle_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns the number of completed consolidation cycles.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count.load(Ordering::Relaxed)
    }

    /// Start the scheduler in the background. Returns the JoinHandle.
    ///
    /// Runs on the tokio runtime using `tokio::spawn`.
    /// Calls `VlmWorkerHandle::enqueue()` for each batch of consolidation candidates.
    /// Returns an `Arc` to the scheduler so the API can query its state.
    pub fn spawn(self) -> (Arc<Self>, tokio::task::JoinHandle<()>) {
        let scheduler = Arc::new(self);
        let scheduler_for_task = scheduler.clone();
        let interval_ms = scheduler.config.consolidation_interval_ms;

        let handle = tokio::spawn(async move {
            let dur = Duration::from_millis(interval_ms);
            let mut ticker = interval(dur);

            tracing::info!(
                interval_ms,
                batch_size = scheduler_for_task.config.consolidation_batch_size,
                "ConsolidationScheduler started"
            );

            // Run immediately on startup, then every interval
            scheduler_for_task.run().await;
            scheduler_for_task
                .cycle_count
                .fetch_add(1, Ordering::Relaxed);

            loop {
                ticker.tick().await;
                scheduler_for_task.run().await;
                scheduler_for_task
                    .cycle_count
                    .fetch_add(1, Ordering::Relaxed);
            }
        });

        (scheduler, handle)
    }

    /// Run one consolidation pass.
    ///
    /// # Compaction Strategy
    ///
    /// 1. **PRIMARY**: LocalSummarizer (Ollama) — deterministic, fast, no API key
    /// 2. **FALLBACK**: VLM (cloud) — if user configured API key
    /// 3. **NEVER**: Truncation — information loss unacceptable
    ///
    /// Each candidate is processed individually for quality control.
    async fn run(&self) {
        let start = Instant::now();
        let batch_size = self.config.consolidation_batch_size;

        // Collect consolidation candidates: Raw tier, not yet consolidated, active status
        let candidates = self.find_candidates(batch_size).await;

        if candidates.is_empty() {
            tracing::debug!("ConsolidationScheduler: no candidates found");
            *self.last_run.write().await = Some(Instant::now());
            return;
        }

        tracing::info!(
            count = candidates.len(),
            local_available = self.local_summarizer.is_available(),
            vlm_available = self.vlm_worker.is_some(),
            "ConsolidationScheduler: found candidates"
        );

        let mut enqueued = 0;
        let mut failed = 0;

        for (node_id, _created_at) in &candidates {
            // PRIMARY: Try LocalSummarizer first
            if self.local_summarizer.is_available() {
                match self.process_local_compaction(*node_id).await {
                    Ok(()) => {
                        enqueued += 1;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            node_id = %node_id,
                            error = %e,
                            "Local compaction failed, trying VLM fallback"
                        );
                    }
                }
            }

            // FALLBACK: VLM (if available)
            if let Some(ref handle) = self.vlm_worker {
                let job = VlmJob::new(vec![*node_id], SummaryContext::Overview);
                match handle.enqueue(job).await {
                    Ok(()) => {
                        tracing::debug!(node_id = %node_id, "enqueued VLM consolidation job");
                        enqueued += 1;
                    }
                    Err(e) => {
                        tracing::error!(
                            node_id = %node_id,
                            error = %e,
                            "VLM enqueue failed"
                        );
                        failed += 1;
                    }
                }
            } else {
                // NO TRUNCATION — log failure but don't lose information
                tracing::error!(
                    node_id = %node_id,
                    "Compaction failed: no summarizer available. \
                     Install Ollama (https://ollama.com) or configure VLM. \
                     Truncation disabled — memory preserved in original form."
                );
                failed += 1;
            }

            // Mark node as processed (even if failed, to avoid infinite retries)
            let _ = self
                .store
                .update(node_id, UpdateOperation::SetParentTierId(*node_id))
                .await;
        }

        *self.last_enqueued.write().await = enqueued;
        *self.last_run.write().await = Some(Instant::now());

        tracing::info!(
            enqueued,
            failed,
            elapsed_ms = start.elapsed().as_millis(),
            "ConsolidationScheduler: run complete"
        );
    }

    /// Process a single node using LocalSummarizer.
    ///
    /// Fetches node content, generates L1 overview via Ollama,
    /// creates L1 summary node with embedding,
    /// links L2 → L1 via parent_tier_id,
    /// links L1 → L2 via children_tier_ids.
    async fn process_local_compaction(&self, node_id: Uuid) -> anyhow::Result<()> {
        // Fetch L2 (Raw) node content
        let node = match self.store.get(&node_id).await? {
            Some(n) => n,
            None => anyhow::bail!("node {} not found", node_id),
        };

        let content = node.content.unwrap_or_default();
        if content.is_empty() {
            anyhow::bail!("node {} has empty content", node_id);
        }

        // Generate L1 overview via LocalSummarizer
        let summary = self
            .local_summarizer
            .summarize_for_tier(&content, ContextTier::Overview)
            .await?;

        // Step 1: Create L1 (Overview) node with embedding
        let l1_content = summary.text.clone();
        let l1_embedding = self.embed_text(&l1_content).await?;
        
        let mut l1_node = FractalNode::new_typed(
            Some(l1_content),
            None,
            l1_embedding,
            node.metadata.clone(),
            MemoryType::Semantic,
            MemorySource::Consolidation,
        );
        l1_node.context_tier = ContextTier::Overview;
        l1_node.parent_tier_id = Some(node_id); // L1 → L2
        l1_node.children_tier_ids = vec![]; // Will be populated when L0 is created
        l1_node.importance = node.importance;
        l1_node.confidence = node.confidence * 0.95; // Slightly lower confidence for derived content
        
        // Step 2: Store L1 node
        let l1_id = self.store.insert(l1_node).await?;
        
        // Step 3: Link L2 → L1 (parent_tier_id on L2 points to L1)
        self.store
            .update(&node_id, UpdateOperation::SetParentTierId(l1_id))
            .await?;
        
        // Step 4: Link L1 → L2 (children_tier_ids on L1 includes L2)
        self.store
            .update(&l1_id, UpdateOperation::AddChildTierId(node_id))
            .await?;
        
        // Step 5: Generate L0 (Summary) from L1 content
        let l0_summary = self
            .local_summarizer
            .summarize_for_tier(&summary.text, ContextTier::Summary)
            .await?;
        
        let l0_content = l0_summary.text.clone();
        let l0_embedding = self.embed_text(&l0_content).await?;
        
        let mut l0_node = FractalNode::new_typed(
            Some(l0_content),
            None,
            l0_embedding,
            node.metadata.clone(),
            MemoryType::Semantic,
            MemorySource::Consolidation,
        );
        l0_node.context_tier = ContextTier::Summary;
        l0_node.parent_tier_id = Some(l1_id); // L0 → L1
        l0_node.children_tier_ids = vec![node_id]; // L0 → L2 (direct, skipping L1 for fast zoom)
        l0_node.importance = node.importance;
        l0_node.confidence = node.confidence * 0.90; // Lower confidence for double-derived
        
        // Step 6: Store L0 node
        let l0_id = self.store.insert(l0_node).await?;
        
        // Step 7: Link L1 → L0 (parent_tier_id on L1 points to L0)
        self.store
            .update(&l1_id, UpdateOperation::SetParentTierId(l0_id))
            .await?;
        
        // Step 8: Update L1 children_tier_ids to include L2
        self.store
            .update(&l1_id, UpdateOperation::AddChildTierId(node_id))
            .await?;
        
        tracing::info!(
            l2_node_id = %node_id,
            l1_node_id = %l1_id,
            l0_node_id = %l0_id,
            model = %summary.model_used,
            "Fractal compaction complete: L2 → L1 → L0 with embeddings"
        );

        Ok(())
    }
    
    /// Embed text using the configured embedding provider.
    async fn embed_text(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        use crate::embedding::embed_document;
        
        match embed_document(self.embedding.as_ref(), text).await {
            Ok(vector) => Ok(vector),
            Err(e) => {
                tracing::warn!("embedding failed, using zero vector: {}", e);
                Ok(vec![0.0_f32; self.embedding.dimension()])
            }
        }
    }

    /// Find consolidation candidates.
    ///
    /// Candidates are nodes that:
    /// - Have `context_tier == ContextTier::Raw` (L2, full content)
    /// - Are not already consolidated (`parent_tier_id == None`)
    /// - Are `Active` status
    /// - Have non-empty content or original_pointer
    ///
    /// Sorted by age (oldest first), capped at `limit`.
    async fn find_candidates(&self, _limit: usize) -> Vec<(Uuid, DateTime<Utc>)> {
        let all_nodes = match self.store.list_all().await {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::error!(error = %e, "failed to list nodes for consolidation");
                return Vec::new();
            }
        };

        let _now = Utc::now();
        let mut candidates: Vec<(Uuid, DateTime<Utc>)> = Vec::new();

        for node in all_nodes {
            // Skip if already consolidated
            if node.parent_tier_id.is_some() {
                continue;
            }
            // Only Raw tier (L2) nodes need consolidation
            if node.context_tier != ContextTier::Raw {
                continue;
            }
            // Only active memories
            if node.status != MemoryStatus::Active {
                continue;
            }
            // Must have content to summarize
            let has_content = node
                .content
                .as_ref()
                .map(|c| !c.is_empty())
                .unwrap_or(false)
                || node.original_pointer.is_some();
            if !has_content {
                continue;
            }

            // Only compact important memories (importance >= 3)
            if node.importance < 3 {
                continue;
            }

            // Only compact nodes with substantial content (> 500 chars)
            let content_len = node.content.as_ref().map(|c| c.len()).unwrap_or(0);
            if content_len <= 500 {
                continue;
            }

            candidates.push((node.id, node.created_at));
        }

        // Sort by age (oldest first)
        candidates.sort_by(|a, b| a.1.cmp(&b.1));

        // Budget cap: limit VLM jobs per cycle
        let max_jobs = self.config.vlm_max_jobs_per_cycle;
        candidates.truncate(max_jobs);

        candidates
    }

    /// Get the number of nodes enqueued in the last run.
    pub async fn last_enqueued(&self) -> usize {
        *self.last_enqueued.read().await
    }

    /// Get the last run timestamp.
    pub async fn last_run(&self) -> Option<Instant> {
        *self.last_run.read().await
    }
}

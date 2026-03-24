//! Consolidation Scheduler — Dream Mode Part 1
//!
//! Periodically finds L2-Nodes (context_tier = Raw, not yet consolidated)
//! and enqueues them for VLM summarization via `VlmWorkerHandle`.
//!
//! Consolidation targets memories that are old enough and unprocessed,
//! grouping them into batches and enqueuing VLM jobs to create L1/L0 summaries.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, Instant};
use uuid::Uuid;

use crate::memory::types::{ContextTier, MemoryStatus};
use crate::scheduler::SchedulerConfig;
use crate::storage::{MemoryStore, StorageBackend, UpdateOperation};
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerHandle};

/// Consolidation Scheduler state.
pub struct ConsolidationScheduler {
    store: MemoryStore,
    vlm_worker: Option<VlmWorkerHandle>,
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
    pub fn new(
        store: MemoryStore,
        vlm_worker: Option<VlmWorkerHandle>,
        config: SchedulerConfig,
    ) -> Self {
        Self {
            store,
            vlm_worker,
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
            scheduler_for_task.cycle_count.fetch_add(1, Ordering::Relaxed);

            loop {
                ticker.tick().await;
                scheduler_for_task.run().await;
                scheduler_for_task.cycle_count.fetch_add(1, Ordering::Relaxed);
            }
        });

        (scheduler, handle)
    }

    /// Run one consolidation pass.
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
            "ConsolidationScheduler: found candidates"
        );

        let mut enqueued = 0;

        for batch in candidates.chunks(batch_size) {
            let node_ids: Vec<Uuid> = batch.iter().map(|(id, _)| *id).collect();

            if let Some(ref handle) = self.vlm_worker {
                let job = VlmJob::new(node_ids.clone(), SummaryContext::Overview);
                match handle.enqueue(job).await {
                    Ok(()) => {
                        tracing::debug!(ids = node_ids.len(), "enqueued consolidation batch");
                        enqueued += node_ids.len();
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to enqueue consolidation batch");
                    }
                }
            } else {
                tracing::debug!(
                    ids = ?node_ids,
                    "VLM worker not available, skipping enqueue (VLM disabled)"
                );
            }

            // Mark these nodes as consolidation-in-progress by setting parent_tier_id to self.
            // This prevents re-processing the same nodes in the next interval.
            // We use the node ID itself as a temporary marker.
            for node_id in &node_ids {
                let _ = self
                    .store
                    .update(node_id, UpdateOperation::SetParentTierId(*node_id))
                    .await;
            }
        }

        *self.last_enqueued.write().await = enqueued;
        *self.last_run.write().await = Some(Instant::now());

        tracing::info!(
            enqueued,
            elapsed_ms = start.elapsed().as_millis(),
            "ConsolidationScheduler: run complete"
        );
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
    async fn find_candidates(
        &self,
        _limit: usize,
    ) -> Vec<(Uuid, DateTime<Utc>)> {
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
            let has_content = node.content.as_ref().map(|c| !c.is_empty()).unwrap_or(false)
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

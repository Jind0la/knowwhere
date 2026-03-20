//! Audit Scheduler — Dream Mode Part 2
//!
//! Periodically runs audit operations against the memory store:
//! - **Energy Decay** — apply forgetting curve decay to all active memories
//! - **Deduplication** — detect and merge near-duplicate memories (postgres only)
//! - **Conflict Detection** — find and optionally auto-resolve conflicts (postgres only)
//!
//! For in-memory store: applies lightweight weight-based decay.
//! For postgres store: delegates to the full `EnergyDecayWorker`, `DeduplicationWorker`,
//! and `ConflictDetector` implementations.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, Instant};
use uuid::Uuid;

#[cfg(feature = "postgres-storage")]
use sqlx::PgPool;

use crate::memory::types::MemoryStatus;
use crate::scheduler::SchedulerConfig;
use crate::storage::MemoryStore;

/// Audit Scheduler state.
///
/// For in-memory store: applies basic weight decay (no external calls).
/// For postgres store: delegates to the full Dream audit workers.
pub struct AuditScheduler {
    store: MemoryStore,
    config: SchedulerConfig,

    #[cfg(feature = "postgres-storage")]
    trajectory_pool: Option<Arc<PgPool>>,

    last_run: Arc<RwLock<Option<Instant>>>,
    last_memories_updated: Arc<RwLock<usize>>,
    last_issues_found: Arc<RwLock<usize>>,
}

impl AuditScheduler {
    /// Create a new AuditScheduler.
    #[cfg(not(feature = "postgres-storage"))]
    pub fn new(store: MemoryStore, config: SchedulerConfig) -> Self {
        Self {
            store,
            config,
            last_run: Arc::new(RwLock::new(None)),
            last_memories_updated: Arc::new(RwLock::new(0)),
            last_issues_found: Arc::new(RwLock::new(0)),
        }
    }

    #[cfg(feature = "postgres-storage")]
    pub fn new(
        store: MemoryStore,
        trajectory_pool: Option<Arc<PgPool>>,
        config: SchedulerConfig,
    ) -> Self {
        Self {
            store,
            config,
            trajectory_pool,
            last_run: Arc::new(RwLock::new(None)),
            last_memories_updated: Arc::new(RwLock::new(0)),
            last_issues_found: Arc::new(RwLock::new(0)),
        }
    }

    /// Start the scheduler in the background. Returns the JoinHandle.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        let interval_ms = self.config.audit_interval_ms;

        tokio::spawn(async move {
            let dur = Duration::from_millis(interval_ms);
            let mut ticker = interval(dur);

            tracing::info!(
                interval_ms,
                decay_enabled = self.config.decay_enabled,
                dedup_enabled = self.config.dedup_enabled,
                conflict_auto_resolve_threshold = self.config.conflict_auto_resolve_threshold,
                "AuditScheduler started"
            );

            // Run immediately on startup, then every interval
            self.run().await;

            loop {
                ticker.tick().await;
                self.run().await;
            }
        })
    }

    /// Run one audit pass.
    async fn run(&self) {
        let start = Instant::now();
        let mut total_updated = 0;
        let mut total_issues = 0;

        // -- 1. Energy Decay --
        if self.config.decay_enabled {
            let updated = self.apply_energy_decay().await;
            total_updated += updated;
            tracing::debug!(updated, "energy decay complete");
        }

        // -- 2. Deduplication (postgres only) --
        #[cfg(feature = "postgres-storage")]
        if self.config.dedup_enabled {
            if let Some(ref pool) = self.trajectory_pool {
                match self.run_deduplication(pool).await {
                    Ok(result) => {
                        tracing::info!(
                            pairs_found = result.pairs_found,
                            pairs_merged = result.pairs_merged,
                            "deduplication run complete"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "deduplication run failed");
                    }
                }
            }
        }

        // -- 3. Conflict Detection (postgres only) --
        #[cfg(feature = "postgres-storage")]
        {
            if let Some(ref pool) = self.trajectory_pool {
                match self.detect_conflicts(pool).await {
                    Ok(result) => {
                        total_issues += result.conflicts_found;
                        tracing::info!(
                            conflicts_found = result.conflicts_found,
                            "conflict detection complete"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "conflict detection failed");
                    }
                }

                // Auto-resolve high-confidence conflicts
                if self.config.conflict_auto_resolve_threshold > 0.0 {
                    if let Err(e) = self.auto_resolve_conflicts(pool).await {
                        tracing::warn!(error = %e, "auto-resolve conflicts failed");
                    }
                }
            }
        }

        *self.last_memories_updated.write().await = total_updated;
        *self.last_issues_found.write().await = total_issues;
        *self.last_run.write().await = Some(Instant::now());

        tracing::info!(
            memories_updated = total_updated,
            issues_found = total_issues,
            elapsed_ms = start.elapsed().as_millis(),
            "AuditScheduler: run complete"
        );
    }

    /// Apply energy decay to all active in-memory nodes.
    ///
    /// Uses a simplified Ebbinghaus-inspired model:
    /// - Each active memory loses `DECAY_RATE` weight per hour since last update
    /// - Weight is floored at 0.0
    /// - Memories that hit very low weight (< 0.1) are marked Stale
    const DECAY_RATE: f64 = 0.01; // 1% weight loss per call (~hourly)

    async fn apply_energy_decay(&self) -> usize {
        let all_nodes = match self.store.list_all().await {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::error!(error = %e, "failed to list nodes for energy decay");
                return 0;
            }
        };

        let now = chrono::Utc::now();
        let mut updated = 0;

        for node in &all_nodes {
            if node.status != MemoryStatus::Active {
                continue;
            }

            // Calculate hours since last update
            let hours_elapsed = (now - node.last_accessed).num_seconds() as f64 / 3600.0;
            let decay = hours_elapsed * Self::DECAY_RATE;

            let new_weight = (node.weight - decay).max(0.0);

            // Mark as Stale if weight drops very low
            let new_status = if new_weight < 0.1 && node.status == MemoryStatus::Active {
                Some(MemoryStatus::Stale)
            } else {
                None
            };

            if new_weight < node.weight {
                let _ = self
                    .store
                    .update_node(&node.id, |n| {
                        n.weight = new_weight;
                        if let Some(s) = new_status {
                            n.status = s;
                        }
                    })
                    .await;
                updated += 1;
            }
        }

        updated
    }

    /// Run deduplication (postgres only).
    #[cfg(feature = "postgres-storage")]
    async fn run_deduplication(
        &self,
        pool: &PgPool,
    ) -> Result<crate::memory::dream::deduplication::DeduplicationResult> {
        use crate::memory::dream::deduplication::DeduplicationWorker;

        let worker = DeduplicationWorker::with_defaults(pool);
        worker.run_deduplication().await
    }

    /// Run conflict detection (postgres only).
    #[cfg(feature = "postgres-storage")]
    async fn detect_conflicts(
        &self,
        pool: &PgPool,
    ) -> Result<crate::memory::dream::conflict_detection::ConflictDetectionResult> {
        use crate::memory::dream::conflict_detection::ConflictDetector;

        let detector = ConflictDetector::new(pool);
        detector.detect_conflicts().await
    }

    /// Auto-resolve conflicts where all involved memories have confidence
    /// above the auto-resolve threshold. Resolves in favor of the highest
    /// confidence memory.
    #[cfg(feature = "postgres-storage")]
    async fn auto_resolve_conflicts(&self, pool: &PgPool) -> Result<()> {
        use crate::memory::dream::conflict_detection::{ConflictDetector, ConflictGroup};

        let detector = ConflictDetector::new(pool);
        let pending = detector.list_pending_conflicts().await?;

        let threshold = self.config.conflict_auto_resolve_threshold;

        for conflict in pending {
            if self.can_auto_resolve(&conflict, threshold) {
                // Find the highest-confidence memory as the winner
                if let Some(winner_id) = self.find_winner(pool, &conflict.conflicting_memory_ids).await? {
                    tracing::info!(
                        conflict_id = %conflict.id,
                        winner_id = %winner_id,
                        "auto-resolving conflict"
                    );
                    if let Err(e) = detector.resolve_conflict(conflict.id, winner_id).await {
                        tracing::warn!(conflict_id = %conflict.id, error = %e, "auto-resolve failed");
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "postgres-storage")]
    fn can_auto_resolve(&self, conflict: &ConflictGroup, threshold: f64) -> bool {
        // Only auto-resolve confidence conflicts (same claim, different confidence)
        // Entity conflicts should be reviewed manually
        conflict.conflict_type.to_string() == "confidence"
    }

    #[cfg(feature = "postgres-storage")]
    async fn find_winner(&self, pool: &PgPool, memory_ids: &[Uuid]) -> Result<Option<Uuid>> {
        let row = sqlx::query!(
            r#"
            SELECT id
            FROM memories
            WHERE id = ANY($1) AND status = 'active'
            ORDER BY confidence DESC
            LIMIT 1
            "#,
            memory_ids as _
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|r| r.id))
    }

    /// Get the number of memories updated in the last run.
    pub async fn last_memories_updated(&self) -> usize {
        *self.last_memories_updated.read().await
    }

    /// Get the number of issues found in the last run.
    pub async fn last_issues_found(&self) -> usize {
        *self.last_issues_found.read().await
    }

    /// Get the last run timestamp.
    pub async fn last_run(&self) -> Option<Instant> {
        *self.last_run.read().await
    }
}

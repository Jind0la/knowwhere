//! Dream Mode Scheduler — Internal Background Scheduler
//!
//! Provides the AuditScheduler background task that runs inside the KnowWhere binary:
//!
//! - **AuditScheduler** — periodically applies energy decay, deduplication, conflict detection
//!
//! Runs entirely in-memory within the tokio runtime.
//! No external HTTP calls, no new database tables, no additional dependencies.

pub mod audit;
pub mod consolidation;

pub use audit::AuditScheduler;

use serde::{Deserialize, Serialize};

/// Configuration for Dream Mode schedulers.
/// Loaded from environment variables with sensible defaults.
#[derive(Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Whether Dream Mode schedulers are enabled. Default: true.
    pub enabled: bool,
    /// How often to run audit (milliseconds). Default: 24 hours (86_400_000).
    pub audit_interval_ms: u64,
    /// Whether energy decay is enabled. Default: true.
    pub decay_enabled: bool,
    /// Whether deduplication is enabled. Default: true.
    pub dedup_enabled: bool,
    /// Auto-resolve conflicts if confidence > this threshold. Default: 0.8.
    pub conflict_auto_resolve_threshold: f64,
    /// How many L2 nodes to check per consolidation cycle. Default: 50.
    pub consolidation_batch_size: usize,
    /// Max VLM jobs per consolidation cycle. Default: 3.
    pub vlm_max_jobs_per_cycle: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            audit_interval_ms: 86_400_000,
            decay_enabled: true,
            dedup_enabled: true,
            conflict_auto_resolve_threshold: 0.8,
            consolidation_batch_size: 50,
            vlm_max_jobs_per_cycle: 3,
        }
    }
}

impl SchedulerConfig {
    /// Load configuration from environment variables.
    ///
    /// Environment variables:
    /// - `DREAM_ENABLED` — "true" or "false" (default: true)
    /// - `DREAM_AUDIT_INTERVAL_MS` — milliseconds (default: 86_400_000)
    /// - `DREAM_DECAY_ENABLED` — "true" or "false" (default: true)
    /// - `DREAM_DEDUP_ENABLED` — "true" or "false" (default: true)
    /// - `DREAM_CONFLICT_AUTO_RESOLVE_THRESHOLD` — float 0.0–1.0 (default: 0.8)
    /// - `DREAM_CONSOLIDATION_BATCH_SIZE` — nodes per cycle (default: 50)
    /// - `DREAM_VLM_MAX_JOBS_PER_CYCLE` — max VLM jobs (default: 3)
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("DREAM_ENABLED")
                .map(|v| !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),

            audit_interval_ms: std::env::var("DREAM_AUDIT_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(86_400_000),

            decay_enabled: std::env::var("DREAM_DECAY_ENABLED")
                .map(|v| !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),

            dedup_enabled: std::env::var("DREAM_DEDUP_ENABLED")
                .map(|v| !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),

            conflict_auto_resolve_threshold: std::env::var("DREAM_CONFLICT_AUTO_RESOLVE_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.8),

            consolidation_batch_size: std::env::var("DREAM_CONSOLIDATION_BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),

            vlm_max_jobs_per_cycle: std::env::var("DREAM_VLM_MAX_JOBS_PER_CYCLE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
        }
    }

    /// Whether Dream Mode schedulers are active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

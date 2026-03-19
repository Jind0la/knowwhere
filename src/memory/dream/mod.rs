//! Dream Mode — Two separate processes
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Dream Mode Definition"
//!
//! Dream Mode consists of TWO separate processes that must NOT be mixed:
//!
//! 1. **Consolidation** (`consolidation.rs`): Bündelt, clustert, verdichtet.
//!    Creates summary nodes from episodic memories. Is about building.
//!
//! 2. **Audit** (`audit.rs`): Prüft auf Drift, Konflikte, Sensitivität.
//!    Flags issues in existing memory structures. Is about checking.
//!
//! Calling this `DreamMode` is a legacy name. Prefer importing the specific
//! engines you need: `consolidation::ConsolidationEngine` or `audit::AuditEngine`.

pub mod audit;
pub mod consolidation;

pub use audit::{AuditConfig, AuditEngine, AuditFinding, AuditReport, AuditFindingType};
pub use consolidation::{ConsolidationConfig, ConsolidationEngine, ConsolidationReport, MemoryCluster};

// ---------------------------------------------------------------------------
// Legacy DreamMode (from original dream.rs)
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::time::{Duration as TokioDuration, Instant};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::memory::fractal_node::cosine_similarity;
use crate::memory::FractalNode;
use crate::storage::MemoryStore;

const SIMILARITY_THRESHOLD: f32 = 0.85;
const BOOST_FACTOR: f64 = 1.1;
const DECAY_FACTOR: f64 = 0.95;
const YOUNG_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DreamStatus {
    pub active: bool,
    pub phase: String,
    pub memories_processed: u64,
    pub consolidations_run: u64,
    pub last_run: Option<DateTime<Utc>>,
}

impl Default for DreamStatus {
    fn default() -> Self {
        Self {
            active: true,
            phase: "idle".to_string(),
            memories_processed: 0,
            consolidations_run: 0,
            last_run: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DreamMode {
    store: MemoryStore,
    status: Arc<RwLock<DreamStatus>>,
    last_micro_dream: Arc<RwLock<Instant>>,
}

impl DreamMode {
    pub fn new(store: MemoryStore) -> Self {
        Self {
            store,
            status: Arc::new(RwLock::new(DreamStatus::default())),
            last_micro_dream: Arc::new(RwLock::new(Instant::now())),
        }
    }

    pub async fn status(&self) -> DreamStatus {
        self.status.read().await.clone()
    }

    /// Micro-dream: adjust node weights by age (boost recent, decay old).
    pub async fn micro_dream(&self) {
        let now = Utc::now();
        let nodes = match self.store.list_all().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("micro_dream list_all failed: {e}");
                return;
            }
        };

        let cutoff = now - chrono::Duration::hours(YOUNG_HOURS);
        let mut updated = 0usize;
        for node in &nodes {
            let factor = if node.created_at > cutoff {
                BOOST_FACTOR
            } else {
                DECAY_FACTOR
            };
            if self
                .store
                .update_node(&node.id, |n| {
                    n.weight *= factor;
                })
                .await
                .is_ok()
            {
                updated += 1;
            }
        }

        let mut status = self.status.write().await;
        status.last_run = Some(now);
        status.memories_processed += updated as u64;

        *self.last_micro_dream.write().await = Instant::now();

        tracing::info!(
            cycle = status.memories_processed,
            updated,
            total = nodes.len(),
            "micro_dream complete"
        );
    }

    /// Full dream: cluster similar nodes under meta-cluster parents.
    pub async fn full_dream(&self) {
        let nodes = match self.store.list_all().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("full_dream list_all failed: {e}");
                return;
            }
        };

        let mut merged: Vec<(Uuid, Uuid)> = Vec::new();
        for (i, a) in nodes.iter().enumerate() {
            for b in nodes.iter().skip(i + 1) {
                let sim = cosine_similarity(&a.vector, &b.vector);
                if sim > SIMILARITY_THRESHOLD {
                    merged.push((a.id, b.id));
                }
            }
        }

        for (id_a, id_b) in &merged {
            let (Some(a), Some(b)) = (
                self.store.get(id_a).await.ok().flatten(),
                self.store.get(id_b).await.ok().flatten(),
            ) else {
                continue;
            };

            let avg_vec: Vec<f32> = a
                .vector
                .iter()
                .zip(&b.vector)
                .map(|(x, y)| (x + y) / 2.0)
                .collect();

            let mut meta = FractalNode::new_session(
                format!("meta-cluster [{id_a} + {id_b}]"),
                avg_vec,
                HashMap::new(),
            );
            meta.children = vec![a.clone(), b.clone()];

            let _ = self.store.insert(meta).await;
        }

        let mut status = self.status.write().await;
        status.consolidations_run += 1;

        tracing::info!(clusters = merged.len(), "full_dream complete");
    }

    pub async fn micro_dream_loop(self) {
        loop {
            tokio::time::sleep(TokioDuration::from_secs(3600)).await;
            self.micro_dream().await;
        }
    }
}

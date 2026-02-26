use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::time::{self, Duration};
use utoipa::ToSchema;
use uuid::Uuid;

use super::fractal_node::cosine_similarity;
use super::FractalNode;
use crate::storage::MemoryStore;

const SIMILARITY_THRESHOLD: f32 = 0.85;
const BOOST_FACTOR: f64 = 1.1;
const DECAY_FACTOR: f64 = 0.95;
const YOUNG_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DreamStatus {
    pub last_run: Option<DateTime<Utc>>,
    pub cycle_count: u64,
}

#[derive(Debug, Clone)]
pub struct DreamMode {
    store: MemoryStore,
    status: Arc<RwLock<DreamStatus>>,
}

impl DreamMode {
    pub fn new(store: MemoryStore) -> Self {
        Self {
            store,
            status: Arc::new(RwLock::new(DreamStatus {
                last_run: None,
                cycle_count: 0,
            })),
        }
    }

    pub async fn status(&self) -> DreamStatus {
        self.status.read().await.clone()
    }

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
        for node in &nodes {
            let factor = if node.created_at > cutoff {
                BOOST_FACTOR
            } else {
                DECAY_FACTOR
            };
            let _ = self
                .store
                .update_node(&node.id, |n| n.weight *= factor)
                .await;
        }

        let mut status = self.status.write().await;
        status.last_run = Some(now);
        status.cycle_count += 1;

        tracing::info!(
            cycle = status.cycle_count,
            nodes = nodes.len(),
            "micro_dream complete"
        );
    }

    /// Einfache Overlap-Community-Detection: Paare mit hoher Similarity werden
    /// unter einem gemeinsamen Meta-Knoten zusammengefasst.
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
            meta.children = vec![a, b];

            let _ = self.store.insert(meta).await;
        }

        tracing::info!(clusters = merged.len(), "full_dream complete");
    }

    pub async fn micro_dream_loop(self) {
        loop {
            time::sleep(Duration::from_secs(3600)).await;
            self.micro_dream().await;
        }
    }
}

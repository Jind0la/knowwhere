use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::RwLock;
use usearch::{new_index, Index, IndexOptions, MetricKind, ScalarKind};
use uuid::Uuid;

use crate::memory::FractalNode;

const USEARCH_THRESHOLD: usize = 50;

/// Wrapper for thread-safe access to USearch Index.
/// The underlying C++ library is thread-safe when externally synchronized via Mutex.
struct SendableIndex(Index);
unsafe impl Send for SendableIndex {}

impl SendableIndex {
    fn new(options: &IndexOptions) -> Result<Self> {
        let index = new_index(options).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self(index))
    }

    fn reserve(&self, capacity: usize) -> Result<()> {
        self.0
            .reserve(capacity)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn add(&self, key: u64, vector: &[f32]) -> Result<()> {
        self.0
            .add(key, vector)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn search(&self, vector: &[f32], count: usize) -> Vec<u64> {
        match self.0.search(vector, count) {
            Ok(matches) => matches.keys,
            Err(e) => {
                tracing::warn!("usearch search error: {e}");
                vec![]
            }
        }
    }
}

#[derive(Clone)]
pub struct MemoryStore {
    nodes: Arc<RwLock<HashMap<Uuid, FractalNode>>>,
    usearch_index: Arc<Mutex<Option<SendableIndex>>>,
    uuid_to_key: Arc<RwLock<HashMap<Uuid, u64>>>,
    key_to_uuid: Arc<RwLock<HashMap<u64, Uuid>>>,
    next_key: Arc<AtomicU64>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            usearch_index: Arc::new(Mutex::new(None)),
            uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
            next_key: Arc::new(AtomicU64::new(1)),
        }
    }

    fn ensure_index(&self, dimension: usize) -> Result<()> {
        let mut guard = self.usearch_index.lock().unwrap();
        if guard.is_none() {
            let options = IndexOptions {
                dimensions: dimension,
                metric: MetricKind::Cos,
                quantization: ScalarKind::F32,
                connectivity: 0,
                expansion_add: 0,
                expansion_search: 0,
                multi: false,
            };
            let index = SendableIndex::new(&options)?;
            index.reserve(1024)?;
            *guard = Some(index);
            tracing::info!(dimension, "usearch index initialized");
        }
        Ok(())
    }

    pub async fn insert(&self, node: FractalNode) -> Result<Uuid> {
        let id = node.id;

        if !node.vector.is_empty() {
            self.ensure_index(node.vector.len())?;
            let key = self.next_key.fetch_add(1, AtomicOrdering::Relaxed);

            let indexed = {
                let guard = self.usearch_index.lock().unwrap();
                if let Some(ref index) = *guard {
                    match index.add(key, &node.vector) {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::warn!(
                                %id,
                                dim = node.vector.len(),
                                "skipping usearch index (dimension mismatch?): {e}"
                            );
                            false
                        }
                    }
                } else {
                    false
                }
            };

            if indexed {
                self.uuid_to_key.write().await.insert(id, key);
                self.key_to_uuid.write().await.insert(key, id);
            }
        }

        self.nodes.write().await.insert(id, node);
        Ok(id)
    }

    pub async fn get(&self, id: &Uuid) -> Result<Option<FractalNode>> {
        Ok(self.nodes.read().await.get(id).cloned())
    }

    pub async fn list_all(&self) -> Result<Vec<FractalNode>> {
        Ok(self.nodes.read().await.values().cloned().collect())
    }

    pub async fn count(&self) -> usize {
        self.nodes.read().await.len()
    }

    pub async fn recent(&self, limit: usize) -> Vec<FractalNode> {
        let nodes = self.nodes.read().await;
        let mut all: Vec<FractalNode> = nodes.values().cloned().collect();
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all.truncate(limit);
        all
    }

    pub async fn update_node<F>(&self, id: &Uuid, updater: F) -> Result<()>
    where
        F: FnOnce(&mut FractalNode),
    {
        if let Some(node) = self.nodes.write().await.get_mut(id) {
            updater(node);
        }
        Ok(())
    }

    pub async fn retrieve_fractal(
        &self,
        query_vector: &[f32],
        top_k: usize,
        max_depth: usize,
    ) -> Vec<FractalNode> {
        let node_count = self.nodes.read().await.len();
        let has_index = self.usearch_index.lock().unwrap().is_some();

        if node_count >= USEARCH_THRESHOLD && has_index && !query_vector.is_empty() {
            let candidate_keys = {
                let guard = self.usearch_index.lock().unwrap();
                match guard.as_ref() {
                    Some(index) => index.search(query_vector, top_k * 2),
                    None => vec![],
                }
            };

            if !candidate_keys.is_empty() {
                let k2u = self.key_to_uuid.read().await;
                let candidate_uuids: Vec<Uuid> = candidate_keys
                    .iter()
                    .filter_map(|k| k2u.get(k).copied())
                    .collect();
                drop(k2u);

                let nodes = self.nodes.read().await;
                let mut scored: Vec<(f32, FractalNode)> = candidate_uuids
                    .iter()
                    .filter_map(|uid| nodes.get(uid))
                    .flat_map(|node| node.zoom_retrieve(query_vector, max_depth))
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
                return scored
                    .into_iter()
                    .take(top_k)
                    .map(|(_, n)| n)
                    .collect();
            }
        }

        // Fallback: fraktales Zoomen ueber alle Nodes
        let nodes = self.nodes.read().await;
        let mut scored: Vec<(f32, FractalNode)> = nodes
            .values()
            .flat_map(|node| node.zoom_retrieve(query_vector, max_depth))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        scored
            .into_iter()
            .take(top_k)
            .map(|(_, n)| n)
            .collect()
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use bm25::{Embedder, EmbedderBuilder, Language, Scorer};
use usearch::{new_index, Index, IndexOptions, MetricKind, ScalarKind};
use uuid::Uuid;

use crate::memory::FractalNode;

const USEARCH_THRESHOLD: usize = 50;
const SAVE_DEBOUNCE_SECS: u64 = 5;

struct CachedBm25 {
    embedder: Embedder,
    scorer: Scorer<Uuid>,
}


/// Wrapper for thread-safe access to USearch Index.
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

    #[allow(dead_code)]
    fn save(&self, path: &str) -> Result<()> {
        self.0.save(path).map_err(|e| anyhow::anyhow!("{e}"))
    }

    #[allow(dead_code)]
    fn load(&self, path: &str) -> Result<()> {
        self.0.load(path).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedState {
    nodes: HashMap<Uuid, FractalNode>,
    uuid_to_key: HashMap<Uuid, u64>,
    key_to_uuid: HashMap<u64, Uuid>,
    next_key: u64,
}

#[derive(Clone)]
pub struct MemoryStore {
    nodes: Arc<RwLock<HashMap<Uuid, FractalNode>>>,
    usearch_index: Arc<Mutex<Option<SendableIndex>>>,
    index_dimension: Arc<Mutex<Option<usize>>>,
    uuid_to_key: Arc<RwLock<HashMap<Uuid, u64>>>,
    key_to_uuid: Arc<RwLock<HashMap<u64, Uuid>>>,
    next_key: Arc<AtomicU64>,
    data_dir: Option<PathBuf>,
    last_save: Arc<Mutex<Instant>>,
    bm25_corpus: Arc<RwLock<Vec<(Uuid, String)>>>,
    bm25_cache: Arc<Mutex<Option<CachedBm25>>>,
    bm25_dirty: Arc<Mutex<bool>>,
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
            index_dimension: Arc::new(Mutex::new(None)),
            uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
            next_key: Arc::new(AtomicU64::new(1)),
            data_dir: None,
            last_save: Arc::new(Mutex::new(Instant::now())),
            bm25_corpus: Arc::new(RwLock::new(Vec::new())),
            bm25_cache: Arc::new(Mutex::new(None)),
            bm25_dirty: Arc::new(Mutex::new(true)),
        }
    }

    pub fn with_persistence(data_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let mut store = Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
            usearch_index: Arc::new(Mutex::new(None)),
            index_dimension: Arc::new(Mutex::new(None)),
            uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
            next_key: Arc::new(AtomicU64::new(1)),
            data_dir: Some(dir.clone()),
            last_save: Arc::new(Mutex::new(Instant::now())),
            bm25_corpus: Arc::new(RwLock::new(Vec::new())),
            bm25_cache: Arc::new(Mutex::new(None)),
            bm25_dirty: Arc::new(Mutex::new(true)),
        };

        let state_path = dir.join("state.json");
        if state_path.exists() {
            match store.load_state(&state_path) {
                Ok(count) => tracing::info!(count, "loaded persisted state"),
                Err(e) => tracing::warn!("failed to load persisted state: {e}"),
            }
        }

        Ok(store)
    }

    fn load_state(&mut self, path: &Path) -> Result<usize> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let state: PersistedState = serde_json::from_reader(reader)?;

        let count = state.nodes.len();
        self.next_key = Arc::new(AtomicU64::new(state.next_key));

        // Rebuild USearch index from vectors
        let mut dimension = None;
        for node in state.nodes.values() {
            if !node.vector.is_empty() {
                dimension = Some(node.vector.len());
                break;
            }
        }

        if let Some(dim) = dimension {
            let options = IndexOptions {
                dimensions: dim,
                metric: MetricKind::Cos,
                quantization: ScalarKind::F32,
                connectivity: 0,
                expansion_add: 0,
                expansion_search: 0,
                multi: false,
            };
            if let Ok(index) = SendableIndex::new(&options) {
                let cap = state.nodes.len().max(1024);
                let _ = index.reserve(cap);

                for (&uuid, &key) in &state.uuid_to_key {
                    if let Some(node) = state.nodes.get(&uuid) {
                        if !node.vector.is_empty() {
                            if let Err(e) = index.add(key, &node.vector) {
                                tracing::warn!(%uuid, "rebuild index skip: {e}");
                            }
                        }
                    }
                }

                *self.usearch_index.lock().unwrap() = Some(index);
                *self.index_dimension.lock().unwrap() = Some(dim);
                tracing::info!(dim, "usearch index rebuilt from persisted state");
            }
        }

        let mut corpus = Vec::new();
        for (id, node) in &state.nodes {
            let text = node.content.as_deref()
                .or(node.original_pointer.as_deref())
                .unwrap_or("");
            if !text.is_empty() {
                corpus.push((*id, text.to_string()));
            }
        }
        self.bm25_corpus = Arc::new(RwLock::new(corpus));

        self.nodes = Arc::new(RwLock::new(state.nodes));
        self.uuid_to_key = Arc::new(RwLock::new(state.uuid_to_key));
        self.key_to_uuid = Arc::new(RwLock::new(state.key_to_uuid));

        Ok(count)
    }

    pub async fn save_to_disk(&self) -> Result<()> {
        let dir = match &self.data_dir {
            Some(d) => d,
            None => return Ok(()),
        };

        let state = PersistedState {
            nodes: self.nodes.read().await.clone(),
            uuid_to_key: self.uuid_to_key.read().await.clone(),
            key_to_uuid: self.key_to_uuid.read().await.clone(),
            next_key: self.next_key.load(AtomicOrdering::Relaxed),
        };

        let tmp_path = dir.join("state.json.tmp");
        let final_path = dir.join("state.json");

        let file = std::fs::File::create(&tmp_path)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer(writer, &state)?;
        std::fs::rename(&tmp_path, &final_path)?;

        Ok(())
    }

    async fn maybe_save(&self) {
        if self.data_dir.is_none() {
            return;
        }
        let should_save = {
            let mut last = self.last_save.lock().unwrap();
            if last.elapsed().as_secs() >= SAVE_DEBOUNCE_SECS {
                *last = Instant::now();
                true
            } else {
                false
            }
        };
        if should_save {
            if let Err(e) = self.save_to_disk().await {
                tracing::warn!("auto-save failed: {e}");
            }
        }
    }

    fn ensure_index(&self, dimension: usize) -> Result<()> {
        let mut guard = self
            .usearch_index
            .lock()
            .map_err(|_| anyhow::anyhow!("usearch mutex poisoned"))?;
        let mut dim_guard = self
            .index_dimension
            .lock()
            .map_err(|_| anyhow::anyhow!("dimension mutex poisoned"))?;

        let need_rebuild = match *dim_guard {
            None => true,
            Some(d) if d != dimension => {
                tracing::warn!(
                    old = d,
                    new = dimension,
                    "embedding dimension changed, rebuilding index"
                );
                true
            }
            _ => guard.is_none(),
        };

        if need_rebuild {
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
            *dim_guard = Some(dimension);
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
                let guard = self
                    .usearch_index
                    .lock()
                    .map_err(|_| anyhow::anyhow!("usearch mutex poisoned"))?;
                if let Some(ref index) = *guard {
                    match index.add(key, &node.vector) {
                        Ok(()) => true,
                        Err(e) => {
                            tracing::warn!(
                                %id,
                                dim = node.vector.len(),
                                "skipping usearch index: {e}"
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

        let bm25_text = node.content.as_deref()
            .or(node.original_pointer.as_deref())
            .unwrap_or("");
        if !bm25_text.is_empty() {
            self.bm25_corpus.write().await.push((id, bm25_text.to_string()));
            *self.bm25_dirty.lock().unwrap() = true;
        }

        self.nodes.write().await.insert(id, node);
        self.maybe_save().await;
        Ok(id)
    }

    pub async fn get(&self, id: &Uuid) -> Result<Option<FractalNode>> {
        Ok(self.nodes.read().await.get(id).cloned())
    }

    pub async fn delete(&self, id: &Uuid) -> Result<bool> {
        let removed = self.nodes.write().await.remove(id).is_some();
        if removed {
            if let Some(key) = self.uuid_to_key.write().await.remove(id) {
                self.key_to_uuid.write().await.remove(&key);
            }
        }
        Ok(removed)
    }

    pub async fn purge_dummy_vectors(&self) -> usize {
        let ids_to_remove: Vec<Uuid> = {
            let nodes = self.nodes.read().await;
            nodes
                .values()
                .filter(|n| !n.vector.is_empty() && n.vector.iter().all(|&v| (v - 0.1).abs() < 1e-6))
                .map(|n| n.id)
                .collect()
        };
        let count = ids_to_remove.len();
        for id in &ids_to_remove {
            let _ = self.delete(id).await;
        }
        count
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

    pub async fn update_vector(&self, id: &Uuid, new_vector: Vec<f32>) -> Result<bool> {
        if new_vector.is_empty() {
            return Ok(false);
        }
        self.ensure_index(new_vector.len())?;

        let mut nodes = self.nodes.write().await;
        let node = match nodes.get_mut(id) {
            Some(n) => n,
            None => return Ok(false),
        };
        node.vector = new_vector.clone();

        let existing_key = self.uuid_to_key.read().await.get(id).copied();
        let key = match existing_key {
            Some(k) => k,
            None => {
                let k = self.next_key.fetch_add(1, AtomicOrdering::Relaxed);
                self.uuid_to_key.write().await.insert(*id, k);
                self.key_to_uuid.write().await.insert(k, *id);
                k
            }
        };
        drop(nodes);

        {
            let guard = self.usearch_index.lock()
                .map_err(|_| anyhow::anyhow!("usearch mutex poisoned"))?;
            if let Some(ref index) = *guard {
                let _ = index.add(key, &new_vector);
            }
        }

        self.maybe_save().await;
        Ok(true)
    }

    fn rebuild_bm25_cache(&self, corpus: &[(Uuid, String)]) {
        let texts: Vec<&str> = corpus.iter().map(|(_, t)| t.as_str()).collect();
        let embedder: Embedder = EmbedderBuilder::with_fit_to_corpus(Language::German, &texts).build();
        let mut scorer: Scorer<Uuid> = Scorer::new();
        for (id, text) in corpus.iter() {
            scorer.upsert(id, embedder.embed(text));
        }
        *self.bm25_cache.lock().unwrap() = Some(CachedBm25 { embedder, scorer });
        *self.bm25_dirty.lock().unwrap() = false;
    }

    pub async fn search_bm25(&self, query: &str, top_k: usize) -> Vec<(Uuid, f32)> {
        let corpus = self.bm25_corpus.read().await;
        if corpus.is_empty() {
            return vec![];
        }

        let dirty = *self.bm25_dirty.lock().unwrap();
        if dirty || self.bm25_cache.lock().unwrap().is_none() {
            self.rebuild_bm25_cache(&corpus);
        }
        drop(corpus);

        let guard = self.bm25_cache.lock().unwrap();
        let cached = match guard.as_ref() {
            Some(c) => c,
            None => return vec![],
        };

        let query_embedding = cached.embedder.embed(query);
        let matches = cached.scorer.matches(&query_embedding);

        matches.into_iter()
            .take(top_k)
            .map(|m| (m.id, m.score))
            .collect()
    }

    pub(crate) fn rrf_fuse(
        vector_ranked: &[Uuid],
        bm25_ranked: &[(Uuid, f32)],
        k: f32,
    ) -> Vec<(Uuid, f32)> {
        let mut scores: HashMap<Uuid, f32> = HashMap::new();
        for (rank, id) in vector_ranked.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (k + rank as f32 + 1.0);
        }
        for (rank, (id, _)) in bm25_ranked.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (k + rank as f32 + 1.0);
        }
        let mut fused: Vec<_> = scores.into_iter().collect();
        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        fused
    }

    pub async fn hybrid_retrieve(
        &self,
        query_text: Option<&str>,
        query_vector: &[f32],
        top_k: usize,
        max_depth: usize,
    ) -> Vec<(f32, FractalNode)> {
        let vector_results = self.retrieve_fractal(query_vector, top_k * 2, max_depth).await;
        let vector_ids: Vec<Uuid> = vector_results.iter().map(|n| n.id).collect();

        let bm25_results = match query_text {
            Some(q) if !q.is_empty() => self.search_bm25(q, top_k * 2).await,
            _ => vec![],
        };

        if bm25_results.is_empty() {
            return vector_results.into_iter()
                .take(top_k)
                .filter_map(|n| {
                    let sim = crate::memory::cosine_similarity(&n.vector, query_vector);
                    Some((sim, n))
                })
                .collect();
        }

        let fused = Self::rrf_fuse(&vector_ids, &bm25_results, 60.0);

        let nodes = self.nodes.read().await;
        fused.into_iter()
            .take(top_k)
            .filter_map(|(id, score)| nodes.get(&id).cloned().map(|n| (score, n)))
            .collect()
    }

    pub async fn retrieve_fractal(
        &self,
        query_vector: &[f32],
        top_k: usize,
        max_depth: usize,
    ) -> Vec<FractalNode> {
        let node_count = self.nodes.read().await.len();
        let has_index = self
            .usearch_index
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false);

        if node_count >= USEARCH_THRESHOLD && has_index && !query_vector.is_empty() {
            let candidate_keys = {
                let guard = match self.usearch_index.lock() {
                    Ok(g) => g,
                    Err(_) => return vec![],
                };
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

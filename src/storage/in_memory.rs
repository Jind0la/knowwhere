use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use bm25::{Embedder, EmbedderBuilder, Language, Scorer};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use usearch::{new_index, Index, IndexOptions, MetricKind, ScalarKind};
use uuid::Uuid;

use crate::embedding::{embed_document, EmbeddingProvider};
use crate::memory::FractalNode;
use crate::storage::backend::EmbeddingRepairReport;
use crate::storage::pipeline;
use crate::storage::shared;
use crate::storage::{
    FusionStrategy, HybridQuery, RetrievalProfile, ScoredNode, StorageBackend, UpdateOperation,
};

const USEARCH_THRESHOLD: usize = 50;
const SAVE_DEBOUNCE_SECS: u64 = 60;
/// Only write binary indexes every Nth state save — they're large (USearch 100-900MB)
/// and rarely change structure. State (JSON) is written on every debounced save.
const BINARY_SAVE_EVERY_N: u64 = 10;

fn node_dimension(node: &FractalNode) -> Option<usize> {
    (!node.vector.is_empty()).then_some(node.vector.len())
}

fn repair_text(node: &FractalNode) -> Option<&str> {
    node.content
        .as_deref()
        .or(node.original_pointer.as_deref())
        .filter(|text| !text.trim().is_empty())
}

fn dominant_dimension(nodes: &HashMap<Uuid, FractalNode>) -> Option<usize> {
    let mut counts = HashMap::new();
    for dim in nodes.values().filter_map(node_dimension) {
        *counts.entry(dim).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(dim, _)| dim)
}

fn index_options(dimension: usize) -> IndexOptions {
    IndexOptions {
        dimensions: dimension,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        connectivity: 0,
        expansion_add: 0,
        expansion_search: 0,
        multi: false,
    }
}

const BINARY_INDEX_VERSION: &str = "0.5.0";

fn index_binary_path(data_dir: &Path, filename: &str) -> PathBuf {
    data_dir.join(filename)
}

fn version_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("usearch_version.txt")
}

/// Save a single USearch index to disk. Holds the mutex during the entire save
/// to prevent concurrent `add()` from corrupting the binary file.
fn save_index_binary(index: &Mutex<Option<SendableIndex>>, path: &Path) -> Result<()> {
    let guard = index.lock().expect("index mutex poisoned");
    if let Some(ref idx) = *guard {
        let path_str = path.to_string_lossy().to_string();
        idx.save(&path_str).map_err(|e| {
            anyhow::anyhow!("failed to save binary index to {}: {e}", path.display())
        })?;
        tracing::debug!("binary index saved to {}", path.display());
    }
    Ok(())
}

/// Load a USearch index from a binary file. Returns `true` if successfully loaded,
/// `false` if the file doesn't exist. Returns `Err` if the file exists but is corrupt
/// or has wrong dimensions.
fn load_index_binary(
    index: &Mutex<Option<SendableIndex>>,
    path: &Path,
    dim: usize,
) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let new_idx = SendableIndex::new(&index_options(dim))?;
    let path_str = path.to_string_lossy().to_string();
    new_idx
        .load(&path_str)
        .map_err(|e| anyhow::anyhow!("failed to load binary index from {}: {e}", path.display()))?;
    let mut guard = index.lock().expect("index mutex poisoned");
    *guard = Some(new_idx);
    Ok(true)
}

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
        self.0.reserve(capacity).map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn add(&self, key: u64, vector: &[f32]) -> Result<()> {
        self.0.add(key, vector).map_err(|e| anyhow::anyhow!("{e}"))
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

    fn save(&self, path: &str) -> Result<()> {
        self.0.save(path).map_err(|e| anyhow::anyhow!("{e}"))
    }

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
    /// Persisted BM25 corpus — rebuilt on startup if missing.
    /// Fixes MED-003: BM25 index for external nodes survives restart.
    bm25_corpus: Vec<(Uuid, String)>,
    /// Coarse 256d index mappings (survives restart).
    #[serde(default)]
    coarse_uuid_to_key: HashMap<Uuid, u64>,
    #[serde(default)]
    coarse_key_to_uuid: HashMap<u64, Uuid>,
    /// Ultra-coarse 64d index mappings (survives restart, TST cascade).
    #[serde(default)]
    ultra_coarse_uuid_to_key: HashMap<Uuid, u64>,
    #[serde(default)]
    ultra_coarse_key_to_uuid: HashMap<u64, Uuid>,
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
    save_count: Arc<AtomicU64>,
    bm25_corpus: Arc<RwLock<Vec<(Uuid, String)>>>,
    bm25_cache: Arc<Mutex<Option<CachedBm25>>>,
    bm25_dirty: Arc<Mutex<bool>>,
    // Matryoshka coarse index (256d truncated embeddings)
    coarse_index: Arc<Mutex<Option<SendableIndex>>>,
    coarse_dimension: Arc<Mutex<Option<usize>>>,
    coarse_uuid_to_key: Arc<RwLock<HashMap<Uuid, u64>>>,
    coarse_key_to_uuid: Arc<RwLock<HashMap<u64, Uuid>>>,
    // Matryoshka ultra-coarse index (64d truncated embeddings, TST-inspired)
    ultra_coarse_index: Arc<Mutex<Option<SendableIndex>>>,
    ultra_coarse_dimension: Arc<Mutex<Option<usize>>>,
    ultra_coarse_uuid_to_key: Arc<RwLock<HashMap<Uuid, u64>>>,
    ultra_coarse_key_to_uuid: Arc<RwLock<HashMap<u64, Uuid>>>,
}

const COARSE_DIM: usize = 256;
const ULTRA_COARSE_DIM: usize = 64;

#[async_trait::async_trait]
impl StorageBackend for MemoryStore {
    async fn insert(&self, node: FractalNode) -> anyhow::Result<Uuid> {
        self.insert(node).await
    }

    async fn get(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>> {
        self.get(id).await
    }

    async fn find_by_external_id(&self, external_id: &str) -> Option<Uuid> {
        self.find_by_external_id(external_id).await
    }

    async fn delete(&self, id: &Uuid) -> anyhow::Result<bool> {
        self.delete(id).await
    }

    async fn update_vector(&self, id: &Uuid, new_vector: Vec<f32>) -> anyhow::Result<bool> {
        self.update_vector(id, new_vector).await
    }

    async fn update(&self, id: &Uuid, op: UpdateOperation) -> anyhow::Result<()> {
        let op = op.clone();
        self.update_node(id, |n| op.apply(n)).await
    }

    async fn hybrid_retrieve(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
        let vector = query.query_vector.as_deref().unwrap_or(&[]);
        let fetch_k = query.profile.fetch_k(query.top_k);
        let mut raw_results = self
            .hybrid_retrieve(
                query.query_text.as_deref(),
                vector,
                fetch_k,
                query.max_depth,
                None, // recency_boost is policy-level; applied below only for !FullFidelity
                query.fusion_strategy,
                query.query_type_routing,
                #[cfg(feature = "postgres-storage")]
                None,
            )
            .await;
        // Apply recency_boost (temporal policy) only for non-FullFidelity profiles.
        // Low-level hybrid_retrieve now returns pure signals (recency/temporal gated at policy).
        if let Some(b) = query.recency_boost {
            if !matches!(query.profile, RetrievalProfile::FullFidelity) {
                pipeline::apply_temporal_boost(&mut raw_results, b);
            }
        }
        let weighted = pipeline::finalize_retrieval(raw_results, query);
        Ok(weighted)
    }

    async fn retrieve_fractal(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
        let vector = query.query_vector.as_deref().unwrap_or(&[]);
        let nodes = self
            .retrieve_fractal(
                vector,
                query.top_k,
                query.max_depth,
                #[cfg(feature = "postgres-storage")]
                FractalNode::ZOOM_PRUNING_THRESHOLD,
                #[cfg(feature = "postgres-storage")]
                None,
            )
            .await;
        // fractal retrieve returns raw nodes (no score) — use 1.0 as default,
        // filtered by user_id if specified
        Ok(nodes
            .into_iter()
            .filter(|node| {
                // user_id filter: scoped to single persona. None = permissive (all nodes).
                let node_uid = node.metadata.get("user_id").and_then(|v| v.as_str());
                match &query.user_id {
                    None => true,
                    Some(uid) => node_uid.is_none_or(|v| v == uid.as_str()),
                }
            })
            .map(|node| ScoredNode {
                id: node.id,
                score: 1.0,
                distribution_scores: None,
                debug: None,
                node,
            })
            .collect())
    }

    async fn search_bm25(
        &self,
        query_text: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<(Uuid, f32)>> {
        Ok(self.search_bm25(query_text, top_k).await)
    }

    async fn list_all(&self) -> anyhow::Result<Vec<FractalNode>> {
        Ok(self.list_all().await.unwrap_or_default())
    }

    async fn recent(&self, limit: usize) -> anyhow::Result<Vec<FractalNode>> {
        Ok(self.recent(limit).await)
    }

    async fn count(&self) -> usize {
        self.count().await
    }

    async fn purge_dummy_vectors(&self) -> usize {
        self.purge_dummy_vectors().await
    }

    async fn repair_embedding_dimensions(
        &self,
        provider: &dyn EmbeddingProvider,
    ) -> anyhow::Result<EmbeddingRepairReport> {
        let target_dimension = provider.dimension();
        let nodes = self.list_all().await?;
        let mut report = EmbeddingRepairReport {
            scanned: nodes.len(),
            repaired: 0,
            skipped: 0,
            target_dimension,
        };
        for node in nodes {
            if node_dimension(&node) == Some(target_dimension) {
                continue;
            }
            let Some(text) = repair_text(&node) else {
                report.skipped += 1;
                continue;
            };
            let vector = embed_document(provider, text).await?;
            if self.update_vector(&node.id, vector).await? {
                report.repaired += 1;
            } else {
                report.skipped += 1;
            }
        }
        let _ = self.rebuild_index_for_dimension(target_dimension).await?;
        Ok(report)
    }

    /// Matryoshka fractal zoom-out: expand via truncated embedding search.
    async fn expand_fractal(
        &self,
        nodes: Vec<ScoredNode>,
        query_vector: &[f32],
        max_depth: usize,
        pruning_threshold: f32,
    ) -> anyhow::Result<Vec<ScoredNode>> {
        use crate::memory::fractal_node::{cosine_similarity, truncate_vector};

        if max_depth == 0 || query_vector.is_empty() {
            return Ok(nodes);
        }

        const EXPAND_FRACTAL_MAX_EXTRA: usize = 100;
        let max_depth = max_depth.min(2);
        let mut expanded: Vec<ScoredNode> = nodes.clone();
        let mut seen: HashSet<Uuid> = nodes.iter().map(|s| s.node.id).collect();
        let max_total = nodes.len().saturating_add(EXPAND_FRACTAL_MAX_EXTRA);

        let all_nodes = self.nodes.read().await;

        if max_depth >= 1 {
            if let Some(coarse_256) = truncate_vector(query_vector, 256) {
                let neighbors = Self::search_by_truncated_vector(&all_nodes, &coarse_256, 256, 10);
                for n in neighbors {
                    if expanded.len() >= max_total {
                        break;
                    }
                    if !seen.insert(n.id) {
                        continue;
                    }
                    let sim = cosine_similarity(&n.vector, query_vector);
                    if sim >= pruning_threshold {
                        expanded.push(ScoredNode {
                            id: n.id,
                            score: sim,
                            distribution_scores: None,
                            debug: None,
                            node: n,
                        });
                    }
                }
            }
        }

        if max_depth >= 2 {
            if let Some(coarse_64) = truncate_vector(query_vector, 64) {
                let clusters = Self::search_by_truncated_vector(&all_nodes, &coarse_64, 64, 5);
                for c in clusters {
                    if expanded.len() >= max_total {
                        break;
                    }
                    if !seen.insert(c.id) {
                        continue;
                    }
                    let sim = cosine_similarity(&c.vector, query_vector);
                    if sim >= pruning_threshold * 0.8 {
                        expanded.push(ScoredNode {
                            id: c.id,
                            score: sim,
                            distribution_scores: None,
                            debug: None,
                            node: c,
                        });
                    }
                }
            }
        }

        expanded.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        Ok(expanded)
    }
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
            save_count: Arc::new(AtomicU64::new(0)),
            bm25_corpus: Arc::new(RwLock::new(Vec::new())),
            bm25_cache: Arc::new(Mutex::new(None)),
            bm25_dirty: Arc::new(Mutex::new(true)),
            coarse_index: Arc::new(Mutex::new(None)),
            coarse_dimension: Arc::new(Mutex::new(None)),
            coarse_uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            coarse_key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
            ultra_coarse_index: Arc::new(Mutex::new(None)),
            ultra_coarse_dimension: Arc::new(Mutex::new(None)),
            ultra_coarse_uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            ultra_coarse_key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
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
            save_count: Arc::new(AtomicU64::new(0)),
            bm25_corpus: Arc::new(RwLock::new(Vec::new())),
            bm25_cache: Arc::new(Mutex::new(None)),
            bm25_dirty: Arc::new(Mutex::new(true)),
            coarse_index: Arc::new(Mutex::new(None)),
            coarse_dimension: Arc::new(Mutex::new(None)),
            coarse_uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            coarse_key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
            ultra_coarse_index: Arc::new(Mutex::new(None)),
            ultra_coarse_dimension: Arc::new(Mutex::new(None)),
            ultra_coarse_uuid_to_key: Arc::new(RwLock::new(HashMap::new())),
            ultra_coarse_key_to_uuid: Arc::new(RwLock::new(HashMap::new())),
        };

        let state_path = dir.join("state.json");
        if state_path.exists() {
            match store.load_state(&state_path) {
                Ok(count) => {
                    tracing::info!(count, "loaded persisted state");
                    // Auto-save binary indices after rebuild so next startup is fast.
                    // Spawn because we're inside the tokio runtime — can't block_on here.
                    let store_clone = store.clone();
                    tokio::spawn(async move {
                        if let Err(e) = store_clone.save_to_disk().await {
                            tracing::warn!("failed to save binary indices after load: {e}");
                        }
                    });
                }
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

        // Determine data directory from state.json path
        let data_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));

        // --- Try binary USearch index load for fast startup ---
        let dimension = dominant_dimension(&state.nodes);
        let binary_available = version_file_path(data_dir).exists()
            && std::fs::read_to_string(version_file_path(data_dir))
                .map(|v| v.trim() == BINARY_INDEX_VERSION)
                .unwrap_or(false);

        let mut used_binary = false;

        if binary_available {
            if let Some(dim) = dimension {
                let main_loaded = load_index_binary(
                    &self.usearch_index,
                    &index_binary_path(data_dir, "usearch.bin"),
                    dim,
                );
                let coarse_loaded = load_index_binary(
                    &self.coarse_index,
                    &index_binary_path(data_dir, "usearch_coarse.bin"),
                    COARSE_DIM,
                );
                let ultra_loaded = load_index_binary(
                    &self.ultra_coarse_index,
                    &index_binary_path(data_dir, "usearch_ultra.bin"),
                    ULTRA_COARSE_DIM,
                );

                if let (Ok(true), Ok(_), Ok(_)) = (&main_loaded, &coarse_loaded, &ultra_loaded) {
                    // All three binary indices loaded successfully
                    *self
                        .index_dimension
                        .lock()
                        .expect("index_dimension mutex poisoned") = Some(dim);
                    tracing::info!("binary USearch indices loaded — skipping vector rebuild");
                    used_binary = true;
                } else {
                    // At least one binary load failed — log details and fall through to rebuild
                    if let Err(ref e) = main_loaded {
                        tracing::warn!(
                            "binary usearch index load failed, falling back to rebuild: {e}"
                        );
                    }
                    if let Err(ref e) = coarse_loaded {
                        tracing::warn!(
                            "binary coarse index load failed, falling back to rebuild: {e}"
                        );
                    }
                    if let Err(ref e) = ultra_loaded {
                        tracing::warn!(
                            "binary ultra-coarse index load failed, falling back to rebuild: {e}"
                        );
                    }
                    // Reset any partially-loaded indices
                    *self
                        .usearch_index
                        .lock()
                        .expect("usearch_index mutex poisoned") = None;
                    *self
                        .coarse_index
                        .lock()
                        .expect("coarse_index mutex poisoned") = None;
                    *self
                        .ultra_coarse_index
                        .lock()
                        .expect("ultra_coarse_index mutex poisoned") = None;
                }
            }
        }

        // --- Fallback: Rebuild USearch index from vectors ---
        if !used_binary {
            if let Some(dim) = dimension {
                if let Ok(index) = SendableIndex::new(&index_options(dim)) {
                    let cap = state.nodes.len().max(1024);
                    let _ = index.reserve(cap);
                    let mut skipped = 0usize;
                    let total = state.uuid_to_key.len();

                    for (i, (&uuid, &key)) in state.uuid_to_key.iter().enumerate() {
                        if (i + 1) % 1000 == 0 {
                            tracing::info!(
                                i = i + 1,
                                total,
                                "rebuilding usearch index from vectors..."
                            );
                        }
                        if let Some(node) = state.nodes.get(&uuid) {
                            if node_dimension(node) == Some(dim) {
                                if let Err(e) = index.add(key, &node.vector) {
                                    tracing::warn!(%uuid, "rebuild index skip: {e}");
                                }
                            } else if node_dimension(node).is_some() {
                                skipped += 1;
                            }
                        }
                    }

                    *self
                        .usearch_index
                        .lock()
                        .expect("usearch_index mutex poisoned") = Some(index);
                    *self
                        .index_dimension
                        .lock()
                        .expect("index_dimension mutex poisoned") = Some(dim);
                    tracing::info!(dim, skipped, "usearch index rebuilt from persisted state");
                }
            }
        }

        // --- Coarse index: binary or rebuild ---
        if !used_binary {
            let coarse_uuids = state.coarse_uuid_to_key.clone();
            if !coarse_uuids.is_empty() {
                let coarse_dim = COARSE_DIM;
                if let Ok(coarse_idx) = SendableIndex::new(&index_options(coarse_dim)) {
                    let cap = coarse_uuids.len().max(1024);
                    let _ = coarse_idx.reserve(cap);
                    let mut coarse_skipped = 0usize;
                    let coarse_total = coarse_uuids.len();
                    for (i, (&uuid, &key)) in coarse_uuids.iter().enumerate() {
                        if (i + 1) % 1000 == 0 {
                            tracing::info!(
                                i = i + 1,
                                total = coarse_total,
                                "rebuilding coarse index from vectors..."
                            );
                        }
                        if let Some(node) = state.nodes.get(&uuid) {
                            if node.vector.len() >= coarse_dim {
                                let vec: Vec<f32> = node.vector[..coarse_dim].to_vec();
                                if let Err(e) = coarse_idx.add(key, &vec) {
                                    tracing::warn!(%uuid, "coarse index rebuild skip: {e}");
                                }
                            } else {
                                coarse_skipped += 1;
                            }
                        }
                    }
                    *self
                        .coarse_index
                        .lock()
                        .expect("coarse_index mutex poisoned") = Some(coarse_idx);
                    *self
                        .coarse_dimension
                        .lock()
                        .expect("coarse_dimension mutex poisoned") = Some(coarse_dim);
                    tracing::info!(
                        coarse_dim,
                        coarse_skipped,
                        "coarse usearch index rebuilt from persisted state"
                    );
                }
            }
        }
        self.coarse_uuid_to_key = Arc::new(RwLock::new(state.coarse_uuid_to_key));
        self.coarse_key_to_uuid = Arc::new(RwLock::new(state.coarse_key_to_uuid));

        // --- Ultra-coarse index: binary or rebuild ---
        if !used_binary {
            let ultra_uuids = state.ultra_coarse_uuid_to_key.clone();
            if !ultra_uuids.is_empty() {
                let ultra_dim = ULTRA_COARSE_DIM;
                if let Ok(ultra_idx) = SendableIndex::new(&index_options(ultra_dim)) {
                    let cap = ultra_uuids.len().max(1024);
                    let _ = ultra_idx.reserve(cap);
                    let mut ultra_skipped = 0usize;
                    let ultra_total = ultra_uuids.len();
                    for (i, (&uuid, &key)) in ultra_uuids.iter().enumerate() {
                        if (i + 1) % 1000 == 0 {
                            tracing::info!(
                                i = i + 1,
                                total = ultra_total,
                                "rebuilding ultra-coarse index from vectors..."
                            );
                        }
                        if let Some(node) = state.nodes.get(&uuid) {
                            if node.vector.len() >= ultra_dim {
                                let vec: Vec<f32> = node.vector[..ultra_dim].to_vec();
                                if let Err(e) = ultra_idx.add(key, &vec) {
                                    tracing::warn!(%uuid, "ultra-coarse index rebuild skip: {e}");
                                }
                            } else {
                                ultra_skipped += 1;
                            }
                        }
                    }
                    *self
                        .ultra_coarse_index
                        .lock()
                        .expect("ultra_coarse_index mutex poisoned") = Some(ultra_idx);
                    *self
                        .ultra_coarse_dimension
                        .lock()
                        .expect("ultra_coarse_dimension mutex poisoned") = Some(ultra_dim);
                    tracing::info!(
                        ultra_dim,
                        ultra_skipped,
                        "ultra-coarse usearch index rebuilt from persisted state"
                    );
                }
            }
        }
        self.ultra_coarse_uuid_to_key = Arc::new(RwLock::new(state.ultra_coarse_uuid_to_key));
        self.ultra_coarse_key_to_uuid = Arc::new(RwLock::new(state.ultra_coarse_key_to_uuid));

        // Load persisted BM25 corpus directly — survives restart for external nodes.
        // Backward compat: if bm25_corpus is empty (old state file), rebuild from nodes.
        let corpus = if state.bm25_corpus.is_empty() {
            let mut rebuilt = Vec::new();
            for (id, node) in &state.nodes {
                let text = node
                    .content
                    .as_deref()
                    .or(node.original_pointer.as_deref())
                    .unwrap_or("");
                if !text.is_empty() {
                    rebuilt.push((*id, text.to_string()));
                }
            }
            tracing::info!(
                count = rebuilt.len(),
                "BM25 corpus rebuilt from nodes (empty persisted state)"
            );
            rebuilt
        } else {
            tracing::info!(
                count = state.bm25_corpus.len(),
                "BM25 corpus loaded from persisted state"
            );
            state.bm25_corpus
        };
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
            bm25_corpus: self.bm25_corpus.read().await.clone(),
            coarse_uuid_to_key: self.coarse_uuid_to_key.read().await.clone(),
            coarse_key_to_uuid: self.coarse_key_to_uuid.read().await.clone(),
            ultra_coarse_uuid_to_key: self.ultra_coarse_uuid_to_key.read().await.clone(),
            ultra_coarse_key_to_uuid: self.ultra_coarse_key_to_uuid.read().await.clone(),
        };

        let tmp_path = dir.join("state.json.tmp");
        let final_path = dir.join("state.json");

        let file = std::fs::File::create(&tmp_path)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer(writer, &state)?;
        std::fs::rename(&tmp_path, &final_path)?;

        // Save binary USearch indices — expensive (100-900MB each), only every Nth save.
        // Rapid inserts (benchmarks, batch loads) would otherwise trigger macOS
        // disk-write limits (52+ MB/s) and kernel kills (see Bug #5 crash report).
        let count = self.save_count.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        if count.is_multiple_of(BINARY_SAVE_EVERY_N) {
            save_index_binary(&self.usearch_index, &index_binary_path(dir, "usearch.bin"))?;
            save_index_binary(
                &self.coarse_index,
                &index_binary_path(dir, "usearch_coarse.bin"),
            )?;
            save_index_binary(
                &self.ultra_coarse_index,
                &index_binary_path(dir, "usearch_ultra.bin"),
            )?;

            // Write version marker for format detection on load
            let ver_path = version_file_path(dir);
            std::fs::write(&ver_path, BINARY_INDEX_VERSION)?;
        }

        Ok(())
    }

    /// Force-save binary USearch indices. Call on graceful shutdown.
    pub fn save_binaries_sync(&self) {
        let dir = match &self.data_dir {
            Some(d) => d,
            None => return,
        };
        let dir = dir.clone();
        if let Err(e) = (|| -> Result<()> {
            save_index_binary(&self.usearch_index, &index_binary_path(&dir, "usearch.bin"))?;
            save_index_binary(
                &self.coarse_index,
                &index_binary_path(&dir, "usearch_coarse.bin"),
            )?;
            save_index_binary(
                &self.ultra_coarse_index,
                &index_binary_path(&dir, "usearch_ultra.bin"),
            )?;
            std::fs::write(version_file_path(&dir), BINARY_INDEX_VERSION)?;
            Ok(())
        })() {
            tracing::warn!("shutdown: failed to save binary indexes: {e}");
        }
    }

    async fn maybe_save(&self) {
        if self.data_dir.is_none() {
            return;
        }
        let should_save = {
            let mut last = self.last_save.lock().expect("last_save mutex poisoned");
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

    fn ensure_index(&self, dimension: usize, extra: usize) -> Result<()> {
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
            let index = SendableIndex::new(&index_options(dimension))?;
            index.reserve((1024 + extra).max(1024))?;
            *guard = Some(index);
            *dim_guard = Some(dimension);
            tracing::info!(dimension, "usearch index initialized");
        } else if let Some(ref index) = *guard {
            // Re-reserve when existing index capacity is insufficient.
            // Without this, every insert after the initial 1024 slots triggers
            // "Reserve capacity ahead of insertions!" warnings from USearch.
            let needed = index.0.size().saturating_add(extra);
            if needed > index.0.capacity() {
                index.reserve(needed)?;
            }
        }
        Ok(())
    }

    fn ensure_coarse_index(&self, dimension: usize, extra: usize) -> Result<()> {
        let mut guard = self
            .coarse_index
            .lock()
            .map_err(|_| anyhow::anyhow!("coarse index mutex poisoned"))?;
        let mut dim_guard = self
            .coarse_dimension
            .lock()
            .map_err(|_| anyhow::anyhow!("coarse dimension mutex poisoned"))?;

        let need_rebuild = match *dim_guard {
            None => true,
            Some(d) if d != dimension => true,
            _ => guard.is_none(),
        };

        if need_rebuild {
            let index = SendableIndex::new(&index_options(dimension))?;
            index.reserve((1024 + extra).max(1024))?;
            *guard = Some(index);
            *dim_guard = Some(dimension);
            tracing::info!(dimension, "coarse usearch index initialized");
        } else if let Some(ref index) = *guard {
            let needed = index.0.size().saturating_add(extra);
            if needed > index.0.capacity() {
                index.reserve(needed)?;
            }
        }
        Ok(())
    }

    fn ensure_ultra_coarse_index(&self, dimension: usize, extra: usize) -> Result<()> {
        let mut guard = self
            .ultra_coarse_index
            .lock()
            .map_err(|_| anyhow::anyhow!("ultra-coarse index mutex poisoned"))?;
        let mut dim_guard = self
            .ultra_coarse_dimension
            .lock()
            .map_err(|_| anyhow::anyhow!("ultra-coarse dimension mutex poisoned"))?;

        let need_rebuild = match *dim_guard {
            None => true,
            Some(d) if d != dimension => true,
            _ => guard.is_none(),
        };

        if need_rebuild {
            let index = SendableIndex::new(&index_options(dimension))?;
            index.reserve((1024 + extra).max(1024))?;
            *guard = Some(index);
            *dim_guard = Some(dimension);
            tracing::info!(dimension, "ultra-coarse usearch index initialized");
        } else if let Some(ref index) = *guard {
            let needed = index.0.size().saturating_add(extra);
            if needed > index.0.capacity() {
                index.reserve(needed)?;
            }
        }
        Ok(())
    }

    async fn coarse_search(&self, vector: &[f32], count: usize) -> Vec<Uuid> {
        let match_keys: Vec<u64> = {
            let guard = match self.coarse_index.lock() {
                Ok(g) => g,
                Err(_) => return vec![],
            };
            let index = match guard.as_ref() {
                Some(i) => i,
                None => return vec![],
            };
            match index.0.search(vector, count) {
                Ok(m) => m.keys,
                Err(e) => {
                    tracing::warn!("coarse search error: {e}");
                    return vec![];
                }
            }
        }; // guard dropped — no MutexGuard across .await
        let key_to_uuid = self.coarse_key_to_uuid.read().await;
        match_keys
            .iter()
            .filter_map(|k| key_to_uuid.get(k).copied())
            .collect()
    }

    async fn ultra_coarse_search(&self, vector: &[f32], count: usize) -> Vec<Uuid> {
        let match_keys: Vec<u64> = {
            let guard = match self.ultra_coarse_index.lock() {
                Ok(g) => g,
                Err(_) => return vec![],
            };
            let index = match guard.as_ref() {
                Some(i) => i,
                None => return vec![],
            };
            match index.0.search(vector, count) {
                Ok(m) => m.keys,
                Err(e) => {
                    tracing::warn!("ultra-coarse search error: {e}");
                    return vec![];
                }
            }
        }; // guard dropped — no MutexGuard across .await
        let key_to_uuid = self.ultra_coarse_key_to_uuid.read().await;
        match_keys
            .iter()
            .filter_map(|k| key_to_uuid.get(k).copied())
            .collect()
    }

    fn index_key(
        &self,
        id: Uuid,
        uuid_to_key: &mut HashMap<Uuid, u64>,
        key_to_uuid: &mut HashMap<u64, Uuid>,
    ) -> u64 {
        if let Some(key) = uuid_to_key.get(&id).copied() {
            return key;
        }
        let key = self.next_key.fetch_add(1, AtomicOrdering::Relaxed);
        uuid_to_key.insert(id, key);
        key_to_uuid.insert(key, id);
        key
    }

    async fn rebuild_index_for_dimension(&self, dimension: usize) -> Result<usize> {
        let nodes = self.nodes.read().await.clone();
        let ids: HashSet<Uuid> = nodes
            .iter()
            .filter(|(_, node)| node_dimension(node) == Some(dimension))
            .map(|(id, _)| *id)
            .collect();
        let mut uuid_to_key = self.uuid_to_key.write().await;
        let mut key_to_uuid = self.key_to_uuid.write().await;
        uuid_to_key.retain(|id, _| ids.contains(id));
        key_to_uuid.retain(|_, id| ids.contains(id));
        let index = SendableIndex::new(&index_options(dimension))?;
        index.reserve(ids.len().max(1024))?;
        let mut indexed = 0usize;
        for id in ids {
            let key = self.index_key(id, &mut uuid_to_key, &mut key_to_uuid);
            if index.add(key, &nodes[&id].vector).is_ok() {
                indexed += 1;
            }
        }
        *self
            .usearch_index
            .lock()
            .expect("usearch_index mutex poisoned") = Some(index);
        *self
            .index_dimension
            .lock()
            .expect("index_dimension mutex poisoned") = Some(dimension);
        tracing::info!(dimension, indexed, "usearch index rebuilt");
        Ok(indexed)
    }

    pub async fn insert(&self, node: FractalNode) -> Result<Uuid> {
        let id = node.id;

        if !node.vector.is_empty() {
            // Fine index (768d)
            self.ensure_index(node.vector.len(), 1)?;
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

            // Coarse index (256d truncated)
            if node.vector.len() >= COARSE_DIM {
                let coarse_vec: Vec<f32> = node.vector[..COARSE_DIM].to_vec();
                self.ensure_coarse_index(COARSE_DIM, 1)?;
                let coarse_key = self.next_key.fetch_add(1, AtomicOrdering::Relaxed);

                let coarse_indexed = {
                    let guard = self
                        .coarse_index
                        .lock()
                        .map_err(|_| anyhow::anyhow!("coarse index mutex poisoned"))?;
                    if let Some(ref index) = *guard {
                        match index.add(coarse_key, &coarse_vec) {
                            Ok(()) => true,
                            Err(e) => {
                                tracing::warn!(%id, "skipping coarse index: {e}");
                                false
                            }
                        }
                    } else {
                        false
                    }
                };

                if coarse_indexed {
                    self.coarse_uuid_to_key.write().await.insert(id, coarse_key);
                    self.coarse_key_to_uuid.write().await.insert(coarse_key, id);
                }
            }

            // Ultra-coarse index (64d truncated, TST-inspired 3-level cascade)
            if node.vector.len() >= ULTRA_COARSE_DIM {
                let ultra_vec: Vec<f32> = node.vector[..ULTRA_COARSE_DIM].to_vec();
                self.ensure_ultra_coarse_index(ULTRA_COARSE_DIM, 1)?;
                let ultra_key = self.next_key.fetch_add(1, AtomicOrdering::Relaxed);

                let ultra_indexed = {
                    let guard = self
                        .ultra_coarse_index
                        .lock()
                        .map_err(|_| anyhow::anyhow!("ultra-coarse index mutex poisoned"))?;
                    if let Some(ref index) = *guard {
                        match index.add(ultra_key, &ultra_vec) {
                            Ok(()) => true,
                            Err(e) => {
                                tracing::warn!(%id, "skipping ultra-coarse index: {e}");
                                false
                            }
                        }
                    } else {
                        false
                    }
                };

                if ultra_indexed {
                    self.ultra_coarse_uuid_to_key
                        .write()
                        .await
                        .insert(id, ultra_key);
                    self.ultra_coarse_key_to_uuid
                        .write()
                        .await
                        .insert(ultra_key, id);
                }
            }
        }

        let bm25_text = node
            .content
            .as_deref()
            .or(node.original_pointer.as_deref())
            .unwrap_or("");
        if !bm25_text.is_empty() {
            self.bm25_corpus
                .write()
                .await
                .push((id, bm25_text.to_string()));
            *self.bm25_dirty.lock().expect("bm25_dirty mutex poisoned") = true;
        }

        self.nodes.write().await.insert(id, node);
        self.maybe_save().await;
        Ok(id)
    }

    pub async fn insert_many(&self, nodes: Vec<FractalNode>) -> Result<Vec<Uuid>> {
        use futures::future::try_join_all;

        // Pre-allocate USearch capacity for the entire batch so concurrent
        // inserts below don't trigger "reserve capacity ahead of insertions"
        // warnings (one per growth event).
        if let Some(dim) = nodes
            .iter()
            .find(|n| !n.vector.is_empty())
            .map(|n| n.vector.len())
        {
            self.ensure_index(dim, nodes.len())?;
        }

        let ids: Vec<_> = nodes.into_iter().map(|n| self.insert(n)).collect();
        try_join_all(ids).await
    }

    pub async fn get(&self, id: &Uuid) -> Result<Option<FractalNode>> {
        Ok(self.nodes.read().await.get(id).cloned())
    }

    /// Check if a node with the given external_id already exists.
    /// Returns the existing node's UUID if found.
    pub async fn find_by_external_id(&self, external_id: &str) -> Option<Uuid> {
        let nodes = self.nodes.read().await;
        for (id, node) in nodes.iter() {
            if let Some(meta) = node.metadata.get("external_id") {
                if let Some(eid) = meta.as_str() {
                    if eid == external_id {
                        return Some(*id);
                    }
                }
            }
        }
        None
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
                .filter(|n| {
                    !n.vector.is_empty() && n.vector.iter().all(|&v| (v - 0.1).abs() < 1e-6)
                })
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
        self.ensure_index(new_vector.len(), 1)?;

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
            let guard = self
                .usearch_index
                .lock()
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
        let embedder: Embedder =
            EmbedderBuilder::with_fit_to_corpus(Language::German, &texts).build();
        let mut scorer: Scorer<Uuid> = Scorer::new();
        for (id, text) in corpus.iter() {
            scorer.upsert(id, embedder.embed(text));
        }
        *self.bm25_cache.lock().expect("bm25_cache mutex poisoned") =
            Some(CachedBm25 { embedder, scorer });
        *self.bm25_dirty.lock().expect("bm25_dirty mutex poisoned") = false;
    }

    pub async fn search_bm25(&self, query: &str, top_k: usize) -> Vec<(Uuid, f32)> {
        let corpus = self.bm25_corpus.read().await;
        if corpus.is_empty() {
            return vec![];
        }

        let dirty = *self.bm25_dirty.lock().expect("bm25_dirty mutex poisoned");
        if dirty
            || self
                .bm25_cache
                .lock()
                .expect("bm25_cache mutex poisoned")
                .is_none()
        {
            self.rebuild_bm25_cache(&corpus);
        }
        drop(corpus);

        let guard = self.bm25_cache.lock().expect("bm25_cache mutex poisoned");
        let cached = match guard.as_ref() {
            Some(c) => c,
            None => return vec![],
        };

        let query_embedding = cached.embedder.embed(query);
        let matches = cached.scorer.matches(&query_embedding);

        matches
            .into_iter()
            .take(top_k)
            .map(|m| (m.id, m.score))
            .collect()
    }

    /// Boost energy for memories that made it into the final top-k retrieval results.
    ///
    /// This is the "access boost" part of the Ebbinghaus energy model — memories
    /// that appear in retrieval results are considered recently used and get their
    /// energy increased so they don't decay away prematurely.
    #[cfg(feature = "postgres-storage")]
    async fn boost_energy_for_retrieval(pool: &sqlx::PgPool, result_ids: &[Uuid], boost: i32) {
        use crate::memory::dream::energy_decay::EnergyDecayWorker;
        let worker = EnergyDecayWorker::with_defaults(pool);
        for id in result_ids {
            if let Err(e) = worker.boost_energy(*id, boost).await {
                tracing::warn!(memory_id = %id, "failed to boost energy: {}", e);
            }
        }
    }

    /// Apply temporal recency boost to close-scoring results.
    ///
    /// When `recency_boost` is set, nodes whose semantic/RRF scores are within
    /// `recency_boost * 0.5` of the max score receive a recency bonus.
    /// The bonus is proportional to how recent each node is relative to
    /// the newest node in the result set. Results are re-sorted after boosting.
    pub async fn hybrid_retrieve<'a>(
        &self,
        query_text: Option<&str>,
        query_vector: &[f32],
        top_k: usize,
        max_depth: usize,
        _recency_boost: Option<f32>,
        fusion_strategy: Option<FusionStrategy>,
        query_type_routing: bool,
        #[cfg(feature = "postgres-storage")] trajectory_store: Option<
            &'a crate::storage::TrajectoryStore<'_>,
        >,
    ) -> Vec<(f32, FractalNode)> {
        #[cfg(feature = "postgres-storage")]
        let start = Instant::now();

        #[cfg(feature = "postgres-storage")]
        let mut trajectory = trajectory_store.map(|_ts| {
            crate::storage::RetrievalTrajectory::new(
                query_text.unwrap_or("").to_string(),
                query_vector.to_vec(),
            )
        });

        let vector_results = self
            .retrieve_fractal(
                query_vector,
                top_k * 2,
                max_depth,
                #[cfg(feature = "postgres-storage")]
                crate::memory::fractal_node::FractalNode::ZOOM_PRUNING_THRESHOLD,
                #[cfg(feature = "postgres-storage")]
                trajectory
                    .as_mut()
                    .map(|t| t as &mut crate::storage::trajectory::RetrievalTrajectory),
            )
            .await;

        // Note: trajectory logging is now done inside retrieve_fractal

        let vector_ids: Vec<Uuid> = vector_results.iter().map(|n| n.id).collect();
        tracing::debug!(count = vector_ids.len(), "hybrid: vector candidates");

        let bm25_results = match query_text {
            Some(q) if !q.is_empty() => {
                let r = self.search_bm25(q, top_k * 2).await;
                tracing::debug!(count = r.len(), "hybrid: bm25 candidates");
                r
            }
            _ => vec![],
        };

        if bm25_results.is_empty() {
            #[cfg(feature = "postgres-storage")]
            let total_candidates = vector_results.len();
            let results: Vec<_> = vector_results
                .into_iter()
                .take(top_k)
                .filter_map(|n| {
                    let sim = crate::memory::cosine_similarity(&n.vector, query_vector);
                    Some((sim, n))
                })
                .collect();
            #[cfg(feature = "postgres-storage")]
            {
                let execution_time_ms = start.elapsed().as_millis() as u64;
                for (i, (score, node)) in results.iter().enumerate() {
                    if let Some(ref mut traj) = trajectory.as_mut() {
                        traj.log_search(node.id, *score, "final result (vector only)");
                    }
                }
                if let (Some(ref mut traj), Some(ts)) = (trajectory.as_mut(), trajectory_store) {
                    traj.execution_time_ms = execution_time_ms;
                    traj.total_candidates = total_candidates;
                    traj.retrieved_count = results.len();
                    traj.max_depth_used = max_depth;
                    if let Err(e) = ts.log_retrieval(traj).await {
                        tracing::warn!("failed to log retrieval trajectory: {e}");
                    }
                }
                // Boost energy for memories that made it into top-k (Ebbinghaus access boost)
                if let Some(ts) = trajectory_store {
                    let top_k_ids: Vec<Uuid> = results.iter().map(|(_, n)| n.id).collect();
                    Self::boost_energy_for_retrieval(ts.pool(), &top_k_ids, 20).await;
                }
            }
            return results;
        }

        #[cfg(feature = "postgres-storage")]
        let total_candidates = vector_ids.len() + bm25_results.len();

        // Build dense candidates with cosine similarity scores
        use crate::retrieval::hybrid::{hybrid_retrieve as hybrid_fuse, DenseCandidate};
        let dense_candidates: Vec<DenseCandidate> = vector_results
            .iter()
            .map(|n| {
                let sim = crate::memory::cosine_similarity(&n.vector, query_vector);
                DenseCandidate::new(n.id, sim, n.vector.clone())
            })
            .collect();

        let fused = hybrid_fuse(
            query_text,
            Some(query_vector),
            &bm25_results,
            &dense_candidates,
            top_k,
            fusion_strategy,
            query_type_routing,
        );

        let nodes = self.nodes.read().await;
        let results: Vec<_> = fused
            .into_iter()
            .filter_map(|fr| nodes.get(&fr.id).cloned().map(|n| (fr.score, n)))
            .collect();

        #[cfg(feature = "postgres-storage")]
        {
            let execution_time_ms = start.elapsed().as_millis() as u64;
            for (i, (score, node)) in results.iter().enumerate() {
                if let Some(ref mut traj) = trajectory.as_mut() {
                    traj.log_search(node.id, *score, "final result (fused)");
                }
            }
            if let (Some(ref mut traj), Some(ts)) = (trajectory.as_mut(), trajectory_store) {
                traj.execution_time_ms = execution_time_ms;
                traj.total_candidates = total_candidates;
                traj.retrieved_count = results.len();
                traj.max_depth_used = max_depth;
                if let Err(e) = ts.log_retrieval(traj).await {
                    tracing::warn!("failed to log retrieval trajectory: {e}");
                }
            }
        }

        // Boost energy for memories in top-k (Ebbinghaus access boost)
        #[cfg(feature = "postgres-storage")]
        if let Some(ts) = trajectory_store {
            let top_k_ids: Vec<Uuid> = results.iter().map(|(_, n)| n.id).collect();
            Self::boost_energy_for_retrieval(ts.pool(), &top_k_ids, 20).await;
        }

        results
    }

    pub async fn retrieve_fractal(
        &self,
        query_vector: &[f32],
        top_k: usize,
        max_depth: usize,
        #[cfg(feature = "postgres-storage")] pruning_threshold: f32,
        #[cfg(feature = "postgres-storage")] trajectory: Option<
            &mut crate::storage::trajectory::RetrievalTrajectory,
        >,
    ) -> Vec<FractalNode> {
        #[cfg(not(feature = "postgres-storage"))]
        let pruning_threshold = crate::memory::fractal_node::FractalNode::ZOOM_PRUNING_THRESHOLD;

        let node_count = self.nodes.read().await.len();
        let has_index = self
            .usearch_index
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false);

        if node_count >= USEARCH_THRESHOLD && has_index && !query_vector.is_empty() {
            // ── 3-Level Cascade (TST-inspired): 64d → 256d → 768d ──
            // Step 1: Ultra-coarse 64d filter — cheap, eliminates ~95% of space
            let mut candidate_uuids: Vec<Uuid> = if query_vector.len() >= ULTRA_COARSE_DIM {
                let ultra_vec = &query_vector[..ULTRA_COARSE_DIM];
                let ultra = self.ultra_coarse_search(ultra_vec, top_k * 8).await;
                if ultra.is_empty() {
                    vec![] // fall through to main index
                } else {
                    ultra
                }
            } else {
                vec![]
            };

            // Step 2: Coarse 256d filter — narrow within ultra candidates
            if !candidate_uuids.is_empty() && query_vector.len() >= COARSE_DIM {
                let coarse_vec = &query_vector[..COARSE_DIM];
                let coarse_candidates = self.coarse_search(coarse_vec, top_k * 4).await;
                if !coarse_candidates.is_empty() {
                    let coarse_set: std::collections::HashSet<Uuid> =
                        coarse_candidates.into_iter().collect();
                    candidate_uuids.retain(|uid| coarse_set.contains(uid));
                }
            }

            // Step 3: Precision zoom_retrieve within filtered candidate set
            if !candidate_uuids.is_empty() {
                let nodes = self.nodes.read().await;
                let owned_nodes: Vec<FractalNode> = candidate_uuids
                    .iter()
                    .filter_map(|uid| nodes.get(uid).cloned())
                    .collect();
                drop(nodes);

                let mut scored: Vec<(f32, FractalNode)> = Vec::new();
                for node in &owned_nodes {
                    let results = node.zoom_retrieve(query_vector, max_depth, pruning_threshold);
                    scored.extend(results.into_iter().map(|(s, n)| (s, n.clone())));
                }
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));

                #[cfg(feature = "postgres-storage")]
                if let Some(traj) = trajectory {
                    for (score, node) in &scored {
                        traj.log_search(node.id, *score, "cascade_candidate");
                    }
                }

                return scored.into_iter().take(top_k).map(|(_, n)| n).collect();
            }

            // Fallback: main 768d index when cascade produced no candidates
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
                let fallback_uuids: Vec<Uuid> = candidate_keys
                    .iter()
                    .filter_map(|k| k2u.get(k).copied())
                    .collect();
                drop(k2u);

                let nodes = self.nodes.read().await;
                let owned_nodes: Vec<FractalNode> = fallback_uuids
                    .iter()
                    .filter_map(|uid| nodes.get(uid).cloned())
                    .collect();
                drop(nodes);

                let mut scored: Vec<(f32, FractalNode)> = Vec::new();
                for node in &owned_nodes {
                    let results = node.zoom_retrieve(query_vector, max_depth, pruning_threshold);
                    scored.extend(results.into_iter().map(|(s, n)| (s, n.clone())));
                }
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));

                #[cfg(feature = "postgres-storage")]
                if let Some(traj) = trajectory {
                    for (score, node) in &scored {
                        traj.log_search(node.id, *score, "usearch_candidate");
                    }
                }

                return scored.into_iter().take(top_k).map(|(_, n)| n).collect();
            }
        }

        // Fallback: linear scan
        #[cfg(feature = "postgres-storage")]
        if let Some(traj) = trajectory {
            traj.log_info("fallback: linear scan");
        }

        let nodes = self.nodes.read().await;
        let mut scored: Vec<(f32, FractalNode)> = nodes
            .values()
            .filter_map(|node| {
                let results = node.zoom_retrieve(query_vector, max_depth, pruning_threshold);
                if results.is_empty() {
                    None
                } else {
                    Some(
                        results
                            .into_iter()
                            .map(move |(s, ref_node)| (s, ref_node.clone())),
                    )
                }
            })
            .flatten()
            .collect();
        drop(nodes);
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        scored.into_iter().take(top_k).map(|(_, n)| n).collect()
    }

    /// In-memory Matryoshka search: rank nodes by truncated cosine similarity.
    fn search_by_truncated_vector(
        all_nodes: &HashMap<Uuid, FractalNode>,
        query: &[f32],
        trunc_dim: usize,
        limit: usize,
    ) -> Vec<FractalNode> {
        use crate::memory::fractal_node::{cosine_similarity, truncate_vector};

        let mut scored: Vec<(f32, FractalNode)> = all_nodes
            .values()
            .filter_map(|node| {
                let trunc = truncate_vector(&node.vector, trunc_dim)?;
                let sim = cosine_similarity(query, &trunc);
                Some((sim, node.clone()))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        scored.into_iter().take(limit).map(|(_, n)| n).collect()
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Matryoshka expand_fractal Unit Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(
    deprecated,
    reason = "tests intentionally exercise legacy FractalNode::new_session constructor"
)]
mod expand_fractal_tests {
    use super::*;
    use crate::memory::fractal_node::FractalNode;
    use crate::memory::types::{MemorySource, MemoryType};
    use crate::storage::{ScoredNode, StorageBackend};

    fn vec768(first: f32, second: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 768];
        v[0] = first;
        v[1] = second;
        v
    }

    #[tokio::test]
    async fn expand_fractal_max_depth_zero_returns_unchanged() {
        let store = MemoryStore::new();
        let node = FractalNode::new_typed(
            Some("seed".into()),
            None,
            vec768(1.0, 0.0),
            Default::default(),
            MemoryType::Semantic,
            MemorySource::Conversation,
        );
        let seed = ScoredNode {
            id: node.id,
            score: 0.9,
            distribution_scores: None,
            debug: None,
            node: node.clone(),
        };

        let result = store
            .expand_fractal(vec![seed.clone()], &vec768(1.0, 0.0), 0, 0.5)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, seed.id);
    }

    #[tokio::test]
    async fn expand_fractal_empty_query_returns_unchanged() {
        let store = MemoryStore::new();
        let node = FractalNode::new_typed(
            Some("seed".into()),
            None,
            vec768(1.0, 0.0),
            Default::default(),
            MemoryType::Semantic,
            MemorySource::Conversation,
        );
        let seed = ScoredNode {
            id: node.id,
            score: 0.9,
            distribution_scores: None,
            debug: None,
            node,
        };

        let result = store
            .expand_fractal(vec![seed.clone()], &[], 2, 0.5)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, seed.id);
    }

    #[tokio::test]
    async fn expand_fractal_finds_cluster_neighbors_at_256d() {
        let store = MemoryStore::new();

        let seed_node = FractalNode::new_typed(
            Some("seed".into()),
            None,
            vec768(1.0, 0.0),
            Default::default(),
            MemoryType::Semantic,
            MemorySource::Conversation,
        );
        let seed_id = seed_node.id;

        let mut neighbor = FractalNode::new_typed(
            Some("neighbor".into()),
            None,
            vec768(0.99, 0.01),
            Default::default(),
            MemoryType::Semantic,
            MemorySource::Conversation,
        );
        neighbor.id = uuid::Uuid::new_v4();

        let mut distant = FractalNode::new_typed(
            Some("distant".into()),
            None,
            vec768(0.0, 1.0),
            Default::default(),
            MemoryType::Semantic,
            MemorySource::Conversation,
        );
        distant.id = uuid::Uuid::new_v4();

        store.insert(seed_node.clone()).await.unwrap();
        store.insert(neighbor.clone()).await.unwrap();
        store.insert(distant).await.unwrap();

        let seed = ScoredNode {
            id: seed_id,
            score: 0.95,
            distribution_scores: None,
            debug: None,
            node: seed_node,
        };

        let expanded = store
            .expand_fractal(vec![seed], &vec768(1.0, 0.0), 1, 0.5)
            .await
            .unwrap();

        let ids: HashSet<_> = expanded.iter().map(|s| s.id).collect();
        assert!(ids.contains(&seed_id), "seed missing: {:?}", ids);
        assert!(
            ids.contains(&neighbor.id),
            "256d cluster neighbor missing: {:?}",
            ids
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Temporal Scoring Unit Tests — Multi Half-Life & Weighting Strategies
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(
    deprecated,
    reason = "tests intentionally exercise legacy FractalNode::new_session constructor"
)]
mod temporal_scoring_tests {
    use super::*;
    use crate::memory::FractalNode;
    use std::collections::HashMap;

    /// Apply hybrid temporal scoring to a vector of (score, age_days) pairs.
    /// Returns new scores after blending with recency.
    fn apply_temporal_scoring(
        mut scores_and_ages: Vec<(f32, f32)>, // (semantic_score, age_days)
        w: f32,
        half_life_days: f32,
    ) -> Vec<f32> {
        for (score, age_days) in &mut scores_and_ages {
            let rf = shared::recency_factor(*age_days, half_life_days);
            *score = *score * (1.0 - w) + rf * w;
        }
        scores_and_ages.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scores_and_ages.iter().map(|(s, _)| *s).collect()
    }

    // ── Core Formula Tests ──

    // ── Multi Half-Life Tests ──

    #[test]
    fn half_life_3d_decays_faster_than_7d() {
        // At 3 days: 3d half-life gives 0.5, 7d half-life gives ~0.743
        let rf_3 = shared::recency_factor(3.0, 3.0);
        let rf_7 = shared::recency_factor(3.0, 7.0);
        assert!(
            rf_3 < rf_7,
            "3d half-life ({rf_3}) should decay faster than 7d ({rf_7})"
        );
        assert!((rf_3 - 0.5).abs() < 0.01);
        assert!(rf_7 > 0.7);
    }

    #[test]
    fn half_life_30d_decays_slower_than_7d() {
        // At 7 days: 7d half-life gives 0.5, 30d half-life gives ~0.851
        let rf_7 = shared::recency_factor(7.0, 7.0);
        let rf_30 = shared::recency_factor(7.0, 30.0);
        assert!(
            rf_30 > rf_7,
            "30d half-life ({rf_30}) should decay slower than 7d ({rf_7})"
        );
        assert!((rf_7 - 0.5).abs() < 0.01);
        assert!(rf_30 > 0.8);
    }

    #[test]
    fn half_life_60d_preserves_much_more_recency() {
        // At 14 days: 7d gives 0.25, 60d gives ~0.851
        let rf_7 = shared::recency_factor(14.0, 7.0);
        let rf_60 = shared::recency_factor(14.0, 60.0);
        assert!((rf_7 - 0.25).abs() < 0.01);
        assert!(
            rf_60 > 0.8,
            "60d half-life should give high recency ({rf_60}) at 14 days"
        );
        assert!(
            rf_60 > rf_7 * 3.0,
            "60d should be >3x higher than 7d at 14 days"
        );
    }

    #[test]
    fn recency_range_wider_with_shorter_half_life() {
        // Shorter half-life → wider range between newest and oldest
        let rf_new = shared::recency_factor(0.0, 3.0);
        let rf_old = shared::recency_factor(10.0, 3.0); // ~0.05 floor
        let range_3 = rf_new - rf_old;

        let rf_new = shared::recency_factor(0.0, 30.0);
        let rf_old = shared::recency_factor(10.0, 30.0);
        let range_30 = rf_new - rf_old;

        assert!(
            range_3 > range_30,
            "3d half-life range ({range_3:.4}) should be wider than 30d ({range_30:.4})"
        );
    }

    // ── Weighting Strategy Tests ──

    #[test]
    fn weight_zero_is_pure_semantic() {
        // w=0: recency has no effect, scores unchanged
        let input = vec![(0.9, 0.0), (0.8, 100.0), (0.5, 1.0)];
        let result = apply_temporal_scoring(input.clone(), 0.0, 7.0);
        // Scores should remain in same relative order (sorted descending)
        assert!((result[0] - 0.9).abs() < 0.001);
        assert!((result[1] - 0.8).abs() < 0.001);
        assert!((result[2] - 0.5).abs() < 0.001);
    }

    #[test]
    fn weight_full_is_pure_recency() {
        // w=0.8 (max): dominated by recency, newest wins regardless of semantic
        let input = vec![(0.9, 100.0), (0.1, 0.0), (0.5, 50.0)];
        let result = apply_temporal_scoring(input, 0.8, 7.0);
        // Brand-new (0d) item should now be top despite low semantic score
        assert!(
            (result[0] - 0.8).abs() < 0.1,
            "New item (0d) should rank first with w=0.8. Got result: {:?}",
            result
        );
    }

    #[test]
    fn weight_moderate_balances_semantic_and_recency() {
        // w=0.3: balanced — very old relevant might still beat new irrelevant
        let input = vec![(0.9, 100.0), (0.4, 0.0)];
        let result = apply_temporal_scoring(input, 0.3, 7.0);
        // Old but highly relevant should still win
        let old_score = 0.9 * 0.7 + 0.05 * 0.3; // 0.63 + 0.015 = 0.645
        let new_score = 0.4 * 0.7 + 1.0 * 0.3; // 0.28 + 0.3 = 0.58
        assert!(
            old_score > new_score,
            "Old relevant should beat new irrelevant at w=0.3"
        );
        assert!((result[0] - old_score).abs() < 0.01);
    }

    #[test]
    fn weight_crossing_point() {
        // As weight increases, a crossover happens where recency overwhelms semantics
        // Find the crossing point between (0.8 score, 10d old) and (0.3 score, 1d old)
        // At w=0: 0.8 > 0.3 → older wins
        // At w=0.8: 0.8*(1-w)+0.05*w vs 0.3*(1-w)+0.5^(1/7)*w
        //           w=0: 0.8 vs 0.3 → older wins
        //           w=0.8: 0.16+0.04=0.20 vs 0.06+0.724=0.784 → newer wins
        let w_cross = 0.2;
        let input = vec![(0.8, 10.0), (0.3, 1.0)];
        let result = apply_temporal_scoring(input, w_cross, 7.0);
        // At low weight, older high-quality should still win
        let old_score = 0.8 * 0.8 + shared::recency_factor(10.0, 7.0) * 0.2;
        let new_score = 0.3 * 0.8 + shared::recency_factor(1.0, 7.0) * 0.2;
        assert!(
            old_score > new_score,
            "At w=0.2, old relevant ({old_score:.4}) should beat new weak ({new_score:.4})"
        );
        assert!((result[0] - old_score).abs() < 0.01);
    }

    // ── Edge Case Tests ──

    #[test]
    fn all_same_age_preserves_semantic_order() {
        // When all items have same age, temporal scoring doesn't reorder
        let input = vec![(0.9, 5.0), (0.7, 5.0), (0.5, 5.0)];
        let result = apply_temporal_scoring(input.clone(), 0.5, 7.0);
        assert!(result[0] > result[1]);
        assert!(result[1] > result[2]);
    }

    #[test]
    fn temporal_scoring_with_small_result_set() {
        // Single item: score changes but no reordering needed
        let input = vec![(0.5, 3.0)];
        let result = apply_temporal_scoring(input, 0.5, 7.0);
        let rf = shared::recency_factor(3.0, 7.0);
        let expected = 0.5 * 0.5 + rf * 0.5;
        assert!((result[0] - expected).abs() < 0.01);
    }

    #[test]
    fn temporal_scoring_idempotent_at_zero_weight() {
        // Running temporal scoring with w=0 twice should give same result
        let input = vec![(0.9, 1.0), (0.8, 5.0), (0.5, 10.0)];
        let r1 = apply_temporal_scoring(input.clone(), 0.0, 7.0);
        let r2 = apply_temporal_scoring(input, 0.0, 7.0);
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert!((a - b).abs() < 0.001);
        }
    }

    // ── Per-Memory-Type Half-Life Tests (proposed improvement #6.1) ──

    #[test]
    fn episodic_3d_vs_semantic_30d_differentiation() {
        // Episodic memory (3d half-life) and Semantic memory (30d half-life)
        // at age=7 days should have very different recency factors
        let epi_rf = shared::recency_factor(7.0, 3.0); // 7d old episodic
        let sem_rf = shared::recency_factor(7.0, 30.0); // 7d old semantic
        assert!(
            epi_rf < 0.2,
            "Episodic at 7d should be nearly fully decayed, got {epi_rf:.4}"
        );
        assert!(
            sem_rf > 0.8,
            "Semantic at 7d should still be strong, got {sem_rf:.4}"
        );
        assert!(
            sem_rf > epi_rf * 4.0,
            "Semantic ({sem_rf:.4}) should be >> Episodic ({epi_rf:.4}) at 7 days"
        );
    }

    #[test]
    fn procedural_60d_stays_strong_for_weeks() {
        // Procedural memory (60d half-life) should retain high recency for weeks
        let rf_14 = shared::recency_factor(14.0, 60.0);
        let rf_30 = shared::recency_factor(30.0, 60.0);
        assert!(
            rf_14 > 0.85,
            "Procedural at 14d should be >0.85, got {rf_14:.4}"
        );
        assert!(
            rf_30 > 0.7,
            "Procedural at 30d should still be >0.7, got {rf_30:.4}"
        );
    }

    #[test]
    fn per_type_half_life_ranking() {
        // At 14 days, different types produce different recency factors
        // Episodic (3d): nearly floor → Procedural (60d): still very strong
        let episodic = shared::recency_factor(14.0, 3.0);
        let semantic = shared::recency_factor(14.0, 30.0);
        let procedural = shared::recency_factor(14.0, 60.0);
        assert!(episodic < semantic);
        assert!(semantic < procedural);
    }

    #[test]
    fn preference_7d_balanced_decay() {
        // Preference (7d half-life) should be between Episodic and Semantic
        let pref_1d = shared::recency_factor(1.0, 7.0);
        let pref_14d = shared::recency_factor(14.0, 7.0);
        assert!(pref_1d > 0.9, "1d preference should be strong");
        assert!(
            (pref_14d - 0.25).abs() < 0.01,
            "14d preference should be 0.25"
        );
    }

    // ── In-Memory Store Integration Test ──

    #[tokio::test]
    async fn test_inmemory_temporal_scoring_with_different_ages() {
        let store = MemoryStore::new();

        // Create nodes with different ages
        let n1 = FractalNode::new_session(
            "very recent important info".to_string(),
            vec![0.9, 0.1, 0.0],
            HashMap::new(),
        );
        let n2 = FractalNode::new_session(
            "week old important info".to_string(),
            vec![0.85, 0.15, 0.0],
            HashMap::new(),
        );
        let n3 = FractalNode::new_session(
            "month old outdated info".to_string(),
            vec![0.3, 0.7, 0.0],
            HashMap::new(),
        );

        store.insert(n1).await.unwrap();
        store.insert(n2).await.unwrap();
        store.insert(n3).await.unwrap();

        // Retrieve without temporal weight
        let query = vec![0.9, 0.1, 0.0];
        #[cfg(feature = "postgres-storage")]
        let results = store.retrieve_fractal(&query, 5, 0, 0.5, None).await;
        #[cfg(not(feature = "postgres-storage"))]
        let results = store.retrieve_fractal(&query, 5, 0).await;
        assert!(!results.is_empty(), "Should return results");

        // Verify the store functions correctly
        assert_eq!(store.count().await, 3);
    }
}

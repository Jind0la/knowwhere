//! Storage backend abstraction.
//!
//! Defines a backend-agnostic interface for KnowWhere's memory storage.
//! All storage backends (MemoryStore, PostgresStore, etc.) implement this trait.
use uuid::Uuid;
use crate::memory::FractalNode;
use crate::memory::types::MemoryStatus;

/// Query parameters for hybrid retrieval (vector + BM25 combined search).
#[derive(Debug, Clone)]
pub struct HybridQuery {
    /// Text query for BM25 keyword search (optional).
    pub query_text: Option<String>,
    /// Dense vector for semantic search (optional — if absent, uses query_text only).
    pub query_vector: Option<Vec<f32>>,
    /// Maximum number of results to return.
    pub top_k: usize,
    /// Maximum fractal zoom depth (0 = top-level only).
    pub max_depth: usize,
}

impl HybridQuery {
    /// Create a text-only query (BM25 only, no vector search).
    pub fn text(text: impl Into<String>, top_k: usize) -> Self {
        Self {
            query_text: Some(text.into()),
            query_vector: None,
            top_k,
            max_depth: 0,
        }
    }

    /// Create a vector-only query (semantic search only, no BM25).
    pub fn vector(vector: Vec<f32>, top_k: usize, max_depth: usize) -> Self {
        Self {
            query_text: None,
            query_vector: Some(vector),
            top_k,
            max_depth,
        }
    }

    /// Create a hybrid query (BM25 + vector, combined via RRF).
    pub fn hybrid(text: impl Into<String>, vector: Vec<f32>, top_k: usize, max_depth: usize) -> Self {
        Self {
            query_text: Some(text.into()),
            query_vector: Some(vector),
            top_k,
            max_depth,
        }
    }
}

/// Operations for updating a node — used by DreamMode, ConsolidationScheduler,
/// and AuditScheduler. This enum-based approach is dyn Trait-compatible
/// (unlike closure-based update_node which doesn't work with Arc<dyn StorageBackend>).
#[derive(Debug, Clone)]
pub enum UpdateOperation {
    /// Multiply the node's weight by a factor (e.g., weight *= 0.95).
    MultiplyWeight(f64),
    /// Set the weight directly (used by AuditScheduler).
    SetWeight(f64),
    /// Set parent_tier_id if not already set (used by ConsolidationScheduler as pending marker).
    SetParentTierId(Uuid),
    /// Set the node status (used by AuditScheduler).
    SetStatus(MemoryStatus),
    /// Composite operation: set weight + optionally status (used by AuditScheduler).
    /// This must be atomic — both changes happen together.
    ApplyAudit { weight: f64, status: Option<MemoryStatus> },
}

impl UpdateOperation {
    /// Apply this operation to a FractalNode in-place.
    pub fn apply(&self, node: &mut FractalNode) {
        match self {
            UpdateOperation::MultiplyWeight(factor) => {
                node.weight *= factor;
            }
            UpdateOperation::SetWeight(w) => {
                node.weight = *w;
            }
            UpdateOperation::SetParentTierId(id) => {
                if node.parent_tier_id.is_none() {
                    node.parent_tier_id = Some(*id);
                }
            }
            UpdateOperation::SetStatus(status) => {
                node.status = status.clone();
            }
            UpdateOperation::ApplyAudit { weight, status } => {
                node.weight = *weight;
                if let Some(s) = status {
                    node.status = s.clone();
                }
            }
        }
    }
}

/// A scored retrieval result from a storage backend.
#[derive(Debug, Clone)]
pub struct ScoredNode {
    pub id: Uuid,
    pub score: f32,
    pub node: FractalNode,
}

impl ScoredNode {
    /// Convert from a (score, node) tuple.
    pub fn from_tuple((score, node): (f32, FractalNode)) -> Self {
        Self {
            id: node.id,
            score,
            node,
        }
    }

    /// Convert from a raw FractalNode (score defaults to 1.0).
    pub fn from_node(node: FractalNode) -> Self {
        Self {
            id: node.id,
            score: 1.0,
            node,
        }
    }
}

/// Core storage operations every backend must implement.
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    // --- CRUD ---

    /// Insert a new memory node. Returns the assigned UUID.
    async fn insert(&self, node: FractalNode) -> anyhow::Result<Uuid>;

    /// Insert multiple nodes concurrently. Returns their assigned UUIDs.
    async fn insert_many(&self, nodes: Vec<FractalNode>) -> anyhow::Result<Vec<Uuid>> {
        use futures::future::try_join_all;
        let ids: Vec<_> = nodes.into_iter().map(|n| self.insert(n)).collect();
        try_join_all(ids).await
    }

    /// Retrieve a node by ID.
    async fn get(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>>;

    /// Delete a node by ID. Returns true if a node was deleted.
    async fn delete(&self, id: &Uuid) -> anyhow::Result<bool>;

    /// Update a node's vector embedding.
    async fn update_vector(&self, id: &Uuid, new_vector: Vec<f32>) -> anyhow::Result<bool>;

    /// Apply an UpdateOperation to a node (dyn Trait-compatible alternative to closure-based updates).
    async fn update(&self, id: &Uuid, op: UpdateOperation) -> anyhow::Result<()>;

    // --- Query ---

    /// Hybrid retrieval: combines vector similarity + BM25 keyword search via RRF.
    ///
    /// - `query_text` + `query_vector`: full hybrid search (RRF fusion)
    /// - `query_vector` only: pure vector similarity search
    /// - `query_text` only: pure BM25 keyword search
    async fn hybrid_retrieve(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>>;

    /// Recursive fractal zoom retrieval — explores children above similarity threshold.
    async fn retrieve_fractal(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>>;

    /// Standalone BM25 keyword search (no vector component).
    async fn search_bm25(&self, query_text: &str, top_k: usize) -> anyhow::Result<Vec<(Uuid, f32)>>;

    // --- Enumeration ---

    /// List all nodes.
    async fn list_all(&self) -> anyhow::Result<Vec<FractalNode>>;

    /// Return the most recently accessed nodes.
    async fn recent(&self, limit: usize) -> anyhow::Result<Vec<FractalNode>>;

    /// Total count of stored nodes.
    async fn count(&self) -> usize;

    // --- Maintenance ---

    /// Remove nodes that have placeholder/dummy vectors (vector is all zeros or near-zero).
    /// Returns the number of nodes removed.
    async fn purge_dummy_vectors(&self) -> usize;
}

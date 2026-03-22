//! Storage backend abstraction.
//!
//! Defines a backend-agnostic interface for KnowWhere's memory storage.
//! All storage backends (MemoryStore, PostgresStore, etc.) implement this trait.
use std::sync::Arc;
use uuid::Uuid;
use crate::memory::FractalNode;

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

/// A scored retrieval result.
#[derive(Debug, Clone)]
pub struct ScoredNode {
    pub id: Uuid,
    pub score: f32,
    pub node: FractalNode,
}

/// Core storage operations every backend must implement.
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    // --- CRUD ---

    /// Insert a new memory node. Returns the assigned UUID.
    async fn insert(&self, node: FractalNode) -> anyhow::Result<Uuid>;

    /// Retrieve a node by ID.
    async fn get(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>>;

    /// Delete a node by ID. Returns true if a node was deleted.
    async fn delete(&self, id: &Uuid) -> anyhow::Result<bool>;

    /// Update a node's vector embedding.
    async fn update_vector(&self, id: &Uuid, new_vector: Vec<f32>) -> anyhow::Result<bool>;

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
}

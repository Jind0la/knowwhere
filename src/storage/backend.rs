//! Storage backend abstraction.
//!
//! Defines a backend-agnostic interface for KnowWhere's memory storage.
//! All storage backends (MemoryStore, PostgresStore, etc.) implement this trait.
use crate::embedding::EmbeddingProvider;
use crate::memory::types::MemoryStatus;
use crate::memory::FractalNode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Query parameters for hybrid retrieval (vector + BM25 combined search).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RetrievalProfile {
    #[default]
    UserFacing,
    AgentDebug,
    FullFidelity,
}

/// Fusion strategy for combining BM25 and dense vector scores.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FusionStrategy {
    /// Weighted linear combination: final = bm25_weight * norm_bm25 + dense_weight * norm_dense
    WeightedSum {
        bm25_weight: f32,
        dense_weight: f32,
    },
    /// Reciprocal Rank Fusion with configurable k constant (default k=60).
    ReciprocalRankFusion { k: f32 },
    /// Pure BM25 only — skip dense scores entirely.
    Bm25Only,
    /// Pure dense only — skip BM25 entirely.
    DenseOnly,
}

impl Default for FusionStrategy {
    fn default() -> Self {
        FusionStrategy::ReciprocalRankFusion { k: 60.0 }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoreDebug {
    pub profile: RetrievalProfile,
    pub trust_tier: String,
    pub base_score: f32,
    pub multiplier: f32,
    /// New explainable fields for temporal + session hybrid scoring
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_boost: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_weight: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// Source-type weighting information for provenance-aware scoring.
    /// Human-readable composite: "synthetic (0.85x)".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    /// The multiplier applied based on the source type classification.
    /// e.g., 0.85 for synthetic, 1.0 for real.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_weight_applied: Option<f32>,
    /// The original source classification (e.g., "real", "synthetic", "derived", "unknown").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_source: Option<String>,
    /// Ebbinghaus forgetting curve factor applied during retrieval scoring.
    /// R(m,t) ∈ (0.0, 1.0] — 1.0 = just reviewed, decays toward 0.0 over time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ebbinghaus_factor: Option<f32>,
}

impl ScoreDebug {
    pub fn final_score(&self) -> f32 {
        self.base_score * self.multiplier
    }
}

impl RetrievalProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            RetrievalProfile::UserFacing => "user-facing",
            RetrievalProfile::AgentDebug => "agent-debug",
            RetrievalProfile::FullFidelity => "full-fidelity",
        }
    }

    pub fn fetch_k(self, top_k: usize) -> usize {
        match self {
            RetrievalProfile::FullFidelity => top_k,
            RetrievalProfile::UserFacing | RetrievalProfile::AgentDebug => top_k.saturating_mul(3),
        }
    }

    pub fn allows(self, node: &FractalNode) -> bool {
        !matches!(self, RetrievalProfile::UserFacing) || !node.is_internal_only()
    }

    pub fn score_multiplier(
        self,
        node: &FractalNode,
        weights: Option<crate::retrieval::source_weighting::SourceTypeWeights>,
    ) -> f32 {
        // Ebbinghaus Forgetting Curve: temporal decay factor based on last review (r_m)
        // and reinforcement count (n_m). Returns 1.0 at review time, decays toward 0.0.
        let ebbinghaus = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        let w = weights.unwrap_or_default();
        let source = crate::retrieval::source_weighting::source_multiplier(node, &w);
        if matches!(self, RetrievalProfile::FullFidelity) {
            // Reduce-to-Core: neutralize tier/mtype/explicit (policy). Source weighting
            // and Ebbinghaus (temporal fact) remain.
            return ebbinghaus * source;
        }
        let explicit = self.explicit_weight(node);
        let tier = self.tier_multiplier(node.trust_tier());
        let mtype = self.memory_type_multiplier(node);
        tier * explicit * mtype * source * ebbinghaus
    }

    fn memory_type_multiplier(self, node: &FractalNode) -> f32 {
        use crate::memory::types::MemoryType;
        match node.memory_type {
            // Facts and decisions are high-value — boost them in retrieval.
            // 1.5x is intentional: Decision nodes are already Synthetic-sourced
            // (~0.85x penalty), so the net effect vs episodic is ~1.27x, not 1.5x.
            // This keeps facts prominent without drowning out conversation nodes.
            MemoryType::Decision => 1.5,
            // Preferences are valuable but less certain
            MemoryType::Preference => 1.2,
            // Procedural knowledge is high-stakes
            MemoryType::Procedural => 1.15,
            // Semantic knowledge has moderate value
            MemoryType::Semantic => 1.05,
            // Episodic, Meta, MemoryChunk — no boost
            _ => 1.0,
        }
    }

    fn explicit_weight(self, node: &FractalNode) -> f32 {
        if matches!(self, RetrievalProfile::FullFidelity) {
            return 1.0;
        }
        // Priority: explicit trust_weight metadata → node.weight field → default 1.0.
        // Both fact_extraction (inline) and consolidation (L1/L0/claim) set node.weight
        // for retrieval boosting; this fallback ensures those boosts actually flow into scoring.
        node.explicit_trust_weight()
            .or_else(|| {
                if (node.weight - 1.0).abs() > f64::EPSILON {
                    Some(node.weight as f32)
                } else {
                    None
                }
            })
            .unwrap_or(1.0)
            .clamp(0.1, 2.0)
    }

    fn tier_multiplier(self, trust_tier: &str) -> f32 {
        match trust_tier {
            "primary" => 1.3,   // High trust: user statements, decisions, explicit claims
            "reference" => 1.1,  // Imported data, documents, manuals
            "derived" => 0.9,    // System-generated summaries, consolidation output
            "volatile" => 0.7,   // Temporary/uncertain data
            _ => 1.0,
        }
    }

    pub fn score_debug(
        self,
        base_score: f32,
        node: &FractalNode,
        weights: Option<crate::retrieval::source_weighting::SourceTypeWeights>,
    ) -> ScoreDebug {
        let source_type =
            crate::retrieval::source_weighting::detect_source_type(node).to_string();
        let w = weights.unwrap_or_default();
        let source_mult = crate::retrieval::source_weighting::source_multiplier(node, &w);
        ScoreDebug {
            profile: self,
            trust_tier: node.trust_tier().to_string(),
            base_score,
            multiplier: self.score_multiplier(node, Some(w)),
            recency_factor: None,
            session_boost: None,
            temporal_weight: None,
            explanation: None,
            source_type: Some(format!("{source_type} ({source_mult:.2}x)")),
            source_weight_applied: Some(source_mult),
            original_source: Some(source_type),
            ebbinghaus_factor: Some(node.ebbinghaus_decay(chrono::Utc::now()) as f32),
        }
    }

    pub fn score_node(
        self,
        base_score: f32,
        node: FractalNode,
        weights: Option<crate::retrieval::source_weighting::SourceTypeWeights>,
    ) -> ScoredNode {
        let debug = self.score_debug(base_score, &node, weights);
        ScoredNode {
            id: node.id,
            score: debug.final_score(),
            distribution_scores: None,
            debug: Some(debug),
            node,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HybridQuery {
    /// Text query for BM25 keyword search (optional).
    pub query_text: Option<String>,
    /// Dense vector for semantic search (optional — if absent, uses query_text only).
    pub query_vector: Option<Vec<f32>>,
    /// Maximum number of results to return.
    pub top_k: usize,
    /// Maximum fractal zoom depth (0 = top-level only).
    pub max_depth: usize,
    /// Retrieval profile: safe for users, agent debugging, or raw full fidelity.
    pub profile: RetrievalProfile,
    /// Optional filter by memory type (e.g. "decision" for architectural decisions only).
    pub memory_type_filter: Option<crate::memory::types::MemoryType>,
    /// Optional filter by user_id in metadata — scopes retrieval to a single persona.
    pub user_id: Option<String>,
    /// Enable multi-query expansion (2-3 reformulations, RRF-fused).
    pub multi_query: bool,
    /// Temporal recency boost factor (0.0–0.20).
    /// When set, nodes with scores close to each other (within recency_boost * 0.5)
    /// receive a recency bonus proportional to how recent they are relative to
    /// the newest node in the result set.
    pub recency_boost: Option<f32>,
    /// Weight for temporal recency in hybrid scoring (0.0 = pure semantic, 1.0 = pure recency).
    /// Recommended: 0.15–0.35 for balanced temporal + semantic retrieval.
    /// When set, applies a global recency factor (exponential decay) combined with semantic score.
    pub temporal_weight: Option<f32>,
    /// Explicit fusion strategy. When set, overrides auto-routing.
    /// Default: ReciprocalRankFusion { k: 60.0 } (backward-compatible).
    #[serde(skip)]
    pub fusion_strategy: Option<FusionStrategy>,
    /// Enable automatic query-type routing (keyword vs. semantic vs. hybrid).
    /// When true AND no explicit fusion_strategy is set, the query text is analyzed
    /// to choose the optimal strategy. When false, falls back to default RRF.
    #[serde(skip)]
    pub query_type_routing: bool,
    /// Optional per-source-type score multipliers for provenance-aware retrieval.
    /// When None, uses the default SourceTypeWeights (real=1.0, synthetic=0.85, derived=0.70, unknown=0.95).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type_weights: Option<crate::retrieval::source_weighting::SourceTypeWeights>,
}

impl HybridQuery {
    /// Create a text-only query (BM25 only, no vector search).
    pub fn text(text: impl Into<String>, top_k: usize) -> Self {
        Self {
            query_text: Some(text.into()),
            query_vector: None,
            top_k,
            max_depth: 0,
            profile: RetrievalProfile::FullFidelity,
            memory_type_filter: None,
            user_id: None,
            multi_query: false,
            recency_boost: None,
            temporal_weight: None,
            fusion_strategy: Some(FusionStrategy::Bm25Only),
            query_type_routing: false,
            source_type_weights: None,
        }
    }

    /// Create a vector-only query (semantic search only, no BM25).
    pub fn vector(vector: Vec<f32>, top_k: usize, max_depth: usize) -> Self {
        Self {
            query_text: None,
            query_vector: Some(vector),
            top_k,
            max_depth,
            profile: RetrievalProfile::FullFidelity,
            memory_type_filter: None,
            user_id: None,
            multi_query: false,
            recency_boost: None,
            temporal_weight: None,
            fusion_strategy: Some(FusionStrategy::DenseOnly),
            query_type_routing: false,
            source_type_weights: None,
        }
    }

    /// Create a hybrid query (BM25 + vector, combined via RRF).
    pub fn hybrid(
        text: impl Into<String>,
        vector: Vec<f32>,
        top_k: usize,
        max_depth: usize,
    ) -> Self {
        Self {
            query_text: Some(text.into()),
            query_vector: Some(vector),
            top_k,
            max_depth,
            profile: RetrievalProfile::FullFidelity,
            memory_type_filter: None,
            user_id: None,
            multi_query: false,
            recency_boost: None,
            temporal_weight: None,
            fusion_strategy: None, // default RRF
            query_type_routing: false,
            source_type_weights: None,
        }
    }

    pub fn with_profile(mut self, profile: RetrievalProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn user_facing(mut self) -> Self {
        self.profile = RetrievalProfile::UserFacing;
        self
    }

    pub fn agent_debug(mut self) -> Self {
        self.profile = RetrievalProfile::AgentDebug;
        self
    }

    pub fn full_fidelity(mut self) -> Self {
        self.profile = RetrievalProfile::FullFidelity;
        self
    }

    /// Attach a memory type filter (e.g. Decision) to this query.
    pub fn with_memory_type(mut self, mt: crate::memory::types::MemoryType) -> Self {
        self.memory_type_filter = Some(mt);
        self
    }

    /// Scope retrieval to a single persona by user_id.
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Enable multi-query expansion.
    pub fn with_multi_query(mut self) -> Self {
        self.multi_query = true;
        self
    }

    /// Apply a temporal recency boost to close-scoring results.
    /// `factor` should be in range 0.0–0.20. Higher values boost
    /// more recent memories more aggressively.
    pub fn with_recency_boost(mut self, factor: f32) -> Self {
        self.recency_boost = Some(factor.clamp(0.0, 0.20));
        self
    }

    /// Set an explicit fusion strategy (overrides auto-routing).
    pub fn with_fusion_strategy(mut self, strategy: FusionStrategy) -> Self {
        self.fusion_strategy = Some(strategy);
        self
    }

    /// Enable automatic query-type routing based on query text characteristics.
    pub fn with_query_type_routing(mut self) -> Self {
        self.query_type_routing = true;
        self
    }

    /// Set custom source-type weights for provenance-aware retrieval.
    ///
    /// When set, overrides the default SourceTypeWeights
    /// (real=1.0, synthetic=0.85, derived=0.70, unknown=0.95).
    pub fn with_source_type_weights(
        mut self,
        weights: crate::retrieval::source_weighting::SourceTypeWeights,
    ) -> Self {
        self.source_type_weights = Some(weights);
        self
    }
}

/// Operations for updating a node — used by DreamMode and AuditScheduler.
/// This enum-based approach is dyn Trait-compatible
/// (unlike closure-based update_node which doesn't work with Arc<dyn StorageBackend>).
#[derive(Debug, Clone, serde::Serialize)]
pub enum UpdateOperation {
    /// Multiply the node's weight by a factor (e.g., weight *= 0.95).
    MultiplyWeight(f64),
    /// Set the weight directly (used by AuditScheduler).
    SetWeight(f64),
    /// Set parent_tier_id if not already set.
    SetParentTierId(Uuid),
    /// Set the node status (used by AuditScheduler).
    SetStatus(MemoryStatus),
    /// Add a child tier ID to children_tier_ids (fractal linking).
    AddChildTierId(Uuid),
    /// Composite operation: set weight + optionally status (used by AuditScheduler).
    /// This must be atomic — both changes happen together.
    ApplyAudit {
        weight: f64,
        status: Option<MemoryStatus>,
    },
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
                node.status = *status;
            }
            UpdateOperation::AddChildTierId(child_id) => {
                if !node.children_tier_ids.contains(child_id) {
                    node.children_tier_ids.push(*child_id);
                }
            }
            UpdateOperation::ApplyAudit { weight, status } => {
                node.weight = *weight;
                if let Some(s) = status {
                    node.status = *s;
                }
            }
        }
    }
}

/// A scored retrieval result from a storage backend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoredNode {
    pub id: Uuid,
    pub score: f32,
    /// Softmax-normalized probability distribution over the entire result set.
    /// Only populated when distributional scoring is enabled (hybrid_retrieve with RRF).
    pub distribution_scores: Option<Vec<f32>>,
    pub debug: Option<ScoreDebug>,
    pub node: FractalNode,
}

#[derive(Debug, Clone, Default)]
pub struct EmbeddingRepairReport {
    pub scanned: usize,
    pub repaired: usize,
    pub skipped: usize,
    pub target_dimension: usize,
}

impl EmbeddingRepairReport {
    pub fn is_empty(&self) -> bool {
        self.scanned == 0 && self.repaired == 0 && self.skipped == 0
    }
}

impl ScoredNode {
    /// Convert from a (score, node) tuple.
    pub fn from_tuple((score, node): (f32, FractalNode)) -> Self {
        Self {
            id: node.id,
            score,
            distribution_scores: None,
            debug: None,
            node,
        }
    }

    /// Convert from a raw FractalNode (score defaults to 1.0).
    pub fn from_node(node: FractalNode) -> Self {
        Self {
            id: node.id,
            score: 1.0,
            distribution_scores: None,
            debug: None,
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

    /// Check if a node with the given external_id already exists.
    /// Returns the existing node's UUID if found.
    async fn find_by_external_id(&self, _external_id: &str) -> Option<Uuid> {
        None // Default: no dedup (overridden by MemoryStore)
    }

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

    /// Expand a flat result set into fractal children via `children_tier_ids`.
    ///
    /// This bridges the gap between consolidation-built UUID links
    /// (`children_tier_ids`) and the old synchronous `zoom_retrieve()`
    /// (which only traversed the unused `self.children` field).
    ///
    /// For each input node with non-empty `children_tier_ids`:
    /// 1. Look up each child UUID via `get()`.
    /// 2. Compute cosine similarity against `query_vector`.
    /// 3. If similarity >= `pruning_threshold`: include the child and
    ///    recursively expand its children (up to `max_depth`).
    ///
    /// The default impl returns the input unchanged (no fractal expansion).
    async fn expand_fractal(
        &self,
        nodes: Vec<ScoredNode>,
        _query_vector: &[f32],
        _max_depth: usize,
        _pruning_threshold: f32,
    ) -> anyhow::Result<Vec<ScoredNode>> {
        Ok(nodes)
    }

    /// Standalone BM25 keyword search (no vector component).
    async fn search_bm25(&self, query_text: &str, top_k: usize)
        -> anyhow::Result<Vec<(Uuid, f32)>>;

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

    /// Re-embed legacy nodes whose stored vector dimension no longer matches the active provider.
    async fn repair_embedding_dimensions(
        &self,
        _provider: &dyn EmbeddingProvider,
    ) -> anyhow::Result<EmbeddingRepairReport> {
        Ok(EmbeddingRepairReport::default())
    }
}

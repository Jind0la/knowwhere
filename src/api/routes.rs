use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::embedding::{EmbeddingProvider, embed_document, embed_query};
use crate::memory::dream::DreamStatus;
#[cfg(feature = "postgres-storage")]
use crate::memory::skills::CreateSkillResponse;
use crate::memory::types::{ContextTier, MemorySource, MemoryType, Sensitivity};
use crate::memory::{DreamMode, Event, EventStore, FractalNode, GovernancePolicy, GovernanceValidator, InMemoryEventStore};
use crate::multimodal::MultimodalData;
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerStatus};
use crate::api::webhooks::{check_webhook_secret, DedupCache};

#[derive(Serialize, ToSchema)]
pub struct ScoredNode {
    pub score: f32,
    pub id: Uuid,
    #[serde(default)]
    pub memory_type: MemoryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    #[schema(value_type = Object)]
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Governance fields (populated when Stage 2 governance is applied)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Sensitivity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub governance_passed: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub governance_issues: Vec<crate::memory::governance::ValidationIssue>,
}

impl ScoredNode {
    fn from_node(score: f32, n: FractalNode) -> Self {
        Self {
            score,
            id: n.id,
            memory_type: n.memory_type,
            source: Some(n.source),
            content: n.content,
            original_pointer: n.original_pointer,
            metadata: n.metadata,
            created_at: n.created_at,
            confidence: None,
            sensitivity: None,
            governance_passed: None,
            governance_issues: vec![],
        }
    }

    pub(crate) fn from_governed(
        score: f32,
        n: FractalNode,
        governance_passed: bool,
        issues: Vec<crate::memory::governance::ValidationIssue>,
    ) -> Self {
        Self {
            score,
            id: n.id,
            memory_type: n.memory_type,
            source: Some(n.source),
            content: n.content,
            original_pointer: n.original_pointer,
            metadata: n.metadata,
            created_at: n.created_at,
            confidence: Some(n.confidence),
            sensitivity: Some(n.sensitivity),
            governance_passed: Some(governance_passed),
            governance_issues: issues,
        }
    }
}

/// Strip markdown/table/emoji formatting for cleaner embeddings.
fn clean_for_embedding(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed == "---"
            || trimmed == "```"
            || trimmed.starts_with("| -")
            || trimmed.starts_with("|--")
        {
            continue;
        }
        let mut cleaned: String = trimmed
            .replace("**", "")
            .replace("##", "")
            .replace('#', "")
            .replace('|', " ")
            .replace("✅", "")
            .replace("❌", "")
            .replace("⚠️", "")
            .replace("🤖", "")
            .replace("🚀", "")
            .replace("🧠", "")
            .replace("```", "");
        // Collapse whitespace
        while cleaned.contains("  ") {
            cleaned = cleaned.replace("  ", " ");
        }
        let cleaned = cleaned.trim().trim_start_matches('-').trim();
        if cleaned.is_empty() || cleaned.len() < 3 {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(cleaned);
    }
    if out.len() > 1024 {
        let original_len = out.len();
        // Stable replacement for nightly-only floor_char_boundary
        let mut end = 1024;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        tracing::debug!(
            original_len,
            truncated_to = out.len(),
            "clean_for_embedding: text truncated for embedding"
        );
    }
    out
}
use crate::storage::{MemoryStore, StorageBackend, HybridQuery};

#[derive(Clone)]
pub struct AppState {
    /// Primary storage backend (trait object for flexibility).
    pub store: Arc<dyn StorageBackend>,
    /// DreamMode and consolidation scheduler need a StorageBackend.
    pub dream_store: Arc<dyn StorageBackend>,
    pub dream: DreamMode,
    pub embedding: Arc<dyn EmbeddingProvider>,
    /// Active governance policy for Stage 2 retrieval validation.
    pub governance_policy: GovernancePolicy,
    /// In-memory event store for Layer 0 (appended to on each mutation).
    /// For production with multiple nodes, use PostgresStore instead.
    pub events: InMemoryEventStore,
    /// PostgreSQL connection pool for trajectory logging and tiered context (postgres-storage feature).
    #[cfg(feature = "postgres-storage")]
    pub trajectory_pool: Option<std::sync::Arc<sqlx::PgPool>>,
    /// VLM background worker handle for async summarization.
    pub vlm_worker: Option<crate::vlm::VlmWorkerHandle>,
    /// Consolidation scheduler for querying cycle_count in /dream/status.
    pub consolidation: Option<std::sync::Arc<crate::scheduler::ConsolidationScheduler>>,
    /// Dedup cache for Frigate webhook events.
    pub frigate_dedup: DedupCache,
    /// Frigate webhook secret (read once at startup, not per-request).
    pub frigate_webhook_secret: Option<String>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

// -- Health Check --

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub node_count: usize,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "Server health status", body = HealthResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let count = state.store.count().await;
    Json(HealthResponse {
        status: "ok".to_string(),
        node_count: count,
    })
}

// -- Embed Text --

#[derive(Deserialize, ToSchema)]
pub struct EmbedRequest {
    pub text: String,
}

#[derive(Serialize, ToSchema)]
pub struct EmbedResponse {
    pub vector: Vec<f32>,
    pub dimension: usize,
    pub provider: String,
}

#[utoipa::path(
    post,
    path = "/embed",
    tag = "embedding",
    request_body = EmbedRequest,
    responses(
        (status = 200, description = "Embedding vector", body = EmbedResponse),
        (status = 500, description = "Embedding failed", body = String)
    )
)]
pub async fn embed_text(
    State(state): State<AppState>,
    Json(req): Json<EmbedRequest>,
) -> Result<Json<EmbedResponse>, (StatusCode, String)> {
    let vector = embed_query(&*state.embedding, &req.text)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let dimension = vector.len();
    let provider = state.embedding.name().to_string();

    Ok(Json(EmbedResponse {
        vector,
        dimension,
        provider,
    }))
}

// -- Store Session --

#[derive(Deserialize, ToSchema)]
pub struct StoreSessionRequest {
    pub content: String,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub metadata: HashMap<String, Value>,
    /// Memory type for this session node (default: episodic).
    #[serde(default = "default_memory_type_str")]
    pub memory_type: String,
    /// Source origin (default: conversation).
    #[serde(default = "default_source_str")]
    pub source: String,
    /// Optional importance 1–10 (default: type-specific).
    #[serde(default)]
    pub importance: Option<i32>,
    /// Optional sensitivity (default: normal).
    #[serde(default)]
    pub sensitivity: Option<Sensitivity>,
}

fn default_memory_type_str() -> String {
    "episodic".to_string()
}

fn default_source_str() -> String {
    "conversation".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct StoreNodeResponse {
    pub id: Uuid,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/store_session",
    tag = "memory",
    request_body = StoreSessionRequest,
    responses(
        (status = 201, description = "Session node created", body = StoreNodeResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn store_session(
    State(state): State<AppState>,
    Json(req): Json<StoreSessionRequest>,
) -> Result<(StatusCode, Json<StoreNodeResponse>), (StatusCode, String)> {
    // Validate content before embedding to avoid opaque upstream errors
    let cleaned = clean_for_embedding(&req.content);
    if cleaned.len() < 4 {
        return Err((StatusCode::BAD_REQUEST, "content too short or empty after cleaning".into()));
    }
    // Reject highly repetitive content — Ollama rejects near-uniform strings
    {
        use std::collections::HashMap;
        let mut freq: HashMap<char, usize> = HashMap::new();
        let mut total = 0usize;
        for c in cleaned.chars() {
            if !c.is_whitespace() {
                *freq.entry(c).or_insert(0) += 1;
                total += 1;
            }
        }
        if total > 0 {
            if let Some(&max_count) = freq.values().max() {
                let ratio = max_count as f64 / total as f64;
                if ratio > 0.9 {
                    return Err((StatusCode::BAD_REQUEST, "content too repetitive for embedding".into()));
                }
            }
        }
    }

    let vector = match req.vector {
        Some(v) if !v.is_empty() => v,
        _ => {
            embed_document(&*state.embedding, &cleaned)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("auto-embed failed: {e}")))?
        }
    };

    let memory_type = MemoryType::parse(&req.memory_type)
        .unwrap_or(MemoryType::Episodic);
    let source = MemorySource::parse(&req.source)
        .unwrap_or(MemorySource::Conversation);

    let mut node = FractalNode::new_typed(
        Some(req.content),
        None,
        vector,
        req.metadata,
        memory_type,
        source,
    );
    if let Some(imp) = req.importance {
        node.importance = imp.clamp(1, 10);
    }
    if let Some(sens) = req.sensitivity {
        node.sensitivity = sens;
    }

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%id, ?memory_type, "session node stored");

    Ok((
        StatusCode::CREATED,
        Json(StoreNodeResponse {
            id,
            message: "session node created".to_string(),
        }),
    ))
}

// -- Store External (Pointer-First: nie Rohdaten, nur Pointer) --

#[derive(Deserialize, ToSchema)]
pub struct StoreExternalRequest {
    pub pointer: String,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub metadata: HashMap<String, Value>,
    #[serde(default)]
    pub multimodal: Option<MultimodalData>,
    /// Memory type (default: semantic).
    #[serde(default = "default_semantic_type_str")]
    pub memory_type: String,
    /// Source origin (default: import).
    #[serde(default = "default_import_source_str")]
    pub source: String,
    /// Optional importance 1–10.
    #[serde(default)]
    pub importance: Option<i32>,
    /// Optional sensitivity.
    #[serde(default)]
    pub sensitivity: Option<Sensitivity>,
}

fn default_semantic_type_str() -> String {
    "semantic".to_string()
}

fn default_import_source_str() -> String {
    "import".to_string()
}

#[utoipa::path(
    post,
    path = "/store_external",
    tag = "memory",
    request_body = StoreExternalRequest,
    responses(
        (status = 201, description = "External pointer node created", body = StoreNodeResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn store_external(
    State(state): State<AppState>,
    Json(req): Json<StoreExternalRequest>,
) -> Result<(StatusCode, Json<StoreNodeResponse>), (StatusCode, String)> {
    let vector = match req.vector {
        Some(v) if !v.is_empty() => v,
        _ => {
            if let Some(ref mm) = req.multimodal {
                let emb = mm.embedding();
                if !emb.is_empty() {
                    emb.to_vec()
                } else {
                    embed_document(&*state.embedding, &req.pointer).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("auto-embed failed: {e}"))
                    })?
                }
            } else {
                embed_document(&*state.embedding, &req.pointer).await.map_err(|e| {
                    (StatusCode::INTERNAL_SERVER_ERROR, format!("auto-embed failed: {e}"))
                })?
            }
        }
    };

    let memory_type = MemoryType::parse(&req.memory_type)
        .unwrap_or(MemoryType::Semantic);
    let source = MemorySource::parse(&req.source)
        .unwrap_or(MemorySource::Import);

    let mut node = FractalNode::new_typed(
        None,
        Some(req.pointer.clone()),
        vector,
        req.metadata,
        memory_type,
        source,
    );
    if let Some(imp) = req.importance {
        node.importance = imp.clamp(1, 10);
    }
    if let Some(sens) = req.sensitivity {
        node.sensitivity = sens;
    }
    if let Some(mm) = req.multimodal {
        node.multimodal = Some(mm);
    }

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%id, ?memory_type, "external pointer node stored");

    Ok((
        StatusCode::CREATED,
        Json(StoreNodeResponse {
            id,
            message: "external pointer node created".to_string(),
        }),
    ))
}

// -- Retrieve Node by ID --

#[utoipa::path(
    get,
    path = "/retrieve/{id}",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Node UUID")
    ),
    responses(
        (status = 200, description = "Node found", body = FractalNode),
        (status = 404, description = "Node not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn retrieve(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<FractalNode>, (StatusCode, String)> {
    tracing::info!(%id, "retrieving node");
    match state.store.get(&id).await {
        Ok(Some(node)) => Ok(Json(node)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("node {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// -- Fractal Retrieve (Zooming) --

#[derive(Deserialize, ToSchema)]
pub struct RetrieveFractalRequest {
    /// Dense query vector (optional — if omitted, query_text is embedded on-the-fly).
    pub query_vector: Option<Vec<f32>>,
    #[serde(default)]
    pub query_text: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// Apply Stage 2 governance filtering (default: true).
    #[serde(default = "default_governance_enabled")]
    pub governance_enabled: bool,
    /// Filter by memory type.
    #[serde(default)]
    pub memory_type_filter: Option<String>,
    /// Maximum context tier to retrieve: "summary", "overview", or "raw".
    /// Only memories at or below this tier are returned (default: "overview").
    #[serde(default = "default_max_tier")]
    pub max_tier: Option<String>,
}

fn default_top_k() -> usize {
    5
}
fn default_max_depth() -> usize {
    3
}
fn default_governance_enabled() -> bool {
    true
}
fn default_max_tier() -> Option<String> {
    // Default to None (show all tiers) — users can opt-in to tier filtering
    // by explicitly passing "summary" or "overview" in their request.
    None
}

#[utoipa::path(
    post,
    path = "/retrieve_fractal",
    tag = "memory",
    request_body = RetrieveFractalRequest,
    responses(
        (status = 200, description = "Fractal retrieval results", body = Vec<ScoredNode>)
    )
)]
pub async fn retrieve_fractal(
    State(state): State<AppState>,
    Json(req): Json<RetrieveFractalRequest>,
) -> Result<Json<Vec<ScoredNode>>, StatusCode> {
    tracing::info!(
        top_k = req.top_k,
        max_depth = req.max_depth,
        has_query_text = req.query_text.is_some(),
        has_query_vector = req.query_vector.is_some(),
        governance = req.governance_enabled,
        max_tier = ?req.max_tier,
        "fractal retrieve"
    );

    // Resolve query vector: use provided vector, or embed query_text on-the-fly
    let query_vector = match &req.query_vector {
        Some(v) => v.clone(),
        None => {
            if let Some(text) = &req.query_text {
                if text.trim().is_empty() {
                    return Err(StatusCode::BAD_REQUEST);
                }
                tracing::info!(query_text = %text, "embedding query text");
                state
                    .embedding
                    .embed(text)
                    .await
                    .map_err(|e| {
                        tracing::error!("embedding failed: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
            } else {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    };
    tracing::info!(query_vector_dim = query_vector.len(), "using query vector");

    // Parse max_tier filter (default: overview)
    let max_tier = req.max_tier.as_ref()
        .and_then(|s| ContextTier::parse(s));

    // Stage 1: Hybrid retrieval via StorageBackend trait
    let query = HybridQuery {
        query_text: req.query_text.clone(),
        query_vector: Some(query_vector),
        top_k: req.top_k,
        max_depth: req.max_depth,
    };
    let results = state
        .store
        .hybrid_retrieve(&query)
        .await
        .map_err(|e| {
            tracing::error!("hybrid_retrieve failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Apply max_tier filter: only include nodes at or below max_tier
    let max_tier_filter = max_tier;
    let results: Vec<crate::storage::ScoredNode> = if let Some(max_t) = max_tier_filter {
        results
            .into_iter()
            .filter(|s| {
                // Higher ordinal = lower tier (Raw=2, Overview=1, Summary=0)
                // Keep node if its tier ordinal <= max_tier ordinal
                s.node.context_tier as usize <= max_t as usize
            })
            .collect()
    } else {
        results
    };

    if !req.governance_enabled {
        let scored: Vec<ScoredNode> = results
            .into_iter()
            .map(|s| ScoredNode::from_node(s.score, s.node))
            .collect();
        return Ok(Json(scored));
    }

    // Optional memory type filter
    let type_filter = req
        .memory_type_filter
        .as_ref()
        .and_then(|s| MemoryType::parse(s));

    // Stage 2: Governance validation
    let validator = GovernanceValidator::new(state.governance_policy.clone());
    let mut scored: Vec<ScoredNode> = results
        .into_iter()
        .filter_map(|s| {
            // Apply optional memory type filter
            if let Some(ref filter) = type_filter {
                if s.node.memory_type != *filter {
                    return None;
                }
            }

            let candidate = s.node.to_governance_candidate();
            let validation = validator.validate(&candidate);

            // Hard-blocked nodes (superseded, restricted, invalid status, irrelevant)
            // are excluded from results entirely.
            if validation.has_hard_block() {
                tracing::debug!(node_id = %s.node.id, "excluded by governance: hard block");
                return None;
            }

            Some(ScoredNode::from_governed(s.score, s.node, validation.passed, validation.issues))
        })
        .collect();

    // Re-sort by combined score (retrieval_score * governance_multiplier).
    // Nodes with hard blocks are already filtered out above.
    scored.sort_by(|a, b| {
        // Apply governance score multiplier to retrieval score
        let multiplier_a = a.governance_issues.iter()
            .map(|i| i.score_impact)
            .fold(1.0_f64, |acc, m| acc * m) as f32;
        let multiplier_b = b.governance_issues.iter()
            .map(|i| i.score_impact)
            .fold(1.0_f64, |acc, m| acc * m) as f32;

        let effective_a = a.score * multiplier_a;
        let effective_b = b.score * multiplier_b;
        effective_b.partial_cmp(&effective_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Json(scored))
}

// -- Delete Node --

#[utoipa::path(
    delete,
    path = "/nodes/{id}",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Node UUID to delete")
    ),
    responses(
        (status = 200, description = "Node deleted"),
        (status = 404, description = "Node not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn delete_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StoreNodeResponse>, (StatusCode, String)> {
    tracing::info!(%id, "deleting node");
    match state.store.delete(&id).await {
        Ok(true) => Ok(Json(StoreNodeResponse {
            id,
            message: "node deleted".to_string(),
        })),
        Ok(false) => Err((StatusCode::NOT_FOUND, format!("node {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// -- Purge Dummy Nodes --

#[derive(Serialize, ToSchema)]
pub struct PurgeResponse {
    pub removed: usize,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/nodes/purge_dummy",
    tag = "memory",
    responses(
        (status = 200, description = "Dummy nodes purged", body = PurgeResponse)
    )
)]
pub async fn purge_dummy(State(state): State<AppState>) -> Json<PurgeResponse> {
    let removed = state.store.purge_dummy_vectors().await;
    tracing::info!(removed, "purged dummy-vector nodes");
    Json(PurgeResponse {
        removed,
        message: format!("{removed} dummy nodes removed"),
    })
}

// -- Recent Nodes --

#[derive(Deserialize, IntoParams)]
pub struct RecentQuery {
    #[serde(default = "default_recent_limit")]
    pub limit: usize,
}

fn default_recent_limit() -> usize {
    20
}

#[utoipa::path(
    get,
    path = "/nodes/recent",
    tag = "memory",
    params(RecentQuery),
    responses(
        (status = 200, description = "Recent nodes", body = Vec<FractalNode>)
    )
)]
pub async fn recent_nodes(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RecentQuery>,
) -> Result<Json<Vec<FractalNode>>, StatusCode> {
    let limit = q.limit.min(100);
    let nodes = state.store.recent(limit).await
        .map_err(|e| {
            tracing::error!("recent failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(nodes))
}

// -- Re-embed All Nodes --

#[derive(Serialize, ToSchema)]
pub struct ReembedResponse {
    pub updated: usize,
    pub failed: usize,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/nodes/reembed_all",
    tag = "memory",
    responses(
        (status = 200, description = "Re-embedding complete", body = ReembedResponse)
    )
)]
pub async fn reembed_all(
    State(state): State<AppState>,
) -> Json<ReembedResponse> {
    let all_nodes = state.store.list_all().await.unwrap_or_default();
    let mut updated = 0usize;
    let mut failed = 0usize;

    for node in &all_nodes {
        let text = match (&node.content, &node.original_pointer) {
            (Some(c), _) => clean_for_embedding(c),
            (_, Some(p)) => p.clone(),
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }

        match embed_document(&*state.embedding, &text).await {
            Ok(vec) => {
                if state.store.update_vector(&node.id, vec).await.unwrap_or(false) {
                    updated += 1;
                } else {
                    failed += 1;
                }
            }
            Err(e) => {
                tracing::warn!(id = %node.id, "reembed failed: {e}");
                failed += 1;
            }
        }
    }

    tracing::info!(updated, failed, "reembed_all complete");
    Json(ReembedResponse {
        updated,
        failed,
        message: format!("{updated} nodes re-embedded, {failed} failed"),
    })
}

// -- Dream Status --

#[utoipa::path(
    get,
    path = "/dream/status",
    tag = "system",
    responses(
        (status = 200, description = "Dream mode status", body = DreamStatus)
    )
)]
pub async fn dream_status(State(state): State<AppState>) -> Json<DreamStatus> {
    let mut status = state.dream.status().await;
    if let Some(ref scheduler) = state.consolidation {
        status.cycle_count = scheduler.cycle_count();
    }
    Json(status)
}

// -- Governance Policy --

/// Get the current governance policy.
#[utoipa::path(
    get,
    path = "/governance/policy",
    tag = "governance",
    responses(
        (status = 200, description = "Current governance policy", body = GovernancePolicy)
    )
)]
pub async fn get_governance_policy(State(state): State<AppState>) -> Json<GovernancePolicy> {
    Json(state.governance_policy.clone())
}

/// Update the governance policy.
#[derive(Deserialize, ToSchema)]
pub struct UpdatePolicyRequest {
    #[serde(default)]
    pub min_confidence: Option<f64>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
    #[serde(default)]
    pub blocked_sensitivities: Option<Vec<Sensitivity>>,
    #[serde(default)]
    pub supersession_enabled: Option<bool>,
    #[serde(default)]
    pub conflict_check_enabled: Option<bool>,
    #[serde(default)]
    pub recency_boost_enabled: Option<bool>,
    #[serde(default)]
    pub recency_penalty_after_days: Option<u32>,
    /// Preset: "default", "strict", or "lenient". Overrides other fields if set.
    #[serde(default)]
    pub preset: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct UpdatePolicyResponse {
    pub message: String,
    pub policy: GovernancePolicy,
}

#[utoipa::path(
    post,
    path = "/governance/policy",
    tag = "governance",
    request_body = UpdatePolicyRequest,
    responses(
        (status = 200, description = "Policy updated", body = UpdatePolicyResponse)
    )
)]
pub async fn update_governance_policy(
    State(state): State<AppState>,
    Json(req): Json<UpdatePolicyRequest>,
) -> Json<UpdatePolicyResponse> {
    let mut policy = state.governance_policy.clone();

    if let Some(preset) = req.preset {
        policy = match preset.as_str() {
            "strict" => GovernancePolicy::strict(),
            "lenient" => GovernancePolicy::lenient(),
            _ => GovernancePolicy::default_policy(),
        };
    }

    if let Some(v) = req.min_confidence {
        policy.min_confidence = v.clamp(0.0, 1.0);
    }
    if let Some(v) = req.max_age_days {
        policy.max_age_days = Some(v);
    }
    if let Some(v) = req.blocked_sensitivities {
        policy.blocked_sensitivities = v;
    }
    if let Some(v) = req.supersession_enabled {
        policy.supersession_enabled = v;
    }
    if let Some(v) = req.conflict_check_enabled {
        policy.conflict_check_enabled = v;
    }
    if let Some(v) = req.recency_boost_enabled {
        policy.recency_boost_enabled = v;
    }
    if let Some(v) = req.recency_penalty_after_days {
        policy.recency_penalty_after_days = v;
    }

    // Note: in a real app this would be persisted. For now it's in-memory only.
    tracing::info!(?policy, "governance policy updated");

    Json(UpdatePolicyResponse {
        message: "governance policy updated".to_string(),
        policy,
    })
}

// -- Event Log (Layer 0 — read-only) --

#[derive(Deserialize, IntoParams)]
pub struct EventsQuery {
    /// Only return events after this ID (cursor-based pagination).
    #[serde(default)]
    pub after_id: Option<Uuid>,
    /// Maximum number of events to return (default 100, max 1000).
    #[serde(default = "default_events_limit")]
    pub limit: i64,
}

fn default_events_limit() -> i64 {
    100
}

#[utoipa::path(
    get,
    path = "/events",
    tag = "system",
    params(EventsQuery),
    responses(
        (status = 200, description = "Event log entries (in-memory, single-node)")
    )
)]
pub async fn list_events(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
) -> Json<Vec<Event>> {
    let limit = q.limit.min(1000);
    match state.events.read_after(q.after_id, limit).await {
        Ok(events) => Json(events),
        Err(e) => {
            tracing::warn!("failed to read events: {e}");
            Json(vec![])
        }
    }
}

// -- Retrieval Trajectory Endpoints (postgres-storage feature) --

/// List recent retrieval runs with cursor-based pagination.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct RetrievalRunsQuery {
    /// Maximum number of runs to return (default 20, max 100).
    #[serde(default = "default_runs_limit")]
    pub limit: i32,
    /// Only return runs before this ID (cursor-based pagination).
    #[serde(default)]
    pub after_id: Option<Uuid>,
}

#[cfg(feature = "postgres-storage")]
fn default_runs_limit() -> i32 {
    20
}

/// Response wrapper for a retrieval run.
#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct RetrievalRunResponse {
    pub id: Uuid,
    pub query_text: String,
    pub run_at: chrono::DateTime<chrono::Utc>,
    pub total_candidates: Option<i32>,
    pub retrieved_count: Option<i32>,
    pub execution_time_ms: Option<i32>,
    pub max_depth_used: Option<i32>,
}

#[cfg(feature = "postgres-storage")]
impl From<crate::storage::RetrievalRunRow> for RetrievalRunResponse {
    fn from(row: crate::storage::RetrievalRunRow) -> Self {
        Self {
            id: row.id,
            query_text: row.query_text,
            run_at: row.run_at,
            total_candidates: row.total_candidates,
            retrieved_count: row.retrieved_count,
            execution_time_ms: row.execution_time_ms,
            max_depth_used: row.max_depth_used,
        }
    }
}

/// Response for trajectory steps.
#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct TrajectoryStepResponse {
    pub step_index: i32,
    pub step_type: String,
    pub memory_id: Option<Uuid>,
    pub score_before: Option<f64>,
    pub score_after: Option<f64>,
    pub rank: Option<i32>,
    pub decision: Option<String>,
    pub filter_reason: Option<String>,
}

#[cfg(feature = "postgres-storage")]
impl From<crate::storage::TrajectoryStepRow> for TrajectoryStepResponse {
    fn from(row: crate::storage::TrajectoryStepRow) -> Self {
        Self {
            step_index: row.step_index,
            step_type: row.step_type,
            memory_id: row.memory_id,
            score_before: row.score_before,
            score_after: row.score_after,
            rank: row.rank,
            decision: row.decision,
            filter_reason: row.filter_reason,
        }
    }
}

/// GET /retrieval/runs — list recent retrieval runs.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/retrieval/runs",
    tag = "retrieval",
    params(RetrievalRunsQuery),
    responses(
        (status = 200, description = "Recent retrieval runs", body = Vec<RetrievalRunResponse>)
    )
)]
pub async fn list_retrieval_runs(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RetrievalRunsQuery>,
) -> Result<Json<Vec<RetrievalRunResponse>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p,
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };
    let store = crate::storage::TrajectoryStore::new(pool.as_ref());
    let limit = q.limit.min(100);
    match store.list_runs(limit, q.after_id).await {
        Ok(rows) => Ok(Json(rows.into_iter().map(RetrievalRunResponse::from).collect())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /retrieval/runs/{id} — get a single retrieval run.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/retrieval/runs/{id}",
    tag = "retrieval",
    params(
        ("id" = Uuid, Path, description = "Retrieval run UUID")
    ),
    responses(
        (status = 200, description = "Retrieval run", body = RetrievalRunResponse),
        (status = 404, description = "Run not found", body = String)
    )
)]
pub async fn get_retrieval_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RetrievalRunResponse>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p,
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };
    let store = crate::storage::TrajectoryStore::new(pool.as_ref());
    match store.get_run(id).await {
        Ok(Some(row)) => Ok(Json(RetrievalRunResponse::from(row))),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("retrieval run {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /retrieval/runs/{id}/trajectory — get all trajectory steps for a retrieval run.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/retrieval/runs/{id}/trajectory",
    tag = "retrieval",
    params(
        ("id" = Uuid, Path, description = "Retrieval run UUID")
    ),
    responses(
        (status = 200, description = "Trajectory steps for this run", body = Vec<TrajectoryStepResponse>),
        (status = 404, description = "Run not found", body = String)
    )
)]
pub async fn get_retrieval_trajectory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<TrajectoryStepResponse>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p,
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };
    let store = crate::storage::TrajectoryStore::new(pool.as_ref());
    match store.get_trajectory(id).await {
        Ok(rows) => Ok(Json(rows.into_iter().map(TrajectoryStepResponse::from).collect())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// -- Tiered Context Endpoints (postgres-storage feature) --

/// Compact a memory to the specified tier (or the next tier down).
///
/// L2 (Raw) → L1 (Overview) → L0 (Summary).
/// If `tier` is omitted, compaction proceeds one step.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct CompactMemoryQuery {
    /// Target tier: "overview" or "summary". Defaults to next tier down.
    #[serde(default)]
    pub tier: Option<String>,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct CompactMemoryResponse {
    pub id: Uuid,
    pub tier: String,
    pub message: String,
}

/// POST /memories/{id}/compact — compact a memory to a higher tier.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/memories/{id}/compact",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Memory UUID to compact")
    ),
    responses(
        (status = 200, description = "Memory compacted", body = CompactMemoryResponse),
        (status = 404, description = "Memory not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn compact_memory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<CompactMemoryQuery>,
) -> Result<Json<CompactMemoryResponse>, (StatusCode, String)> {
    use crate::memory::{ContextTier, TieredCompactionWorker};

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let target_tier = q.tier.as_ref().and_then(|s| ContextTier::parse(s));

    let worker = TieredCompactionWorker::new((*pool).clone(), state.embedding.clone(), state.vlm_worker.clone());
    match worker.compact_memory(id, target_tier).await {
        Ok(new_id) => {
            let tier_str = if new_id == id {
                id.to_string() // no new tier created
            } else {
                target_tier.map(|t| t.to_string()).unwrap_or_else(|| "next".to_string())
            };
            Ok(Json(CompactMemoryResponse {
                id: new_id,
                tier: tier_str,
                message: if new_id == id {
                    "memory already at target tier".to_string()
                } else {
                    format!("compacted to {}", target_tier.map(|t| t.to_string()).unwrap_or_else(|| "next tier".to_string()))
                },
            }))
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// GET /memories/{id} — retrieve a memory node with optional tier loading.
///
/// If `tier=raw` is specified and the memory has an L2 (raw) version available,
/// returns the raw content. Otherwise returns the node as-is.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct GetMemoryQuery {
    /// Desired context tier: "summary", "overview", or "raw".
    /// If the memory is stored at a lower tier than requested, follows parent_tier_id chain.
    #[serde(default)]
    pub tier: Option<String>,
}

/// GET /memories/{id} — retrieve a memory node (extended with tier support for postgres-storage).
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/memories/{id}",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Memory UUID")
    ),
    responses(
        (status = 200, description = "Memory node", body = FractalNode),
        (status = 404, description = "Memory not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn get_memory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<GetMemoryQuery>,
) -> Result<Json<FractalNode>, (StatusCode, String)> {
    use crate::memory::ContextTier;

    tracing::info!(%id, tier = ?q.tier, "get_memory");

    match state.store.get(&id).await {
        Ok(Some(node)) => {
            // If a specific tier was requested, check if we need to follow the chain
            if let Some(ref tier_str) = q.tier {
                if let Some(requested_tier) = ContextTier::parse(tier_str) {
                    // If node is below requested tier, try to follow parent_tier_id chain
                    if node.context_tier as usize > requested_tier as usize {
                        // e.g., node is Summary (0) but Raw (2) was requested
                        // Try to find a higher-tier memory via parent chain
                        // For now, log and return the node as-is (full chain lookup needs DB)
                        tracing::debug!(
                            node_id = %id,
                            node_tier = %node.context_tier,
                            requested_tier = %requested_tier,
                            "requested tier not directly available, returning current tier"
                        );
                    }
                }
            }
            Ok(Json(node))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("memory {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// =============================================================================
// Conflict Detection Endpoints
// =============================================================================

#[cfg(feature = "postgres-storage")]
use crate::memory::dream::conflict_detection::{ConflictDetector, ConflictGroup};
#[cfg(feature = "postgres-storage")]
use crate::memory::dream::energy_decay::{EnergyDecayWorker, MemoryEnergyInfo, DecayResult, CompressionResult};
#[cfg(feature = "postgres-storage")]
use crate::memory::dream::deduplication::{DeduplicationWorker, DuplicatePair, DeduplicationRunRow, DeduplicationResult};

/// GET /conflicts — list all pending (unresolved) conflicts.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/conflicts",
    tag = "governance",
    responses(
        (status = 200, description = "List of pending conflicts", body = Vec<ConflictGroup>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_conflicts(State(state): State<AppState>) -> Result<Json<Vec<ConflictGroup>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let detector = ConflictDetector::new(&pool);
    match detector.list_pending_conflicts().await {
        Ok(conflicts) => Ok(Json(conflicts)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// POST /conflicts/{id}/resolve — resolve a conflict by choosing the winning memory.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct ResolveConflictRequest {
    /// The ID of the memory that should win this conflict.
    pub winning_memory_id: Uuid,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct ResolveConflictResponse {
    pub message: String,
    pub winning_memory_id: Uuid,
    pub conflict_id: Uuid,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/conflicts/{id}/resolve",
    tag = "governance",
    params(
        ("id" = Uuid, Path, description = "Conflict ID to resolve")
    ),
    request_body = ResolveConflictRequest,
    responses(
        (status = 200, description = "Conflict resolved", body = ResolveConflictResponse),
        (status = 404, description = "Conflict not found", body = String),
        (status = 400, description = "Winning memory not in conflict", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn resolve_conflict(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ResolveConflictRequest>,
) -> Result<Json<ResolveConflictResponse>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let detector = ConflictDetector::new(&pool);
    match detector.resolve_conflict(id, req.winning_memory_id).await {
        Ok(()) => Ok(Json(ResolveConflictResponse {
            message: "conflict resolved".to_string(),
            winning_memory_id: req.winning_memory_id,
            conflict_id: id,
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err((StatusCode::NOT_FOUND, msg))
            } else if msg.contains("not part of conflict") {
                Err((StatusCode::BAD_REQUEST, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

// =============================================================================
// Energy Decay Routes (Ebbinghaus forgetting curve)
// =============================================================================

/// Request body for energy boost.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct BoostEnergyRequest {
    /// Energy units to add (e.g. 20 for a retrieval access).
    #[schema(default = 20)]
    pub boost: i32,
}

/// Response after boosting energy.
#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct BoostEnergyResponse {
    pub memory_id: Uuid,
    pub boost: i32,
    pub message: String,
}

/// POST /memories/{id}/energy/boost — boost energy after memory access
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/memories/{id}/energy/boost",
    tag = "dream",
    params(
        ("id" = Uuid, Path, description = "Memory ID to boost")
    ),
    request_body = BoostEnergyRequest,
    responses(
        (status = 200, description = "Energy boosted", body = BoostEnergyResponse),
        (status = 404, description = "Memory not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn boost_memory_energy(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<BoostEnergyRequest>,
) -> Result<Json<BoostEnergyResponse>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let worker = EnergyDecayWorker::with_defaults(&pool);
    match worker.boost_energy(id, req.boost).await {
        Ok(()) => Ok(Json(BoostEnergyResponse {
            memory_id: id,
            boost: req.boost,
            message: format!("energy boosted by {}", req.boost),
        })),
        Err(e) => {
            if e.to_string().contains("0 rows") {
                Err((StatusCode::NOT_FOUND, format!("memory {} not found or not active", id)))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
            }
        }
    }
}

/// Query params for low-energy memory listing.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema, IntoParams)]
pub struct LowEnergyQuery {
    /// Maximum number of memories to return (default: 10).
    #[param(default = 10)]
    pub limit: i32,
}

/// GET /energy/low?limit=10 — list low-energy memories (candidates for compression)
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/energy/low",
    tag = "dream",
    params(
        LowEnergyQuery
    ),
    responses(
        (status = 200, description = "Low-energy memories", body = Vec<MemoryEnergyInfo>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_low_energy_memories(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<LowEnergyQuery>,
) -> Result<Json<Vec<MemoryEnergyInfo>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let worker = EnergyDecayWorker::with_defaults(&pool);
    match worker.find_low_energy_memories(query.limit).await {
        Ok(memories) => Ok(Json(memories)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// POST /energy/decay/apply — apply energy decay to all active memories
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/energy/decay/apply",
    tag = "dream",
    responses(
        (status = 200, description = "Decay applied", body = DecayResult),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn apply_energy_decay(
    State(state): State<AppState>,
) -> Result<Json<DecayResult>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let worker = EnergyDecayWorker::with_defaults(&pool);
    match worker.apply_decay().await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Request body for cluster compression.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct CompressClusterRequest {
    /// Memory IDs to compress (2–4 memories).
    pub memory_ids: Vec<Uuid>,
}

/// POST /energy/compress — compress a cluster of low-energy memories into one
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/energy/compress",
    tag = "dream",
    request_body = CompressClusterRequest,
    responses(
        (status = 200, description = "Cluster compressed", body = CompressionResult),
        (status = 400, description = "Invalid request (need 2+ memories)", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn compress_memory_cluster(
    State(state): State<AppState>,
    Json(req): Json<CompressClusterRequest>,
) -> Result<Json<CompressionResult>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    if req.memory_ids.len() < 2 {
        return Err((StatusCode::BAD_REQUEST, "need at least 2 memory IDs to compress".into()));
    }

    let worker = EnergyDecayWorker::with_defaults(&pool);
    match worker.compress_cluster(&req.memory_ids).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

// =============================================================================
// Deduplication Routes
// =============================================================================

/// GET /deduplication/candidates — find duplicate memory pairs (preview, no merge)
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/deduplication/candidates",
    tag = "dream",
    responses(
        (status = 200, description = "Duplicate candidate pairs", body = Vec<DuplicatePair>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_deduplication_candidates(
    State(state): State<AppState>,
) -> Result<Json<Vec<DuplicatePair>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let worker = DeduplicationWorker::with_defaults(&pool);
    match worker.find_duplicates().await {
        Ok(pairs) => Ok(Json(pairs)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// POST /deduplication/run — run full deduplication (find + merge all duplicates)
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/deduplication/run",
    tag = "dream",
    responses(
        (status = 200, description = "Deduplication run result", body = DeduplicationResult),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn run_deduplication(
    State(state): State<AppState>,
) -> Result<Json<DeduplicationResult>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let worker = DeduplicationWorker::with_defaults(&pool);
    match worker.run_deduplication().await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Query params for recent deduplication runs.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema, IntoParams)]
pub struct DedupRunsQuery {
    /// Maximum number of runs to return (default: 10).
    #[param(default = 10)]
    pub limit: i32,
}

/// GET /deduplication/runs — list recent deduplication runs
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/deduplication/runs",
    tag = "dream",
    params(
        DedupRunsQuery
    ),
    responses(
        (status = 200, description = "Recent deduplication runs", body = Vec<DeduplicationRunRow>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_deduplication_runs(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<DedupRunsQuery>,
) -> Result<Json<Vec<DeduplicationRunRow>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let worker = DeduplicationWorker::with_defaults(&pool);
    match worker.recent_runs(query.limit).await {
        Ok(runs) => Ok(Json(runs)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// =============================================================================
// Self-Healing Endpoints (Content Hashing for External Nodes)
// =============================================================================

#[cfg(feature = "postgres-storage")]
use crate::memory::self_healing::{
    HealthCheckResult, HealingStats, RepairStatus, SelfHealingService,
};

/// POST /memories/{id}/reindex — re-compute content hash + semantic thumbnail for an external node.
///
/// This is useful when the underlying file has changed and needs to be re-indexed.
#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct ReindexResponse {
    pub memory_id: Uuid,
    pub content_hash: Option<String>,
    pub thumbnail_words: usize,
    pub message: String,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/memories/{id}/reindex",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Memory UUID to re-index")
    ),
    responses(
        (status = 200, description = "Memory re-indexed", body = ReindexResponse),
        (status = 404, description = "Memory not found", body = String),
        (status = 400, description = "Memory has no external pointer", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn reindex_external_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ReindexResponse>, (StatusCode, String)> {
    use std::path::PathBuf;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    // Fetch the memory to get its original_pointer
    let node = match state.store.get(&id).await {
        Ok(Some(n)) => n,
        Ok(None) => return Err((StatusCode::NOT_FOUND, format!("memory {id} not found"))),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let uri = node
        .original_pointer
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("memory {id} has no original_pointer (not an external node)")))?;

    // Convert URI to path
    let file_path: PathBuf = if uri.starts_with("file://") {
        PathBuf::from(&uri[7..])
    } else {
        PathBuf::from(uri)
    };

    if !file_path.exists() {
        return Err((StatusCode::BAD_REQUEST, format!("pointer file does not exist: {}", file_path.display())));
    }

    // Get file_root from env or default
    let file_root = std::env::var("KNOWWHERE_FILE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));

    let service = SelfHealingService::new((*pool).clone(), file_root);
    service
        .index_external_node(id, &file_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("reindex failed: {e}")))?;

    // Fetch updated hash/thumbnail for response
    let (content_hash, thumbnail_words) = {
        let row = sqlx::query!(
            r#"
            SELECT content_hash, semantic_thumbnail
            FROM memories WHERE id = $1
            "#,
            id,
        )
        .fetch_one(
            match state.trajectory_pool.as_ref() {
                Some(arc) => &**arc,
                None => return Err((StatusCode::INTERNAL_SERVER_ERROR, "no trajectory pool".into()).into()),
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        (
            row.content_hash,
            row.semantic_thumbnail.as_ref().map(|t| t.split_whitespace().count()).unwrap_or(0),
        )
    };

    Ok(Json(ReindexResponse {
        memory_id: id,
        content_hash,
        thumbnail_words,
        message: "reindexed successfully".to_string(),
    }))
}

/// GET /memories/{id}/health — check if an external node's pointer is still valid.
///
/// If broken, performs automatic self-healing (hash → semantic fallback).
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/memories/{id}/health",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Memory UUID to health-check")
    ),
    responses(
        (status = 200, description = "Health check result", body = HealthCheckResult),
        (status = 404, description = "Memory not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn memory_health_check(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<HealthCheckResult>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let file_root = std::env::var("KNOWWHERE_FILE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));

    let service = SelfHealingService::new((*pool).clone(), file_root);
    match service.health_check(id).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /self-healing/stats — statistics about broken vs. repaired pointers.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/self-healing/stats",
    tag = "memory",
    responses(
        (status = 200, description = "Self-healing statistics", body = HealingStats),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn self_healing_stats(
    State(state): State<AppState>,
) -> Result<Json<HealingStats>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let file_root = std::env::var("KNOWWHERE_FILE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));

    let service = SelfHealingService::new((*pool).clone(), file_root);
    match service.stats().await {
        Ok(stats) => Ok(Json(stats)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// =============================================================================
// Namespace Routes
// =============================================================================

/// GET /namespaces — list all namespaces.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces",
    tag = "namespaces",
    responses(
        (status = 200, description = "All namespaces", body = Vec<crate::memory::namespaces::MemoryNamespace>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_namespaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::memory::namespaces::MemoryNamespace>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.list_all().await {
        Ok(ns) => Ok(Json(ns)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /namespaces/{path} — get a namespace by path (e.g. `agent/skills`).
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces/{path}",
    tag = "namespaces",
    params(
        ("path" = String, Path, description = "Namespace path (e.g. agent/skills)")
    ),
    responses(
        (status = 200, description = "Namespace found"),
        (status = 404, description = "Namespace not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn get_namespace(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Result<Json<crate::memory::namespaces::MemoryNamespace>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.find_by_path(&path).await {
        Ok(Some(ns)) => Ok(Json(ns)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("namespace '{path}' not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /namespaces/{path}/memories — list memories within a namespace.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct NamespaceMemoriesQuery {
    #[param(default = 50)]
    pub limit: i32,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces/{path}/memories",
    tag = "namespaces",
    params(
        ("path" = String, Path, description = "Namespace path"),
        NamespaceMemoriesQuery
    ),
    responses(
        (status = 200, description = "Memories in this namespace"),
        (status = 404, description = "Namespace not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn namespace_memories(
    State(state): State<AppState>,
    Path(path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<NamespaceMemoriesQuery>,
) -> Result<Json<Vec<crate::memory::namespaces::MemoryRow>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.find_by_path(&path).await {
        Ok(Some(ns)) => match store.memories_in_namespace(ns.id, q.limit).await {
            Ok(rows) => Ok(Json(rows)),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("namespace '{path}' not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// POST /namespaces — create a new namespace.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct CreateNamespaceRequest {
    pub path: String,
    pub description: Option<String>,
    pub memory_type_hint: Option<String>,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct CreateNamespaceResponse {
    pub id: Uuid,
    pub message: String,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/namespaces",
    tag = "namespaces",
    request_body = CreateNamespaceRequest,
    responses(
        (status = 201, description = "Namespace created", body = CreateNamespaceResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn create_namespace(
    State(state): State<AppState>,
    Json(req): Json<CreateNamespaceRequest>,
) -> Result<(StatusCode, Json<CreateNamespaceResponse>), (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let depth = req.path.matches('/').count() as i32 + 1;
    let ns = crate::memory::namespaces::MemoryNamespace {
        id: Uuid::new_v4(),
        path: req.path.clone(),
        depth,
        parent_id: None,
        description: req.description,
        memory_type_hint: req.memory_type_hint,
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.create(&ns).await {
        Ok(id) => Ok((
            StatusCode::CREATED,
            Json(CreateNamespaceResponse {
                id,
                message: format!("namespace '{}' created", req.path),
            }),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /namespaces/{path}/search — search memories within a namespace.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct NamespaceSearchQuery {
    pub q: String,
    #[param(default = 10)]
    pub top_k: usize,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces/{path}/search",
    tag = "namespaces",
    params(
        ("path" = String, Path, description = "Namespace path"),
        NamespaceSearchQuery
    ),
    responses(
        (status = 200, description = "Search results from namespace", body = Vec<ScoredNode>)
    )
)]
pub async fn namespace_search(
    State(state): State<AppState>,
    Path(path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<NamespaceSearchQuery>,
) -> Result<Json<Vec<ScoredNode>>, (StatusCode, String)> {
    use crate::memory::namespaces::NamespaceStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = NamespaceStore::new(pool.as_ref());

    // Resolve path to namespace ID
    let namespace_id = match store.find_by_path(&path).await {
        Ok(Some(ns)) => ns.id,
        Ok(None) => return Err((StatusCode::NOT_FOUND, format!("namespace '{path}' not found"))),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Get memory IDs in this namespace
    let memory_rows = match store.memories_in_namespace(namespace_id, 500).await {
        Ok(rows) => rows,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let memory_ids: Vec<Uuid> = memory_rows.into_iter().map(|r| r.id).collect();

    if memory_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    // Search in-memory store for those IDs with the query
    let query_text = q.q.clone();
    let query_vector = embed_query(&*state.embedding, &query_text)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("embed failed: {e}")))?;

    // Hybrid retrieve with the namespace constraint
    let query = HybridQuery {
        query_text: Some(query_text),
        query_vector: Some(query_vector),
        top_k: q.top_k,
        max_depth: 3,
    };
    let all_results = state
        .store
        .hybrid_retrieve(&query)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("hybrid_retrieve failed: {e}")))?;

    // Filter to only memories in this namespace
    let filtered: Vec<ScoredNode> = all_results
        .into_iter()
        .filter(|s| memory_ids.contains(&s.node.id))
        .map(|s| ScoredNode::from_node(s.score, s.node))
        .collect();

    Ok(Json(filtered))
}

// =============================================================================
// Skills Routes
// =============================================================================

/// POST /skills — create a new skill.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/skills",
    tag = "skills",
    request_body = crate::memory::skills::CreateSkillRequest,
    responses(
        (status = 201, description = "Skill created", body = CreateSkillResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn create_skill(
    State(state): State<AppState>,
    Json(req): Json<crate::memory::skills::CreateSkillRequest>,
) -> Result<(StatusCode, Json<crate::memory::skills::CreateSkillResponse>), (StatusCode, String)> {
    use crate::memory::skills::{CreateSkillResponse, SkillsStore};

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.create(&req).await {
        Ok(id) => Ok((
            StatusCode::CREATED,
            Json(CreateSkillResponse {
                id,
                message: format!("skill '{}' created", req.skill_name),
            }),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /skills — list all skills (filter by category or min_proficiency).
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct ListSkillsQuery {
    /// Filter by category (e.g. `language`, `tool`, `domain`, `framework`).
    #[param(default)]
    pub category: Option<String>,
    /// Minimum proficiency filter (1–10).
    #[param(default)]
    pub min_proficiency: Option<i32>,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/skills",
    tag = "skills",
    params(ListSkillsQuery),
    responses(
        (status = 200, description = "List of skills", body = Vec<crate::memory::skills::AgentSkill>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_skills(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ListSkillsQuery>,
) -> Result<Json<Vec<crate::memory::skills::AgentSkill>>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.list(q.category.as_deref(), q.min_proficiency).await {
        Ok(skills) => Ok(Json(skills)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /skills/{id} — get a single skill by ID.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/skills/{id}",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    responses(
        (status = 200, description = "Skill found", body = crate::memory::skills::AgentSkill),
        (status = 404, description = "Skill not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn get_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::memory::skills::AgentSkill>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.get(id).await {
        Ok(Some(skill)) => Ok(Json(skill)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("skill {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// PUT /skills/{id} — update a skill.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    put,
    path = "/skills/{id}",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    request_body = crate::memory::skills::UpdateSkillRequest,
    responses(
        (status = 200, description = "Skill updated"),
        (status = 404, description = "Skill not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn update_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<crate::memory::skills::UpdateSkillRequest>,
) -> Result<Json<crate::memory::skills::UpdateSkillResponse>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.update(id, &req).await {
        Ok(()) => Ok(Json(crate::memory::skills::UpdateSkillResponse {
            message: format!("skill {id} updated"),
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// DELETE /skills/{id} — delete a skill.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    delete,
    path = "/skills/{id}",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    responses(
        (status = 200, description = "Skill deleted"),
        (status = 404, description = "Skill not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::memory::skills::UpdateSkillResponse>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.delete(id).await {
        Ok(()) => Ok(Json(crate::memory::skills::UpdateSkillResponse {
            message: format!("skill {id} deleted"),
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// POST /skills/{id}/use — record a skill usage event.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct UseSkillQuery {
    /// Whether the skill usage was successful (default: true).
    #[serde(default = "default_success")]
    pub success: bool,
}

    #[allow(dead_code)]
fn default_success() -> bool {
    true
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct UseSkillResponse {
    pub message: String,
    pub skill_id: Uuid,
    pub success: bool,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/skills/{id}/use",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    responses(
        (status = 200, description = "Usage recorded", body = UseSkillResponse),
        (status = 404, description = "Skill not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn use_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<UseSkillQuery>,
) -> Result<Json<UseSkillResponse>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.mark_used(id, q.success).await {
        Ok(()) => Ok(Json(UseSkillResponse {
            message: format!("skill {} marked as used (success={})", id, q.success),
            skill_id: id,
            success: q.success,
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("0 rows") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// GET /skills/match — find skills relevant to a task description.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct MatchSkillsQuery {
    /// Free-text task description.
    pub task: String,
    /// Maximum number of results (default 5).
    #[param(default = 5)]
    pub top_k: usize,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/skills/match",
    tag = "skills",
    params(MatchSkillsQuery),
    responses(
        (status = 200, description = "Matching skills", body = Vec<crate::memory::skills::AgentSkill>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn match_skills(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<MatchSkillsQuery>,
) -> Result<Json<Vec<crate::memory::skills::AgentSkill>>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into())),
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.match_task(&q.task, q.top_k).await {
        Ok(skills) => Ok(Json(skills)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// VLM Worker Routes — Summarization via Background Worker
// ---------------------------------------------------------------------------

/// Request body for enqueuing a summarization job.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VlmEnqueueRequest {
    /// IDs of memory nodes to summarize.
    pub node_ids: Vec<Uuid>,
    /// Context level for the summary.
    #[serde(default)]
    pub context: SummaryContext,
    /// Optional priority (1–10, higher = processed first). Default 5.
    #[serde(default = "default_vlm_priority")]
    pub priority: u8,
}

fn default_vlm_priority() -> u8 {
    5
}

/// Response after enqueuing a job.
#[derive(Serialize, ToSchema)]
pub struct VlmEnqueueResponse {
    pub job_id: Uuid,
    pub queue_depth: usize,
}

/// GET /vlm/status — Worker queue status.
#[utoipa::path(
    get,
    path = "/vlm/status",
    tag = "vlm",
    responses(
        (status = 200, description = "VLM worker status", body = VlmWorkerStatus),
        (status = 503, description = "VLM worker not configured", body = String)
    )
)]
pub async fn vlm_status(
    State(state): State<AppState>,
) -> Result<Json<VlmWorkerStatus>, (StatusCode, String)> {
    match &state.vlm_worker {
        Some(handle) => {
            let status = handle.status().await;
            Ok(Json(status))
        }
        None => Err((StatusCode::SERVICE_UNAVAILABLE, "VLM worker not configured (set OPENAI_API_KEY or GROK_API_KEY)".into())),
    }
}

/// POST /vlm/summarize — Enqueue a summarization job (non-blocking).
#[utoipa::path(
    post,
    path = "/vlm/summarize",
    tag = "vlm",
    request_body = VlmEnqueueRequest,
    responses(
        (status = 202, description = "Job enqueued", body = VlmEnqueueResponse),
        (status = 400, description = "Invalid request", body = String),
        (status = 503, description = "VLM worker not configured", body = String)
    )
)]
pub async fn vlm_enqueue(
    State(state): State<AppState>,
    Json(req): Json<VlmEnqueueRequest>,
) -> Result<(StatusCode, Json<VlmEnqueueResponse>), (StatusCode, String)> {
    let handle = state.vlm_worker
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "VLM worker not configured".into()))?;

    if req.node_ids.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "node_ids must not be empty".into()));
    }

    if req.priority == 0 || req.priority > 10 {
        return Err((StatusCode::BAD_REQUEST, "priority must be 1–10".into()));
    }

    let job = VlmJob::new(req.node_ids.clone(), req.context)
        .with_priority(req.priority);

    let job_id = job.id;

    handle.enqueue(job).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let status = handle.status().await;

    tracing::info!(job_id = %job_id, queue_depth = status.queue_depth, "VLM job enqueued");

    Ok((StatusCode::ACCEPTED, Json(VlmEnqueueResponse {
        job_id,
        queue_depth: status.queue_depth,
    })))
}

// -- Frigate Webhook --

use crate::connectors::store_external_event;
use crate::connectors::ExternalEvent;

/// Frigate event payload from webhook POST body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FrigateWebhookEvent {
    /// Unique event ID from Frigate (used for deduplication).
    pub id: String,
    /// Camera name that captured the event.
    #[serde(default)]
    pub camera: String,
    /// Detected label (e.g., "person", "car").
    #[serde(default)]
    pub label: String,
    /// Confidence/top score of the detection.
    #[serde(default)]
    pub top_score: f64,
    /// Pointer to the snapshot image.
    #[serde(default)]
    pub snapshot_path: Option<String>,
    /// Pointer to the clip video.
    #[serde(default)]
    pub clip_path: Option<String>,
}

impl FrigateWebhookEvent {
    /// Build a pointer URI for this event.
    fn pointer(&self) -> String {
        format!("frigate://cameras/{}/events/{}", self.camera, self.id)
    }

    /// Build multimodal data if snapshot or clip is available.
    fn multimodal(&self) -> Option<MultimodalData> {
        if let Some(ref path) = self.snapshot_path {
            return Some(MultimodalData::Image {
                pointer: path.clone(),
                embedding: vec![],
            });
        }
        if let Some(ref path) = self.clip_path {
            // Frigate clip path — treat as Image for now (could be Audio/Video later)
            return Some(MultimodalData::Image {
                pointer: path.clone(),
                embedding: vec![],
            });
        }
        None
    }

    /// Convert to an ExternalEvent for storage.
    fn to_external_event(self) -> ExternalEvent {
        use serde_json::json;
        // IMPORTANT: borrow methods first (pointer, multimodal), then move fields
        let pointer = self.pointer();
        let multimodal = self.multimodal();
        let camera = self.camera;
        let label = self.label;
        let top_score = self.top_score;
        ExternalEvent {
            pointer,
            metadata: std::collections::HashMap::from([
                ("source".to_string(), json!("frigate")),
                ("camera".to_string(), json!(camera)),
                ("label".to_string(), json!(label)),
                ("score".to_string(), json!(top_score)),
            ]),
            multimodal,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct WebhookResponse {
    pub status: String,
    pub event_id: String,
}

#[utoipa::path(
    post,
    path = "/webhooks/frigate",
    tag = "webhooks",
    params(
        ("secret" = Option<String>, Query, description = "Webhook secret (alternative to X-Webhook-Secret header)")
    ),
    request_body = FrigateWebhookEvent,
    responses(
        (status = 200, description = "Event stored", body = WebhookResponse),
        (status = 401, description = "Unauthorized — invalid or missing secret", body = String),
        (status = 409, description = "Duplicate event — already processed", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn webhook_frigate(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<FrigateWebhookEvent>,
) -> Result<Json<WebhookResponse>, (StatusCode, String)> {
    // 1. Authenticate via secret (read once at startup from AppState)
    let webhook_secret = state.frigate_webhook_secret.as_deref();
    let header_secret = headers
        .get("X-Webhook-Secret")
        .and_then(|v| v.to_str().ok());
    let query_secret = params.get("secret").map(|s| s.as_str());

    if !check_webhook_secret(webhook_secret, header_secret, query_secret) {
        tracing::warn!("frigate webhook: unauthorized (bad secret)");
        return Err((StatusCode::UNAUTHORIZED, "invalid or missing webhook secret".into()));
    }

    // 2. Deduplicate by event_id
    let event_id = payload.id.clone();
    let dedup_key = format!("frigate:{}", event_id);
    if state.frigate_dedup.seen_or_insert(&dedup_key).await {
        tracing::debug!(event_id = %event_id, "frigate webhook: duplicate event");
        return Err((StatusCode::CONFLICT, format!("event {} already processed", event_id)));
    }

    // 3. Build external event and store
    let event = payload.to_external_event();
    match store_external_event(state.store.as_ref(), &state.embedding, event).await {
        Ok(id) => {
            tracing::info!(event_id = %event_id, node_id = %id, "frigate webhook event stored");
            Ok(Json(WebhookResponse {
                status: "stored".into(),
                event_id,
            }))
        }
        Err(e) => {
            tracing::error!(event_id = %event_id, error = %e, "frigate webhook: failed to store");
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("store failed: {e}")))
        }
    }
}

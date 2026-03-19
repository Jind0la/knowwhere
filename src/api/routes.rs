use std::collections::HashMap;
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
use crate::memory::types::{ConflictState, MemorySource, MemoryStatus, MemoryType, Sensitivity};
use crate::memory::{
    DreamMode, Event, FractalNode, GovernancePolicy, GovernanceValidator, InMemoryEventStore,
};
use crate::multimodal::MultimodalData;

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
        out.truncate(out.floor_char_boundary(1024));
        tracing::debug!(
            original_len,
            truncated_to = out.len(),
            "clean_for_embedding: text truncated for embedding"
        );
    }
    out
}
use crate::storage::MemoryStore;

#[derive(Clone)]
pub struct AppState {
    pub store: MemoryStore,
    pub dream: DreamMode,
    pub embedding: Arc<dyn EmbeddingProvider>,
    /// Active governance policy for Stage 2 retrieval validation.
    #[serde(skip)]
    pub governance_policy: GovernancePolicy,
    /// In-memory event store for Layer 0 (appended to on each mutation).
    /// For production with multiple nodes, use PostgresStore instead.
    pub events: InMemoryEventStore,
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
    let vector = match req.vector {
        Some(v) if !v.is_empty() => v,
        _ => {
            let embed_text = clean_for_embedding(&req.content);
            embed_document(&*state.embedding, &embed_text)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("auto-embed failed: {e}")))?
        }
    };

    let memory_type = MemoryType::from_str(&req.memory_type)
        .unwrap_or(MemoryType::Episodic);
    let source = MemorySource::from_str(&req.source)
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

    let memory_type = MemoryType::from_str(&req.memory_type)
        .unwrap_or(MemoryType::Semantic);
    let source = MemorySource::from_str(&req.source)
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
    pub query_vector: Vec<f32>,
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
) -> Json<Vec<ScoredNode>> {
    tracing::info!(
        top_k = req.top_k,
        max_depth = req.max_depth,
        has_query_text = req.query_text.is_some(),
        governance = req.governance_enabled,
        "fractal retrieve"
    );

    // Stage 1: Hybrid retrieval
    let results = state
        .store
        .hybrid_retrieve(
            req.query_text.as_deref(),
            &req.query_vector,
            req.top_k,
            req.max_depth,
        )
        .await;

    if !req.governance_enabled {
        let scored: Vec<ScoredNode> = results
            .into_iter()
            .map(|(score, node)| ScoredNode::from_node(score, node))
            .collect();
        return Json(scored);
    }

    // Optional memory type filter
    let type_filter = req
        .memory_type_filter
        .as_ref()
        .and_then(|s| MemoryType::from_str(s));

    // Stage 2: Governance validation
    let validator = GovernanceValidator::new(state.governance_policy.clone());
    let mut scored: Vec<ScoredNode> = results
        .into_iter()
        .filter_map(|(score, node)| {
            // Apply optional memory type filter
            if let Some(ref filter) = type_filter {
                if node.memory_type != *filter {
                    return None;
                }
            }

            let candidate = node.to_governance_candidate();
            let validation = validator.validate(&candidate);

            // Hard-blocked nodes (superseded, restricted, invalid status, irrelevant)
            // are excluded from results entirely.
            if validation.has_hard_block() {
                tracing::debug!(node_id = %node.id, "excluded by governance: hard block");
                return None;
            }

            Some(ScoredNode::from_governed(score, node, validation.passed, validation.issues))
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

    Json(scored)
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
) -> Json<Vec<FractalNode>> {
    let limit = q.limit.min(100);
    Json(state.store.recent(limit).await)
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
    Json(state.dream.status().await)
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
            _ => GovernancePolicy::default(),
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

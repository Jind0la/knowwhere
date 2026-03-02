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
use crate::memory::{DreamMode, FractalNode, NodeType};
use crate::multimodal::MultimodalData;

#[derive(Serialize, ToSchema)]
pub struct ScoredNode {
    pub score: f32,
    pub id: Uuid,
    #[serde(default)]
    pub node_type: NodeType,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    #[schema(value_type = Object)]
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl ScoredNode {
    fn from_node(score: f32, n: FractalNode) -> Self {
        Self {
            score,
            id: n.id,
            node_type: n.node_type,
            content: n.content,
            original_pointer: n.original_pointer,
            metadata: n.metadata,
            created_at: n.created_at,
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
        out.truncate(out.floor_char_boundary(1024));
    }
    out
}
use crate::storage::MemoryStore;

#[derive(Clone)]
pub struct AppState {
    pub store: MemoryStore,
    pub dream: DreamMode,
    pub embedding: Arc<dyn EmbeddingProvider>,
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

    let node = FractalNode::new_session(req.content, vector, req.metadata);
    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%id, "session node stored");

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

    let node = match req.multimodal {
        Some(mm) => FractalNode::new_external_multimodal(req.pointer, vector, req.metadata, mm),
        None => FractalNode::new_external(req.pointer, vector, req.metadata),
    };

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%id, "external pointer node stored");

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
}

fn default_top_k() -> usize {
    5
}
fn default_max_depth() -> usize {
    3
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
        "fractal retrieve"
    );
    let results = state
        .store
        .hybrid_retrieve(
            req.query_text.as_deref(),
            &req.query_vector,
            req.top_k,
            req.max_depth,
        )
        .await;
    let scored: Vec<ScoredNode> = results
        .into_iter()
        .map(|(score, node)| ScoredNode::from_node(score, node))
        .collect();
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

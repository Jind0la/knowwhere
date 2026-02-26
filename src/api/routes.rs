use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::dream::DreamStatus;
use crate::memory::{DreamMode, FractalNode};
use crate::multimodal::MultimodalData;
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
    let vector = state
        .embedding
        .embed(&req.text)
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
        _ => state
            .embedding
            .embed(&req.content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("auto-embed failed: {e}")))?,
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
                    state.embedding.embed(&req.pointer).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("auto-embed failed: {e}"))
                    })?
                }
            } else {
                state.embedding.embed(&req.pointer).await.map_err(|e| {
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
        (status = 200, description = "Fractal retrieval results", body = Vec<FractalNode>)
    )
)]
pub async fn retrieve_fractal(
    State(state): State<AppState>,
    Json(req): Json<RetrieveFractalRequest>,
) -> Json<Vec<FractalNode>> {
    tracing::info!(top_k = req.top_k, max_depth = req.max_depth, "fractal retrieve");
    let results = state
        .store
        .retrieve_fractal(&req.query_vector, req.top_k, req.max_depth)
        .await;
    Json(results)
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

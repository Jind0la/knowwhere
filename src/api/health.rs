use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::types::AppState;
use crate::embedding::embed_query;

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

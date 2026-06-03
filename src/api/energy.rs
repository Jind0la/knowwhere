use std::collections::HashMap;
#[cfg(feature = "postgres-storage")]
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api::types::*;
use crate::embedding::{embed_document, embed_document_batch, embed_query};
use crate::memory::FractalNode;
#[cfg(feature = "postgres-storage")]
use crate::memory::dream::energy_decay::{MemoryEnergyInfo, DecayResult, CompressionResult};
#[cfg(feature = "postgres-storage")]
use crate::services::lifecycle::LifecycleService;
use crate::storage::RetrievalProfile;


// =============================================================================
// Energy Decay Routes (Ebbinghaus forgetting curve)
// =============================================================================

#[cfg(feature = "postgres-storage")]
fn lifecycle_service(state: &AppState) -> Result<LifecycleService, (StatusCode, String)> {
    let pool = state.trajectory_pool.as_ref().cloned().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "postgres-storage not configured".into(),
    ))?;
    Ok(LifecycleService::new(pool))
}

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
    let service = lifecycle_service(&state)?;
    match service.boost_energy(id, req.boost).await {
        Ok(()) => Ok(Json(BoostEnergyResponse {
            memory_id: id,
            boost: req.boost,
            message: format!("energy boosted by {}", req.boost),
        })),
        Err(e) => {
            if e.to_string().contains("0 rows") {
                Err((
                    StatusCode::NOT_FOUND,
                    format!("memory {} not found or not active", id),
                ))
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
    let service = lifecycle_service(&state)?;
    match service.list_low_energy(query.limit).await {
        Ok(memories) => Ok(Json(memories)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// POST /energy/decay — apply energy decay to all active memories
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/energy/decay",
    tag = "dream",
    responses(
        (status = 200, description = "Decay applied", body = DecayResult),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn apply_energy_decay(
    State(state): State<AppState>,
) -> Result<Json<DecayResult>, (StatusCode, String)> {
    let service = lifecycle_service(&state)?;
    match service.apply_decay().await {
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
    if req.memory_ids.len() < 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            "need at least 2 memory IDs to compress".into(),
        ));
    }

    let service = lifecycle_service(&state)?;
    match service.compress_cluster(&req.memory_ids).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

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
use crate::storage::RetrievalProfile;


// =============================================================================
// Conflict Detection Endpoints
// =============================================================================

#[cfg(feature = "postgres-storage")]
use crate::memory::dream::conflict_detection::{ConflictDetector, ConflictGroup};
#[cfg(feature = "postgres-storage")]
use crate::memory::dream::deduplication::{
    DeduplicationResult, DeduplicationRunRow, DeduplicationWorker, DuplicatePair,
};
#[cfg(feature = "postgres-storage")]
use crate::memory::dream::energy_decay::{CompressionResult, DecayResult, MemoryEnergyInfo};
#[cfg(feature = "postgres-storage")]
use crate::services::lifecycle::LifecycleService;

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
pub async fn list_conflicts(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConflictGroup>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
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

/// POST /conflicts/auto-resolve — automatically resolve pending entity conflicts
/// using source metadata heuristics.
///
/// Resolution rules (entity conflicts only):
/// - If only one memory has a source_memory_id → sourced memory wins
/// - If neither has a source → remains pending
/// - If both have sources → compare confidence (>1.5x) and recency (>30 days)
#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct AutoResolveResponse {
    /// Number of conflicts successfully resolved.
    pub resolved: usize,
    /// Number of conflicts that remain pending.
    pub remaining: usize,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/conflicts/auto-resolve",
    tag = "governance",
    responses(
        (status = 200, description = "Auto-resolution results", body = AutoResolveResponse),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn auto_resolve_conflicts(
    State(state): State<AppState>,
) -> Result<Json<AutoResolveResponse>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let detector = ConflictDetector::new(&pool);
    match detector.auto_resolve().await {
        Ok(result) => Ok(Json(AutoResolveResponse {
            resolved: result.resolved,
            remaining: result.remaining,
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

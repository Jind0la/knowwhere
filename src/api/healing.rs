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
// Self-Healing Endpoints (Content Hashing for External Nodes)
// =============================================================================

#[cfg(feature = "postgres-storage")]
use crate::memory::self_healing::{
    HealingStats, HealthCheckResult, RepairStatus, SelfHealingService,
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    // Fetch the memory to get its original_pointer
    let node = match state.store.get(&id).await {
        Ok(Some(n)) => n,
        Ok(None) => return Err((StatusCode::NOT_FOUND, format!("memory {id} not found"))),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let uri = node.original_pointer.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("memory {id} has no original_pointer (not an external node)"),
        )
    })?;

    // Convert URI to path
    let file_path: PathBuf = if uri.starts_with("file://") {
        PathBuf::from(&uri[7..])
    } else {
        PathBuf::from(uri)
    };

    if !file_path.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("pointer file does not exist: {}", file_path.display()),
        ));
    }

    // Get file_root from env or default
    let file_root = std::env::var("KNOWWHERE_FILE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));

    let service = SelfHealingService::new((*pool).clone(), file_root);
    service
        .index_external_node(id, &file_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reindex failed: {e}"),
            )
        })?;

    // Fetch updated hash/thumbnail for response
    let (content_hash, thumbnail_words) = {
        let row: (Option<String>, Option<String>) = sqlx::query_as(
            r#"
            SELECT content_hash, semantic_thumbnail
            FROM memories WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_one(match state.trajectory_pool.as_ref() {
            Some(arc) => &**arc,
            None => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "no trajectory pool".into(),
                )
                    .into())
            }
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        (
            row.0,
            row.1
                .as_ref()
                .map(|t| t.split_whitespace().count())
                .unwrap_or(0),
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
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

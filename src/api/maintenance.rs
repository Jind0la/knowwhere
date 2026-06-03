use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api::auth::AuthContext;
use crate::api::types::{clean_for_embedding, AppState};
use super::store::StoreNodeResponse;
use crate::embedding::{embed_document, embed_document_batch, EmbeddingProvider};
use crate::memory::dream::DreamStatus;
use crate::memory::types::{MemoryType, Sensitivity};
use crate::memory::FractalNode;

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
            chunk_ids: None,
        })),
        Ok(false) => Err((StatusCode::NOT_FOUND, format!("node {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// -- Batch Delete Nodes --

#[derive(Deserialize, ToSchema)]
pub struct BatchDeleteRequest {
    pub ids: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct BatchDeleteResponse {
    pub deleted: usize,
    pub not_found: usize,
}

#[utoipa::path(
    post,
    path = "/nodes/batch_delete",
    tag = "memory",
    request_body = BatchDeleteRequest,
    responses(
        (status = 200, description = "Nodes deleted", body = BatchDeleteResponse),
        (status = 500, description = "Internal error")
    )
)]
pub async fn batch_delete_nodes(
    State(state): State<AppState>,
    Json(req): Json<BatchDeleteRequest>,
) -> Result<Json<BatchDeleteResponse>, (StatusCode, String)> {
    let mut deleted = 0usize;
    let mut not_found = 0usize;
    for id_str in &req.ids {
        match Uuid::parse_str(id_str) {
            Ok(id) => match state.store.delete(&id).await {
                Ok(true) => deleted += 1,
                Ok(false) => not_found += 1,
                Err(_) => not_found += 1,
            },
            Err(_) => not_found += 1,
        }
    }
    tracing::info!(
        deleted,
        not_found,
        total = req.ids.len(),
        "batch delete complete"
    );
    Ok(Json(BatchDeleteResponse { deleted, not_found }))
}

// -- Deduplicate by external_id --

#[derive(Serialize, ToSchema)]
pub struct DedupResponse {
    pub total_nodes: usize,
    pub groups: usize,
    pub duplicates_removed: usize,
    pub errors: usize,
}

#[utoipa::path(
    post,
    path = "/nodes/deduplicate",
    tag = "memory",
    responses(
        (status = 200, description = "Deduplication complete", body = DedupResponse)
    )
)]
pub async fn deduplicate_nodes(
    State(state): State<AppState>,
) -> Result<Json<DedupResponse>, (StatusCode, String)> {
    let all = state
        .store
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = all.len();

    // Group by external_id
    let mut by_eid: std::collections::HashMap<String, Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> =
        std::collections::HashMap::new();
    for node in &all {
        if let Some(eid) = node
            .metadata
            .get("external_id")
            .and_then(|v| v.as_str())
        {
            by_eid
                .entry(eid.to_string())
                .or_default()
                .push((node.id, node.created_at));
        }
    }

    let groups = by_eid.len();
    let mut to_delete = Vec::new();

    for (_eid, mut nodes) in by_eid {
        if nodes.len() > 1 {
            nodes.sort_by_key(|(_, ts)| *ts);
            // Keep first (oldest), delete rest
            for (id, _) in &nodes[1..] {
                to_delete.push(id.to_string());
            }
        }
    }

    let dup_count = to_delete.len();
    let mut removed = 0usize;
    let mut errors = 0usize;

    for id_str in &to_delete {
        match Uuid::parse_str(id_str) {
            Ok(id) => match state.store.delete(&id).await {
                Ok(true) => removed += 1,
                _ => errors += 1,
            },
            Err(_) => errors += 1,
        }
    }

    tracing::info!(
        total,
        groups,
        duplicates = dup_count,
        removed,
        errors,
        "deduplicate complete"
    );

    Ok(Json(DedupResponse {
        total_nodes: total,
        groups,
        duplicates_removed: removed,
        errors,
    }))
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
    let nodes = state.store.recent(limit).await.map_err(|e| {
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
pub async fn reembed_all(State(state): State<AppState>) -> Json<ReembedResponse> {
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
                if state
                    .store
                    .update_vector(&node.id, vec)
                    .await
                    .unwrap_or(false)
                {
                    updated += 1;
                } else {
                    failed += 1;
                }
            }
            Err(e) => {
                tracing::warn!(%node.id, "re-embed failed: {}", e);
                failed += 1;
            }
        }
    }

    Json(ReembedResponse {
        updated,
        failed,
        message: format!("re-embedded {updated} nodes, {failed} failed"),
    })
}

// -- Repair Embedding Dimensions (1536 → 1024) --

#[derive(Serialize, ToSchema)]
pub struct RepairEmbeddingsResponse {
    pub scanned: usize,
    pub repaired: usize,
    pub skipped: usize,
    pub target_dimension: usize,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/maintenance/repair_embeddings",
    tag = "maintenance",
    responses(
        (status = 200, description = "Embedding repair complete", body = RepairEmbeddingsResponse),
        (status = 403, description = "Admin only")
    )
)]
pub async fn repair_embeddings(
    State(state): State<AppState>,
    auth: axum::Extension<AuthContext>,
) -> Result<Json<RepairEmbeddingsResponse>, StatusCode> {
    // Admin-only: require admin API key
    if !auth.is_admin() {
        return Err(StatusCode::FORBIDDEN);
    }

    match state
        .store
        .repair_embedding_dimensions(&*state.embedding)
        .await
    {
        Ok(report) => Ok(Json(RepairEmbeddingsResponse {
            scanned: report.scanned,
            repaired: report.repaired,
            skipped: report.skipped,
            target_dimension: report.target_dimension,
            message: format!(
                "Repaired {} of {} memories ({} skipped). Target dimension: {}",
                report.repaired, report.scanned, report.skipped, report.target_dimension
            ),
        })),
        Err(e) => {
            tracing::error!("Embedding repair failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
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

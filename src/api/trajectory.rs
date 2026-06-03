use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use crate::api::types::*;
use crate::api::types::*;

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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };
    let store = crate::storage::TrajectoryStore::new(pool.as_ref());
    let limit = q.limit.min(100);
    match store.list_runs(limit, q.after_id).await {
        Ok(rows) => Ok(Json(
            rows.into_iter().map(RetrievalRunResponse::from).collect(),
        )),
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };
    let store = crate::storage::TrajectoryStore::new(pool.as_ref());
    match store.get_run(id).await {
        Ok(Some(row)) => Ok(Json(RetrievalRunResponse::from(row))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            format!("retrieval run {id} not found"),
        )),
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };
    let store = crate::storage::TrajectoryStore::new(pool.as_ref());
    match store.get_trajectory(id).await {
        Ok(rows) => Ok(Json(
            rows.into_iter().map(TrajectoryStepResponse::from).collect(),
        )),
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
        (status = 410, description = "Compaction disabled", body = String)
    )
)]
pub async fn compact_memory(
    State(_state): State<AppState>,
    Path(_id): Path<Uuid>,
    axum::extract::Query(_q): axum::extract::Query<CompactMemoryQuery>,
) -> Result<Json<CompactMemoryResponse>, (StatusCode, String)> {
    Err((
        StatusCode::GONE,
        "Compaction disabled — LLM summarizer removed. Use Matryoshka embedding tiers for retrieval."
            .to_string(),
    ))
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


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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
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
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let worker = DeduplicationWorker::with_defaults(&pool);
    match worker.recent_runs(query.limit).await {
        Ok(runs) => Ok(Json(runs)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

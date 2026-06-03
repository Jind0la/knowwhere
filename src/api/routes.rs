use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
#[cfg(feature = "postgres-storage")]
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api::auth::AuthContext;
use crate::api::turns::{BatchTurnItem, PaginatedSessionTurns, ScoredTurn, SessionTurn, SessionTurnsResponse, TurnContext};
use crate::api::webhooks::{check_webhook_secret, DedupCache};
use crate::embedding::router::EmbeddingRouter;
use crate::embedding::{embed_document, embed_document_batch, embed_query, EmbeddingProvider};
use crate::memory::dream::DreamStatus;
#[cfg(feature = "postgres-storage")]
use crate::memory::skills::CreateSkillResponse;
use crate::memory::types::{ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity};
use crate::memory::{
    DreamMode, Event, EventStore, FractalNode, GovernancePolicy, GovernanceValidator,
    InMemoryEventStore,
};
use crate::memory::fact_extraction::{FactExtractionContext, FactExtractor};
use crate::multimodal::MultimodalData;
use crate::storage::FusionStrategy;

#[path = "routes/governance_events.rs"]
mod governance_events;
pub use governance_events::*;
#[path = "routes/webhooks.rs"]
mod webhook_routes;
pub use webhook_routes::*;

#[path = "health.rs"]
mod health;
pub use health::*;

#[path = "store.rs"]
mod store;
pub use store::*;

#[path = "retrieve.rs"]
mod retrieve;
pub use retrieve::*;

#[path = "rerank.rs"]
mod rerank;
pub use rerank::*;

#[path = "maintenance.rs"]
mod maintenance;
pub use maintenance::*;

use crate::api::subconscious_qa::{
    is_multi_session_type, is_temporal_question, openai_qa_answer, qa_answer, qa_context_limit,
    source_context_block, source_timestamp,
};
use crate::api::types::*;

pub use crate::api::types::{
    clean_for_embedding, AppState, RetrievalScoreDebug, ScoredNode,
};



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

// =============================================================================
// Namespace Routes
// =============================================================================

/// GET /namespaces — list all namespaces.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces",
    tag = "namespaces",
    responses(
        (status = 200, description = "All namespaces", body = Vec<crate::memory::namespaces::MemoryNamespace>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_namespaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::memory::namespaces::MemoryNamespace>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.list_all().await {
        Ok(ns) => Ok(Json(ns)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /namespaces/{path} — get a namespace by path (e.g. `agent/skills`).
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces/{path}",
    tag = "namespaces",
    params(
        ("path" = String, Path, description = "Namespace path (e.g. agent/skills)")
    ),
    responses(
        (status = 200, description = "Namespace found"),
        (status = 404, description = "Namespace not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn get_namespace(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Result<Json<crate::memory::namespaces::MemoryNamespace>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.find_by_path(&path).await {
        Ok(Some(ns)) => Ok(Json(ns)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            format!("namespace '{path}' not found"),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /namespaces/{path}/memories — list memories within a namespace.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct NamespaceMemoriesQuery {
    #[param(default = 50)]
    pub limit: i32,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces/{path}/memories",
    tag = "namespaces",
    params(
        ("path" = String, Path, description = "Namespace path"),
        NamespaceMemoriesQuery
    ),
    responses(
        (status = 200, description = "Memories in this namespace"),
        (status = 404, description = "Namespace not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn namespace_memories(
    State(state): State<AppState>,
    Path(path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<NamespaceMemoriesQuery>,
) -> Result<Json<Vec<crate::memory::namespaces::MemoryRow>>, (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.find_by_path(&path).await {
        Ok(Some(ns)) => match store.memories_in_namespace(ns.id, q.limit).await {
            Ok(rows) => Ok(Json(rows)),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            format!("namespace '{path}' not found"),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// POST /namespaces — create a new namespace.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct CreateNamespaceRequest {
    pub path: String,
    pub description: Option<String>,
    pub memory_type_hint: Option<String>,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct CreateNamespaceResponse {
    pub id: Uuid,
    pub message: String,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/namespaces",
    tag = "namespaces",
    request_body = CreateNamespaceRequest,
    responses(
        (status = 201, description = "Namespace created", body = CreateNamespaceResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn create_namespace(
    State(state): State<AppState>,
    Json(req): Json<CreateNamespaceRequest>,
) -> Result<(StatusCode, Json<CreateNamespaceResponse>), (StatusCode, String)> {
    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let depth = req.path.matches('/').count() as i32 + 1;
    let ns = crate::memory::namespaces::MemoryNamespace {
        id: Uuid::new_v4(),
        path: req.path.clone(),
        depth,
        parent_id: None,
        description: req.description,
        memory_type_hint: req.memory_type_hint,
    };

    let store = crate::memory::namespaces::NamespaceStore::new(pool.as_ref());
    match store.create(&ns).await {
        Ok(id) => Ok((
            StatusCode::CREATED,
            Json(CreateNamespaceResponse {
                id,
                message: format!("namespace '{}' created", req.path),
            }),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /namespaces/{path}/search — search memories within a namespace.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct NamespaceSearchQuery {
    pub q: String,
    #[param(default = 10)]
    pub top_k: usize,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/namespaces/{path}/search",
    tag = "namespaces",
    params(
        ("path" = String, Path, description = "Namespace path"),
        NamespaceSearchQuery
    ),
    responses(
        (status = 200, description = "Search results from namespace", body = Vec<ScoredNode>)
    )
)]
pub async fn namespace_search(
    State(state): State<AppState>,
    Path(path): Path<String>,
    axum::extract::Query(q): axum::extract::Query<NamespaceSearchQuery>,
) -> Result<Json<Vec<ScoredNode>>, (StatusCode, String)> {
    use crate::memory::namespaces::NamespaceStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = NamespaceStore::new(pool.as_ref());

    // Resolve path to namespace ID
    let namespace_id = match store.find_by_path(&path).await {
        Ok(Some(ns)) => ns.id,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("namespace '{path}' not found"),
            ))
        }
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Get memory IDs in this namespace
    let memory_rows = match store.memories_in_namespace(namespace_id, 500).await {
        Ok(rows) => rows,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let memory_ids: Vec<Uuid> = memory_rows.into_iter().map(|r| r.id).collect();

    if memory_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    // Search in-memory store for those IDs with the query
    let query_text = q.q.clone();
    let query_vector = embed_query(&*state.embedding, &query_text)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("embed failed: {e}"),
            )
        })?;

    // Hybrid retrieve with the namespace constraint
    let query = HybridQuery {
        query_text: Some(query_text),
        query_vector: Some(query_vector),
        top_k: q.top_k,
        max_depth: 3,
        profile: RetrievalProfile::UserFacing,
        memory_type_filter: None,
        user_id: None,
        multi_query: false,
        recency_boost: None,
        temporal_weight: None,
        fusion_strategy: None,
        query_type_routing: false,
        source_type_weights: state.default_source_type_weights,
    };
    let all_results = state.store.hybrid_retrieve(&query).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("hybrid_retrieve failed: {e}"),
        )
    })?;

    // Filter to only memories in this namespace
    let filtered: Vec<ScoredNode> = all_results
        .into_iter()
        .filter(|s| memory_ids.contains(&s.node.id))
        .map(|entry| ScoredNode::from_storage(entry, false))
        .collect();

    Ok(Json(filtered))
}

// =============================================================================
// Skills Routes
// =============================================================================

/// POST /skills — create a new skill.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/skills",
    tag = "skills",
    request_body = crate::memory::skills::CreateSkillRequest,
    responses(
        (status = 201, description = "Skill created", body = CreateSkillResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn create_skill(
    State(state): State<AppState>,
    Json(req): Json<crate::memory::skills::CreateSkillRequest>,
) -> Result<(StatusCode, Json<crate::memory::skills::CreateSkillResponse>), (StatusCode, String)> {
    use crate::memory::skills::{CreateSkillResponse, SkillsStore};

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.create(&req).await {
        Ok(id) => Ok((
            StatusCode::CREATED,
            Json(CreateSkillResponse {
                id,
                message: format!("skill '{}' created", req.skill_name),
            }),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /skills — list all skills (filter by category or min_proficiency).
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct ListSkillsQuery {
    /// Filter by category (e.g. `language`, `tool`, `domain`, `framework`).
    #[param(default)]
    pub category: Option<String>,
    /// Minimum proficiency filter (1–10).
    #[param(default)]
    pub min_proficiency: Option<i32>,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/skills",
    tag = "skills",
    params(ListSkillsQuery),
    responses(
        (status = 200, description = "List of skills", body = Vec<crate::memory::skills::AgentSkill>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn list_skills(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ListSkillsQuery>,
) -> Result<Json<Vec<crate::memory::skills::AgentSkill>>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.list(q.category.as_deref(), q.min_proficiency).await {
        Ok(skills) => Ok(Json(skills)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /skills/{id} — get a single skill by ID.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/skills/{id}",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    responses(
        (status = 200, description = "Skill found", body = crate::memory::skills::AgentSkill),
        (status = 404, description = "Skill not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn get_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::memory::skills::AgentSkill>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.get(id).await {
        Ok(Some(skill)) => Ok(Json(skill)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("skill {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// PUT /skills/{id} — update a skill.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    put,
    path = "/skills/{id}",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    request_body = crate::memory::skills::UpdateSkillRequest,
    responses(
        (status = 200, description = "Skill updated"),
        (status = 404, description = "Skill not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn update_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<crate::memory::skills::UpdateSkillRequest>,
) -> Result<Json<crate::memory::skills::UpdateSkillResponse>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.update(id, &req).await {
        Ok(()) => Ok(Json(crate::memory::skills::UpdateSkillResponse {
            message: format!("skill {id} updated"),
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// DELETE /skills/{id} — delete a skill.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    delete,
    path = "/skills/{id}",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    responses(
        (status = 200, description = "Skill deleted"),
        (status = 404, description = "Skill not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::memory::skills::UpdateSkillResponse>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.delete(id).await {
        Ok(()) => Ok(Json(crate::memory::skills::UpdateSkillResponse {
            message: format!("skill {id} deleted"),
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// POST /skills/{id}/use — record a skill usage event.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct UseSkillQuery {
    /// Whether the skill usage was successful (default: true).
    #[serde(default = "default_success")]
    pub success: bool,
}

#[allow(dead_code)]
fn default_success() -> bool {
    true
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct UseSkillResponse {
    pub message: String,
    pub skill_id: Uuid,
    pub success: bool,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/skills/{id}/use",
    tag = "skills",
    params(
        ("id" = Uuid, Path, description = "Skill UUID")
    ),
    responses(
        (status = 200, description = "Usage recorded", body = UseSkillResponse),
        (status = 404, description = "Skill not found", body = String),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn use_skill(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<UseSkillQuery>,
) -> Result<Json<UseSkillResponse>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.mark_used(id, q.success).await {
        Ok(()) => Ok(Json(UseSkillResponse {
            message: format!("skill {} marked as used (success={})", id, q.success),
            skill_id: id,
            success: q.success,
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("0 rows") {
                Err((StatusCode::NOT_FOUND, msg))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
            }
        }
    }
}

/// GET /skills/match — find skills relevant to a task description.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, IntoParams)]
pub struct MatchSkillsQuery {
    /// Free-text task description.
    pub task: String,
    /// Maximum number of results (default 5).
    #[param(default = 5)]
    pub top_k: usize,
}

#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/skills/match",
    tag = "skills",
    params(MatchSkillsQuery),
    responses(
        (status = 200, description = "Matching skills", body = Vec<crate::memory::skills::AgentSkill>),
        (status = 503, description = "postgres-storage not configured", body = String)
    )
)]
pub async fn match_skills(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<MatchSkillsQuery>,
) -> Result<Json<Vec<crate::memory::skills::AgentSkill>>, (StatusCode, String)> {
    use crate::memory::skills::SkillsStore;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let store = SkillsStore::new(pool.as_ref());
    match store.match_task(&q.task, q.top_k).await {
        Ok(skills) => Ok(Json(skills)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /entities — search entity edges
#[cfg(feature = "postgres-storage")]
pub async fn entity_search(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<EntitySearchParams>,
) -> Result<Json<Vec<EntityEdge>>, (StatusCode, String)> {
    use sqlx::Row;

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => return Err((StatusCode::SERVICE_UNAVAILABLE, "PostgreSQL not configured".into())),
    };

    let query = if let Some(ref entity_type) = params.entity_type {
        if let Some(ref relation) = params.relation {
            "SELECT id, source_node_id, target_node_id, entity_type, entity_name, relation_type, confidence, extracted_at FROM entity_edges WHERE entity_type = $1 AND relation_type = $2 ORDER BY confidence DESC LIMIT $3"
        } else {
            "SELECT id, source_node_id, target_node_id, entity_type, entity_name, relation_type, confidence, extracted_at FROM entity_edges WHERE entity_type = $1 ORDER BY confidence DESC LIMIT $2"
        }
    } else {
        "SELECT id, source_node_id, target_node_id, entity_type, entity_name, relation_type, confidence, extracted_at FROM entity_edges ORDER BY confidence DESC LIMIT $1"
    };

    let limit = params.limit.unwrap_or(50).min(200) as i64;
    let rows: Vec<sqlx::postgres::PgRow> = if let Some(ref entity_type) = params.entity_type {
        if let Some(ref relation) = params.relation {
            sqlx::query(query).bind(entity_type).bind(relation).bind(limit).fetch_all(pool.as_ref()).await
        } else {
            sqlx::query(query).bind(entity_type).bind(limit).fetch_all(pool.as_ref()).await
        }
    } else {
        sqlx::query(query).bind(limit).fetch_all(pool.as_ref()).await
    }.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let edges: Vec<EntityEdge> = rows.iter().map(|row| EntityEdge {
        id: row.get("id"),
        source_node_id: row.get("source_node_id"),
        target_node_id: row.get("target_node_id"),
        entity_type: row.get("entity_type"),
        entity_name: row.get("entity_name"),
        relation_type: row.get("relation_type"),
        confidence: row.get("confidence"),
        extracted_at: row.get("extracted_at"),
    }).collect();

    Ok(Json(edges))
}

#[derive(Deserialize)]
pub struct EntitySearchParams {
    entity_type: Option<String>,
    relation: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct EntityEdge {
    id: Uuid,
    source_node_id: Uuid,
    target_node_id: Option<Uuid>,
    entity_type: String,
    entity_name: String,
    relation_type: String,
    confidence: f64,
    extracted_at: Option<chrono::DateTime<chrono::Utc>>,
}


// =============================================================================
// Turn-Level Routes (per-turn embedding pipeline)
// =============================================================================

/// Validate that a speaker_role string is one of the allowed values.
fn validate_speaker_role(role: &str) -> Result<&str, (StatusCode, String)> {
    match role {
        "user" | "assistant" | "system" | "tool" => Ok(role),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("invalid speaker_role '{}': expected user, assistant, system, or tool", other),
        )),
    }
}

#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct StoreTurnRequest {
    pub session_id: String,
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct StoreTurnResponse {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    pub turn_index: i32,
    pub message: String,
}

/// POST /store_turn — Store a single conversational turn with per-turn embedding.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/store_turn",
    tag = "turn-level",
    request_body = StoreTurnRequest,
    responses(
        (status = 201, description = "Turn stored", body = StoreTurnResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Embedding or storage failed"),
        (status = 503, description = "postgres-storage not configured")
    )
)]
pub async fn store_turn(
    State(state): State<AppState>,
    Json(req): Json<StoreTurnRequest>,
) -> Result<(StatusCode, Json<StoreTurnResponse>), (StatusCode, String)> {
    let pg = state.pg_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "postgres-storage not configured".into(),
    ))?;
    if req.content.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "content is empty".into()));
    }
    let speaker = validate_speaker_role(&req.speaker_role)?;
    let cleaned = clean_for_embedding(&req.content);
    let vector = match req.vector {
        Some(v) if !v.is_empty() => v,
        _ => embed_document(&*state.embedding, &cleaned)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("embed failed: {e}")))?,
    };
    let emb_type = state.embedding.name().to_string();
    let emb_dim = state.embedding.dimension() as i32;
    let turn_id = pg
        .store_turn(&req.session_id, req.turn_index, speaker, &cleaned, vector, req.metadata, &emb_type, emb_dim)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let session_id = pg
        .find_or_create_session(&req.session_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(StoreTurnResponse {
        turn_id, session_id, turn_index: req.turn_index, message: "turn stored".into(),
    })))
}

#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct StoreTurnsBatchRequest {
    pub session_id: String,
    pub turns: Vec<BatchTurnItem>,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct StoreTurnsBatchResponse {
    pub session_id: Uuid,
    pub turn_ids: Vec<Uuid>,
    pub count: usize,
    pub message: String,
}

/// POST /store_turns — Store multiple turns in a single batch.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/store_turns",
    tag = "turn-level",
    request_body = StoreTurnsBatchRequest,
    responses(
        (status = 201, description = "Turns stored", body = StoreTurnsBatchResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Embedding or storage failed"),
        (status = 503, description = "postgres-storage not configured")
    )
)]
pub async fn store_turns_batch(
    State(state): State<AppState>,
    Json(req): Json<StoreTurnsBatchRequest>,
) -> Result<(StatusCode, Json<StoreTurnsBatchResponse>), (StatusCode, String)> {
    let pg = state.pg_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into(),
    ))?;
    if req.turns.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no turns provided".into()));
    }
    for item in &req.turns {
        validate_speaker_role(&item.speaker_role)?;
        if item.content.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "turn content is empty".into()));
        }
    }
    let cleaned: Vec<String> = req.turns.iter().map(|t| clean_for_embedding(&t.content)).collect();
    let refs: Vec<&str> = cleaned.iter().map(String::as_str).collect();
    let embeddings = embed_document_batch(&*state.embedding, &refs)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("batch embed failed: {e}")))?;
    let emb_type = state.embedding.name().to_string();
    let emb_dim = state.embedding.dimension() as i32;
    let (session_id, turn_ids) = pg
        .store_turns_batch(&req.session_id, &req.turns, embeddings, &emb_type, emb_dim)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(StoreTurnsBatchResponse {
        session_id, turn_ids, count: req.turns.len(), message: "turns batch stored".into(),
    })))
}

#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct RetrieveTurnsRequest {
    pub query_text: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub speaker_filter: Option<String>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
    #[serde(default)]
    pub external_session_id: Option<String>,
    #[serde(default)]
    pub context_window: Option<i32>,
}

#[cfg(feature = "postgres-storage")]
#[derive(Serialize, ToSchema)]
pub struct RetrieveTurnsResponse {
    pub query: String,
    pub results: Vec<ScoredTurn>,
    pub query_time_ms: u64,
}

/// POST /retrieve/turns — Retrieve turns by semantic similarity.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    post,
    path = "/retrieve/turns",
    tag = "turn-level",
    request_body = RetrieveTurnsRequest,
    responses(
        (status = 200, description = "Turn retrieval results", body = RetrieveTurnsResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Retrieval failed"),
        (status = 503, description = "postgres-storage not configured")
    )
)]
pub async fn retrieve_turns(
    State(state): State<AppState>,
    Json(req): Json<RetrieveTurnsRequest>,
) -> Result<Json<RetrieveTurnsResponse>, (StatusCode, String)> {
    let pg = state.pg_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into(),
    ))?;
    if req.query_text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "query_text is empty".into()));
    }
    if let Some(ref speaker) = req.speaker_filter {
        validate_speaker_role(speaker)?;
    }
    let start = std::time::Instant::now();
    let query_vector = embed_query(&*state.embedding, &req.query_text)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("embed failed: {e}")))?;
    let mut session_uuid_filter = req.session_id;
    if let Some(ref ext_id) = req.external_session_id {
        if session_uuid_filter.is_none() {
            match pg.find_or_create_session(ext_id).await {
                Ok(sid) => session_uuid_filter = Some(sid),
                Err(_) => {}
            }
        }
    }
    let mut results = pg
        .retrieve_turns(&query_vector, req.top_k.max(1).min(100), req.speaker_filter.as_deref(), session_uuid_filter)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(window) = req.context_window {
        if window > 0 {
            for turn in &mut results {
                if let Ok(adjacent) = pg.get_adjacent_turns(turn.session_id, turn.turn_index, window).await {
                    if !adjacent.is_empty() {
                        turn.adjacent_turns = Some(adjacent);
                    }
                }
            }
        }
    }
    let query_time_ms = start.elapsed().as_millis() as u64;
    Ok(Json(RetrieveTurnsResponse { query: req.query_text, results, query_time_ms }))
}

/// Query parameters for session turns listing.
#[cfg(feature = "postgres-storage")]
#[derive(Deserialize, ToSchema)]
pub struct SessionTurnsQuery {
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// Order direction: "asc" (default) or "desc".
    #[serde(default = "default_order")]
    pub order: String,
}

#[cfg(feature = "postgres-storage")]
fn default_order() -> String {
    "asc".to_string()
}

/// GET /sessions/:session_id/turns — Get turns for a session with optional pagination.
/// Without query params, returns all turns (backward compatible).
/// With ?offset=N&limit=M, returns a paginated window.
#[cfg(feature = "postgres-storage")]
#[utoipa::path(
    get,
    path = "/sessions/{session_id}/turns",
    tag = "turn-level",
    params(
        ("session_id" = Uuid, Path, description = "Session UUID"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Page size"),
        ("order" = Option<String>, Query, description = "Sort order: asc or desc"),
    ),
    responses(
        (status = 200, description = "Session turns", body = PaginatedSessionTurns),
        (status = 503, description = "postgres-storage not configured")
    )
)]
pub async fn get_session_turns(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(query): Query<SessionTurnsQuery>,
) -> Result<Json<PaginatedSessionTurns>, (StatusCode, String)> {
    let pg = state.pg_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE, "postgres-storage not configured".into(),
    ))?;

    let session_row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT external_id FROM conversation_sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(&pg.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let external_session_id = session_row.and_then(|r| r.0);

    let order_desc = query.order.to_lowercase() == "desc";
    let limit = query.limit.unwrap_or(1000).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);

    let (turns, total_count) = pg
        .list_turns_by_session(session_id, offset, limit, order_desc)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(PaginatedSessionTurns {
        session_id,
        external_session_id,
        turns,
        total_count,
        offset,
        limit,
    }))
}

#[cfg(test)]
mod chunking_tests {
    use super::chunk_into_rounds;

    #[test]
    fn chunk_empty_text_returns_single_empty_chunk() {
        let chunks = chunk_into_rounds("", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn chunk_single_line_no_prefix_returns_single_chunk() {
        let text = "Just a simple message without any role prefix";
        let chunks = chunk_into_rounds(text, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn chunk_user_assistant_pair_returns_two_chunks() {
        let text = "user: Hello there\nassistant: Hi! How can I help?";
        let chunks = chunk_into_rounds(text, 10);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Hello there"));
        assert!(chunks[1].contains("How can I help"));
    }

    #[test]
    fn chunk_multiple_rounds_returns_correct_count() {
        let text = "user: Question 1\nassistant: Answer 1\nuser: Question 2\nassistant: Answer 2\nuser: Question 3\nassistant: Answer 3";
        let chunks = chunk_into_rounds(text, 10);
        // Each role prefix starts a new chunk, so 6 chunks total
        assert_eq!(chunks.len(), 6);
    }

    #[test]
    fn chunk_human_ai_prefixes() {
        let text = "human: Hello\nai: Hi there\nhuman: Question";
        let chunks = chunk_into_rounds(text, 10);
        // Each role prefix starts a new chunk
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn chunk_merges_tiny_rounds_below_min_chars() {
        let text = "user: Hi\nassistant: Hello\nuser: Bye\nassistant: Goodbye";
        let chunks = chunk_into_rounds(text, 50);
        // With min_round_chars=50, tiny rounds should be merged
        assert!(chunks.len() <= 2);
    }

    #[test]
    fn chunk_no_role_prefixes_returns_original_text() {
        let text = "This is just a long text\nwith multiple lines\nbut no role prefixes at all";
        let chunks = chunk_into_rounds(text, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn chunk_preserves_multiline_content() {
        let text = "user: Line 1\nLine 2\nLine 3\nassistant: Response 1\nResponse 2";
        let chunks = chunk_into_rounds(text, 10);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Line 1"));
        assert!(chunks[0].contains("Line 3"));
        assert!(chunks[1].contains("Response 1"));
    }
}

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use crate::api::types::*;
use crate::api::types::*;

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

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
use crate::memory::skills::CreateSkillResponse;

use crate::storage::RetrievalProfile;

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

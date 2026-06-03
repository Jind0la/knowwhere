use std::collections::HashMap;
#[cfg(feature = "postgres-storage")]
use std::path::PathBuf;
use std::sync::Arc;

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

fn default_top_k() -> usize {
    5
}
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
use crate::api::turns::{BatchTurnItem, ScoredTurn, PaginatedSessionTurns, SessionTurnsResponse};

use crate::storage::RetrievalProfile;


// =============================================================================
// Turn-Level Routes (per-turn embedding pipeline)
// =============================================================================

/// Validate that a speaker_role string is one of the allowed values.









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
    use super::super::store::chunk_into_rounds;

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
        let text = "user: Hello there
assistant: Hi! How can I help?";
        let chunks = chunk_into_rounds(text, 10);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Hello there"));
        assert!(chunks[1].contains("How can I help"));
    }

    #[test]
    fn chunk_multiple_rounds_returns_correct_count() {
        let text = "user: Question 1
assistant: Answer 1
user: Question 2
assistant: Answer 2
user: Question 3
assistant: Answer 3";
        let chunks = chunk_into_rounds(text, 10);
        // Each role prefix starts a new chunk, so 6 chunks total
        assert_eq!(chunks.len(), 6);
    }

    #[test]
    fn chunk_human_ai_prefixes() {
        let text = "human: Hello
ai: Hi there
human: Question";
        let chunks = chunk_into_rounds(text, 10);
        // Each role prefix starts a new chunk
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn chunk_merges_tiny_rounds_below_min_chars() {
        let text = "user: Hi
assistant: Hello
user: Bye
assistant: Goodbye";
        let chunks = chunk_into_rounds(text, 50);
        // With min_round_chars=50, tiny rounds should be merged
        assert!(chunks.len() <= 2);
    }

    #[test]
    fn chunk_no_role_prefixes_returns_original_text() {
        let text = "This is just a long text
with multiple lines
but no role prefixes at all";
        let chunks = chunk_into_rounds(text, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn chunk_preserves_multiline_content() {
        let text = "user: Line 1
Line 2
Line 3
assistant: Response 1
Response 2";
        let chunks = chunk_into_rounds(text, 10);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Line 1"));
        assert!(chunks[0].contains("Line 3"));
        assert!(chunks[1].contains("Response 1"));
    }
}

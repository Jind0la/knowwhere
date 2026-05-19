//! Turn-level conversation storage — API types.
//!
//! These types bridge the PostgreSQL turn storage layer (`PostgresStore`)
//! and the HTTP API routes. They are the wire-format and route-response
//! structs for turn CRUD and retrieval.
//!
//! Reference: docs/turn-level-schema-design.md

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[cfg(feature = "postgres-storage")]
use crate::memory::conversation::EmbeddingInfo;

// =============================================================================
// Request types
// =============================================================================

/// A single turn item in a batch store request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BatchTurnItem {
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Request to store a single conversational turn.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StoreTurnRequest {
    pub session_id: String,
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Request to store multiple turns in a batch.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StoreTurnsRequest {
    pub session_id: String,
    pub turns: Vec<BatchTurnItem>,
}

/// Request to update an existing turn.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateTurnRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    /// Provider that generated the embedding (e.g. "local_ollama", "openai").
    /// Must be paired with embedding_dim when embedding is provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_type: Option<String>,
    /// Dimensionality of the embedding vector.
    /// Must be paired with embedding_type when embedding is provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_dim: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_role: Option<String>,
}

// =============================================================================
// Response types
// =============================================================================

/// A scored turn returned from vector-similarity retrieval.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScoredTurn {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    pub external_session_id: Option<String>,
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub similarity: f32,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    /// Embedding metadata (provider, dimension, model) — vector excluded to keep responses compact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg(feature = "postgres-storage")]
    pub embedding_info: Option<EmbeddingInfo>,
    /// Adjacent turns for context (if context_window was requested).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjacent_turns: Option<Vec<TurnContext>>,
}

/// A single turn in a session-reconstruction response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionTurn {
    pub turn_id: Uuid,
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub content_preview: String,
    pub token_count: Option<i32>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    /// Embedding metadata (provider, dimension, model) — vector excluded to keep responses compact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg(feature = "postgres-storage")]
    pub embedding_info: Option<EmbeddingInfo>,
}

/// Response containing all turns for a session, ordered by turn_index.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionTurnsResponse {
    pub session_id: Uuid,
    pub external_session_id: Option<String>,
    pub turns: Vec<SessionTurn>,
}

/// Paginated response containing turns for a session with offset/limit metadata.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PaginatedSessionTurns {
    pub session_id: Uuid,
    pub external_session_id: Option<String>,
    pub turns: Vec<SessionTurn>,
    pub total_count: i64,
    pub offset: i64,
    pub limit: i64,
}

/// An adjacent turn for context-window expansion.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TurnContext {
    pub turn_id: Uuid,
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

/// Response after storing a single turn.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct StoreTurnResponse {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    /// 0-based turn index within the session (first turn = 0).
    pub turn_index: i32,
    pub message: String,
}

//! Turn-Level Conversation Storage Types
//!
//! Defines the data model for first-class turn storage, replacing
//! implicit session-level chunking with explicit per-turn entities.
//! Each turn has its own embedding and stable identity.
//!
//! Reference: docs/turn-level-schema-design.md

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Structured embedding vector with provider metadata.
///
/// Wraps the raw embedding vector with type information (which provider
/// generated it), dimensionality, and optional provider-specific metadata.
/// This replaces bare `Vec<f32>` in turn records so consumers can
/// distinguish between embeddings from different models or providers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingInfo {
    /// The embedding vector itself.
    pub vector: Vec<f32>,
    /// Provider that generated this embedding (e.g. "local_ollama", "openai").
    pub provider: String,
    /// Dimensionality of the embedding vector.
    pub dimension: usize,
    /// Additional provider-specific metadata (model name, latency, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Who spoke in a conversation turn.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerRole {
    User,
    Assistant,
    System,
    Tool,
}

impl SpeakerRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpeakerRole::User => "user",
            SpeakerRole::Assistant => "assistant",
            SpeakerRole::System => "system",
            SpeakerRole::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(SpeakerRole::User),
            "assistant" => Some(SpeakerRole::Assistant),
            "system" => Some(SpeakerRole::System),
            "tool" => Some(SpeakerRole::Tool),
            _ => None,
        }
    }

    /// Infer speaker role from content prefix patterns.
    /// Used during migration of legacy session-chunk data.
    pub fn infer_from_content(content: &str) -> Self {
        let lower = content.trim().to_ascii_lowercase();
        if lower.starts_with("user:") || lower.starts_with("human:") {
            SpeakerRole::User
        } else if lower.starts_with("assistant:") || lower.starts_with("ai:") {
            SpeakerRole::Assistant
        } else if lower.starts_with("system:") {
            SpeakerRole::System
        } else if lower.starts_with("tool:") || lower.starts_with("function:") {
            SpeakerRole::Tool
        } else {
            // Heuristic: if content mentions "I" or "you" early, likely assistant
            if lower.contains("i can") || lower.contains("i'll") || lower.contains("here is") {
                SpeakerRole::Assistant
            } else {
                SpeakerRole::User
            }
        }
    }
}

/// A single conversation session (one chat, one thread).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConversationSession {
    pub id: Uuid,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub participant_count: i32,
    pub turn_count: i32,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single turn within a conversation session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConversationTurn {
    pub id: Uuid,
    pub session_id: Uuid,
    pub turn_index: i32,
    pub speaker_role: SpeakerRole,
    pub content: String,
    /// First 500 chars of content (stored column).
    pub content_preview: Option<String>,
    /// Embedding vector with provider metadata (type, dimension).
    pub embedding: Option<EmbeddingInfo>,
    pub token_count: Option<i32>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A scored turn result from retrieval.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredTurn {
    pub turn: ConversationTurn,
    pub similarity: f32,
    pub session_external_id: Option<String>,
    /// Adjacent turns for context (if context_window was requested).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub adjacent_turns: Vec<ConversationTurn>,
}

// =============================================================================
// Database row types (SQLx FromRow)
// =============================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub participant_count: i32,
    pub turn_count: i32,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<SessionRow> for ConversationSession {
    fn from(row: SessionRow) -> Self {
        ConversationSession {
            id: row.id,
            external_id: row.external_id,
            title: row.title,
            participant_count: row.participant_count,
            turn_count: row.turn_count,
            started_at: row.started_at,
            ended_at: row.ended_at,
            metadata: row.metadata,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TurnRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub turn_index: i32,
    pub speaker_role: String,
    pub content: String,
    pub content_preview: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub embedding_type: Option<String>,
    pub embedding_dim: Option<i32>,
    pub token_count: Option<i32>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl TurnRow {
    pub fn to_turn(&self) -> ConversationTurn {
        let embedding_info = self.embedding.as_ref().map(|vec| EmbeddingInfo {
            vector: vec.clone(),
            provider: self.embedding_type.clone().unwrap_or_else(|| "unknown".to_string()),
            dimension: self.embedding_dim.unwrap_or(0) as usize,
            metadata: None,
        });
        ConversationTurn {
            id: self.id,
            session_id: self.session_id,
            turn_index: self.turn_index,
            speaker_role: SpeakerRole::parse(&self.speaker_role).unwrap_or(SpeakerRole::User),
            content: self.content.clone(),
            content_preview: self.content_preview.clone(),
            embedding: embedding_info,
            token_count: self.token_count,
            metadata: self.metadata.clone(),
            created_at: self.created_at,
        }
    }
}

/// Row returned from vector-similarity search over conversation_turns.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScoredTurnRow {
    #[sqlx(flatten)]
    pub inner: TurnRow,
    pub similarity: f64,
    pub session_external_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_role_parse_valid() {
        assert_eq!(SpeakerRole::parse("user"), Some(SpeakerRole::User));
        assert_eq!(SpeakerRole::parse("assistant"), Some(SpeakerRole::Assistant));
        assert_eq!(SpeakerRole::parse("system"), Some(SpeakerRole::System));
        assert_eq!(SpeakerRole::parse("tool"), Some(SpeakerRole::Tool));
    }

    #[test]
    fn speaker_role_parse_invalid() {
        assert_eq!(SpeakerRole::parse("random"), None);
        assert_eq!(SpeakerRole::parse(""), None);
        assert_eq!(SpeakerRole::parse("USER"), None); // case sensitive
    }

    #[test]
    fn speaker_role_as_str() {
        assert_eq!(SpeakerRole::User.as_str(), "user");
        assert_eq!(SpeakerRole::Assistant.as_str(), "assistant");
        assert_eq!(SpeakerRole::System.as_str(), "system");
        assert_eq!(SpeakerRole::Tool.as_str(), "tool");
    }

    #[test]
    fn infer_role_from_prefix() {
        assert_eq!(
            SpeakerRole::infer_from_content("user: hello"),
            SpeakerRole::User
        );
        assert_eq!(
            SpeakerRole::infer_from_content("human: hi"),
            SpeakerRole::User
        );
        assert_eq!(
            SpeakerRole::infer_from_content("assistant: hi there"),
            SpeakerRole::Assistant
        );
        assert_eq!(
            SpeakerRole::infer_from_content("ai: response"),
            SpeakerRole::Assistant
        );
        assert_eq!(
            SpeakerRole::infer_from_content("system: boot"),
            SpeakerRole::System
        );
        assert_eq!(
            SpeakerRole::infer_from_content("tool: result"),
            SpeakerRole::Tool
        );
    }

    #[test]
    fn infer_role_heuristic() {
        // No prefix, uses content heuristics
        assert_eq!(
            SpeakerRole::infer_from_content("I can help you with that"),
            SpeakerRole::Assistant
        );
        assert_eq!(
            SpeakerRole::infer_from_content("Here is the file you requested"),
            SpeakerRole::Assistant
        );
        // Generic text defaults to User
        assert_eq!(
            SpeakerRole::infer_from_content("random text without prefix"),
            SpeakerRole::User
        );
    }

    #[test]
    fn speaker_role_serde_roundtrip() {
        let roles = vec![
            SpeakerRole::User,
            SpeakerRole::Assistant,
            SpeakerRole::System,
            SpeakerRole::Tool,
        ];
        for role in roles {
            let json = serde_json::to_string(&role).unwrap();
            let parsed: SpeakerRole = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn speaker_role_json_format() {
        let json = serde_json::to_string(&SpeakerRole::User).unwrap();
        assert_eq!(json, "\"user\"");
        let json = serde_json::to_string(&SpeakerRole::Assistant).unwrap();
        assert_eq!(json, "\"assistant\"");
    }

    #[test]
    fn conversation_session_defaults() {
        let session = ConversationSession {
            id: Uuid::new_v4(),
            external_id: Some("test-session".into()),
            title: Some("Test".into()),
            participant_count: 2,
            turn_count: 0,
            started_at: Utc::now(),
            ended_at: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(session.participant_count, 2);
        assert_eq!(session.turn_count, 0);
        assert!(session.external_id.is_some());
    }

    #[test]
    fn conversation_turn_fields() {
        let turn = ConversationTurn {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            turn_index: 3,
            speaker_role: SpeakerRole::User,
            content: "test content".into(),
            content_preview: Some("test content".into()),
            embedding: Some(EmbeddingInfo {
                vector: vec![0.1, 0.2, 0.3],
                provider: "local_ollama".into(),
                dimension: 3,
                metadata: None,
            }),
            token_count: Some(12),
            metadata: serde_json::json!({"model": "test"}),
            created_at: Utc::now(),
        };
        assert_eq!(turn.turn_index, 3);
        assert_eq!(turn.speaker_role, SpeakerRole::User);
        assert_eq!(turn.content, "test content");
        assert!(turn.embedding.is_some());
    }

    #[test]
    fn turn_row_to_turn_conversion() {
        let row = TurnRow {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            turn_index: 0,
            speaker_role: "user".to_string(),
            content: "hello".to_string(),
            content_preview: Some("hello".to_string()),
            embedding: None,
            embedding_type: None,
            embedding_dim: None,
            token_count: Some(1),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        };
        let turn = row.to_turn();
        assert_eq!(turn.turn_index, 0);
        assert_eq!(turn.speaker_role, SpeakerRole::User);
        assert_eq!(turn.content, "hello");
    }

    #[test]
    fn turn_row_with_embedding_info_to_turn() {
        let row = TurnRow {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            turn_index: 1,
            speaker_role: "assistant".to_string(),
            content: "response".to_string(),
            content_preview: Some("response".to_string()),
            embedding: Some(vec![0.1, 0.2]),
            embedding_type: Some("openai".into()),
            embedding_dim: Some(2),
            token_count: Some(5),
            metadata: serde_json::json!({"model": "text-embedding-3-small"}),
            created_at: Utc::now(),
        };
        let turn = row.to_turn();
        assert!(turn.embedding.is_some());
        let emb = turn.embedding.unwrap();
        assert_eq!(emb.vector, vec![0.1, 0.2]);
        assert_eq!(emb.provider, "openai");
        assert_eq!(emb.dimension, 2);
        assert!(emb.metadata.is_none());
    }

    #[test]
    fn embedding_info_serde_roundtrip() {
        let info = EmbeddingInfo {
            vector: vec![1.0, 2.0, 3.0],
            provider: "local_ollama".into(),
            dimension: 3,
            metadata: Some(serde_json::json!({"model": "snowflake-arctic-embed2"})),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: EmbeddingInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.vector, info.vector);
        assert_eq!(parsed.provider, info.provider);
        assert_eq!(parsed.dimension, info.dimension);
        assert!(parsed.metadata.is_some());
    }

    #[test]
    fn embedding_info_no_metadata_serde() {
        let info = EmbeddingInfo {
            vector: vec![0.0],
            provider: "grok".into(),
            dimension: 1,
            metadata: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        // metadata should be absent, not null
        assert!(!json.contains("metadata"));
        let parsed: EmbeddingInfo = serde_json::from_str(&json).unwrap();
        assert!(parsed.metadata.is_none());
    }
}

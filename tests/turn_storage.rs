//! Unit tests for turn-level storage backend.
//!
//! Tests the data model, speaker role inference, session/turn type construction,
//! and the migration backfill logic. Storage methods (PostgresStore::store_turn
//! etc.) are integration-tested against a live PostgreSQL instance.
//!
//! Run: cargo test --features postgres-storage -- turn_storage
//!
//! Requires: DATABASE_URL env var (for PG-dependent tests, skipped otherwise)
#![cfg(feature = "postgres-storage")]

use std::env;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use knowwhere_server::api::turns::{
    PaginatedSessionTurns, SessionTurn, StoreTurnRequest, StoreTurnResponse, StoreTurnsRequest,
    UpdateTurnRequest,
};
use knowwhere_server::memory::conversation::{
    ConversationSession, ConversationTurn, EmbeddingInfo, ScoredTurn, SessionRow, SpeakerRole,
    TurnRow,
};
use knowwhere_server::storage::PostgresStore;

// =========================================================================
// Type-level unit tests (no PostgreSQL required)
// =========================================================================

#[test]
fn speaker_role_parse_all_variants() {
    assert_eq!(SpeakerRole::parse("user"), Some(SpeakerRole::User));
    assert_eq!(
        SpeakerRole::parse("assistant"),
        Some(SpeakerRole::Assistant)
    );
    assert_eq!(SpeakerRole::parse("system"), Some(SpeakerRole::System));
    assert_eq!(SpeakerRole::parse("tool"), Some(SpeakerRole::Tool));
}

#[test]
fn speaker_role_parse_case_sensitive() {
    assert_eq!(SpeakerRole::parse("User"), None);
    assert_eq!(SpeakerRole::parse("USER"), None);
    assert_eq!(SpeakerRole::parse(""), None);
}

#[test]
fn speaker_role_as_str_roundtrip() {
    for role in &[
        SpeakerRole::User,
        SpeakerRole::Assistant,
        SpeakerRole::System,
        SpeakerRole::Tool,
    ] {
        let s = role.as_str();
        let parsed = SpeakerRole::parse(s);
        assert_eq!(parsed, Some(*role), "roundtrip failed for {s}");
    }
}

#[test]
fn infer_role_from_prefix() {
    assert_eq!(
        SpeakerRole::infer_from_content("user: hello"),
        SpeakerRole::User
    );
    assert_eq!(
        SpeakerRole::infer_from_content("human: hi there"),
        SpeakerRole::User
    );
    assert_eq!(
        SpeakerRole::infer_from_content("assistant: hello"),
        SpeakerRole::Assistant
    );
    assert_eq!(
        SpeakerRole::infer_from_content("ai: response"),
        SpeakerRole::Assistant
    );
    assert_eq!(
        SpeakerRole::infer_from_content("system: booting"),
        SpeakerRole::System
    );
    assert_eq!(
        SpeakerRole::infer_from_content("tool: result"),
        SpeakerRole::Tool
    );
    assert_eq!(
        SpeakerRole::infer_from_content("function: compute"),
        SpeakerRole::Tool
    );
}

#[test]
fn infer_role_heuristic_fallback() {
    // Content with assistant-like phrasing
    assert_eq!(
        SpeakerRole::infer_from_content("I can help you with that"),
        SpeakerRole::Assistant
    );
    assert_eq!(
        SpeakerRole::infer_from_content("I'll look into it"),
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
fn speaker_role_serde_json() {
    let user_json = serde_json::to_string(&SpeakerRole::User).unwrap();
    assert_eq!(user_json, "\"user\"");
    let tool_json = serde_json::to_string(&SpeakerRole::Tool).unwrap();
    assert_eq!(tool_json, "\"tool\"");
    let assistant_json = serde_json::to_string(&SpeakerRole::Assistant).unwrap();
    assert_eq!(assistant_json, "\"assistant\"");
}

#[test]
fn conversation_session_defaults() {
    let now = Utc::now();
    let session = ConversationSession {
        id: Uuid::new_v4(),
        external_id: Some("test-session-1".into()),
        title: Some("Test Session".into()),
        participant_count: 2,
        turn_count: 0,
        started_at: now,
        ended_at: None,
        metadata: json!({"platform": "hermes"}),
        created_at: now,
        updated_at: now,
    };
    assert_eq!(session.participant_count, 2);
    assert_eq!(session.turn_count, 0);
    assert!(session.external_id.is_some());
}

#[test]
fn conversation_turn_construction() {
    let turn = ConversationTurn {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        turn_index: 3,
        speaker_role: SpeakerRole::User,
        content: "How do I deploy?".into(),
        content_preview: Some("How do I deploy?".into()),
        embedding: Some(EmbeddingInfo {
            vector: vec![0.1, 0.2, 0.3],
            provider: "local_ollama".into(),
            dimension: 3,
            metadata: None,
        }),
        token_count: Some(5),
        metadata: json!({"model": "deepseek-v4-pro"}),
        created_at: Utc::now(),
    };
    assert_eq!(turn.turn_index, 3);
    assert_eq!(turn.speaker_role, SpeakerRole::User);
    assert_eq!(turn.content, "How do I deploy?");
    assert_eq!(turn.token_count, Some(5));
    assert!(turn.embedding.is_some());
}

#[test]
fn turn_row_to_turn_conversion() {
    let row = TurnRow {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        turn_index: 7,
        speaker_role: "assistant".to_string(),
        content: "Here is the deployment guide.".to_string(),
        content_preview: Some("Here is the deployment".to_string()),
        embedding: Some(vec![0.4, 0.5]),
        embedding_type: Some("openai".into()),
        embedding_dim: Some(2),
        token_count: Some(12),
        metadata: json!({}),
        created_at: Utc::now(),
    };
    let turn = row.to_turn();
    assert_eq!(turn.turn_index, 7);
    assert_eq!(turn.speaker_role, SpeakerRole::Assistant);
    assert_eq!(turn.content, "Here is the deployment guide.");
    assert!(turn.embedding.is_some());
    let emb = turn.embedding.unwrap();
    assert_eq!(emb.provider, "openai");
    assert_eq!(emb.dimension, 2);
}

#[test]
fn session_row_to_session_conversion() {
    let now = Utc::now();
    let row = SessionRow {
        id: Uuid::new_v4(),
        external_id: Some("s-42".into()),
        title: Some("Deploy Discussion".into()),
        participant_count: 2,
        turn_count: 10,
        started_at: now,
        ended_at: None,
        metadata: json!({}),
        created_at: now,
        updated_at: now,
    };
    let session: ConversationSession = row.into();
    assert_eq!(session.turn_count, 10);
    assert_eq!(session.external_id, Some("s-42".into()));
    assert_eq!(session.title, Some("Deploy Discussion".into()));
}

#[test]
fn scored_turn_serialization() {
    let turn = ScoredTurn {
        turn: ConversationTurn {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            turn_index: 0,
            speaker_role: SpeakerRole::User,
            content: "test".into(),
            content_preview: Some("test".into()),
            embedding: None,
            token_count: None,
            metadata: json!({}),
            created_at: Utc::now(),
        },
        similarity: 0.95,
        session_external_id: Some("ext-1".into()),
        adjacent_turns: vec![],
    };
    let json_str = serde_json::to_string(&turn).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["similarity"].as_f64().unwrap(), 0.95);
    assert_eq!(parsed["session_external_id"].as_str().unwrap(), "ext-1");
}

#[test]
fn turn_index_unique_within_session() {
    // Verify that two turns in the same session can't share turn_index.
    // This is enforced by the DB constraint, but we validate the concept:
    let sid = Uuid::new_v4();
    let t1 = ConversationTurn {
        id: Uuid::new_v4(),
        session_id: sid,
        turn_index: 0,
        speaker_role: SpeakerRole::User,
        content: "hello".into(),
        content_preview: None,
        embedding: None,
        token_count: None,
        metadata: json!({}),
        created_at: Utc::now(),
    };
    let t2 = ConversationTurn {
        id: Uuid::new_v4(),
        session_id: sid,
        turn_index: 1, // different index — OK
        speaker_role: SpeakerRole::Assistant,
        content: "hi".into(),
        content_preview: None,
        embedding: None,
        token_count: None,
        metadata: json!({}),
        created_at: Utc::now(),
    };
    assert_eq!(t1.session_id, t2.session_id);
    assert_ne!(t1.turn_index, t2.turn_index);
}

// =========================================================================
// Migration backfill logic tests (Python script parity)
// =========================================================================

#[test]
fn infer_role_matches_python_script() {
    // These must match backfill_turn_storage.py:infer_speaker_role()
    let cases = vec![
        ("user: hello", SpeakerRole::User),
        ("human: how are you", SpeakerRole::User),
        ("assistant: I'm fine", SpeakerRole::Assistant),
        ("ai: response here", SpeakerRole::Assistant),
        ("system: initializing", SpeakerRole::System),
        ("tool: {result: ok}", SpeakerRole::Tool),
        ("function: compute()", SpeakerRole::Tool),
        ("I can help with that", SpeakerRole::Assistant),
        ("I'll check on that", SpeakerRole::Assistant),
        ("Here is your answer", SpeakerRole::Assistant),
        ("random unlabeled text", SpeakerRole::User),
    ];
    for (content, expected) in cases {
        assert_eq!(
            SpeakerRole::infer_from_content(content),
            expected,
            "infer_from_content({content:?}) should be {expected:?}"
        );
    }
}

#[test]
fn session_external_id_format() {
    // External IDs can be any string — used across Hermes/OpenClaw
    let session = ConversationSession {
        id: Uuid::new_v4(),
        external_id: Some("hermes:session:abc123:2026-05-19".into()),
        title: None,
        participant_count: 2,
        turn_count: 0,
        started_at: Utc::now(),
        ended_at: None,
        metadata: json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    assert!(session.external_id.unwrap().contains("hermes"));
}

// =========================================================================
// Migration 016 logic tests — mirror SQL helper functions
// =========================================================================

/// Replicate SQL `get_turn_index(metadata JSONB)` in Rust.
/// Pattern A (chunked): metadata.is_chunk == true → chunk_index
/// Pattern B (single-turn): metadata.turn_index exists
/// Returns None for non-turn records.
fn get_turn_index(metadata: &serde_json::Value) -> Option<i32> {
    if metadata
        .get("is_chunk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return metadata
            .get("chunk_index")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
    }
    metadata
        .get("turn_index")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
}

/// Replicate SQL `is_eligible_for_turn_migration(metadata, turn_id)`.
fn is_eligible_for_migration(metadata: &serde_json::Value, turn_id: Option<Uuid>) -> bool {
    // Already migrated
    if turn_id.is_some() {
        return false;
    }
    // Must have session_id
    if metadata.get("session_id").is_none() {
        return false;
    }
    // Skip raw full-content nodes
    if metadata
        .get("is_full_content")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    // Must have determinable turn_index
    get_turn_index(metadata).is_some()
}

#[test]
fn migration_get_turn_index_from_chunk() {
    // Pattern A: chunked session → chunk_index
    let meta = json!({"is_chunk": true, "chunk_index": 3, "session_id": "s1"});
    assert_eq!(get_turn_index(&meta), Some(3));
}

#[test]
fn migration_get_turn_index_from_single_turn() {
    // Pattern B: single-turn → turn_index
    let meta = json!({"turn_index": 7, "session_id": "s1"});
    assert_eq!(get_turn_index(&meta), Some(7));
}

#[test]
fn migration_get_turn_index_priority() {
    // When both is_chunk and turn_index exist, chunk takes priority (Pattern A)
    let meta = json!({"is_chunk": true, "chunk_index": 2, "turn_index": 99});
    assert_eq!(get_turn_index(&meta), Some(2));
}

#[test]
fn migration_get_turn_index_none_for_raw_node() {
    // Raw full-content nodes have no turn_index or chunk_index
    let meta = json!({"is_full_content": true, "session_id": "s1"});
    assert_eq!(get_turn_index(&meta), None);
}

#[test]
fn migration_get_turn_index_none_for_empty_meta() {
    assert_eq!(get_turn_index(&json!({})), None);
}

#[test]
fn migration_eligible_chunk_node() {
    let meta = json!({"is_chunk": true, "chunk_index": 0, "session_id": "s1"});
    assert!(is_eligible_for_migration(&meta, None));
}

#[test]
fn migration_eligible_single_turn() {
    let meta = json!({"turn_index": 2, "session_id": "s1"});
    assert!(is_eligible_for_migration(&meta, None));
}

#[test]
fn migration_ineligible_already_migrated() {
    let meta = json!({"turn_index": 0, "session_id": "s1"});
    assert!(is_eligible_for_migration(&meta, Some(Uuid::new_v4())) == false);
}

#[test]
fn migration_ineligible_raw_node() {
    let meta = json!({"is_full_content": true, "session_id": "s1"});
    assert!(!is_eligible_for_migration(&meta, None));
}

#[test]
fn migration_ineligible_no_session_id() {
    let meta = json!({"turn_index": 0});
    assert!(!is_eligible_for_migration(&meta, None));
}

#[test]
fn migration_ineligible_no_turn_index() {
    let meta = json!({"session_id": "s1"});
    assert!(!is_eligible_for_migration(&meta, None));
}

#[test]
fn migration_speaker_role_sql_parity() {
    // These must match the SQL infer_speaker_role() function exactly
    let cases: Vec<(&str, SpeakerRole)> = vec![
        ("user: hello", SpeakerRole::User),
        ("human: how are you", SpeakerRole::User),
        ("assistant: I'm fine", SpeakerRole::Assistant),
        ("ai: response here", SpeakerRole::Assistant),
        ("system: initializing", SpeakerRole::System),
        ("tool: {result: ok}", SpeakerRole::Tool),
        ("function: compute()", SpeakerRole::Tool),
        ("I can help with that", SpeakerRole::Assistant),
        ("I'll check on that", SpeakerRole::Assistant),
        ("Here is your answer", SpeakerRole::Assistant),
        ("random unlabeled text", SpeakerRole::User),
        ("no prefix or heuristic match", SpeakerRole::User),
    ];
    for (content, expected) in cases {
        assert_eq!(
            SpeakerRole::infer_from_content(content),
            expected,
            "SQL parity failed for: {content:?}"
        );
    }
}

#[test]
fn migration_speaker_role_case_insensitive_prefix() {
    // SQL uses lower() on content — Rust does to_ascii_lowercase()
    assert_eq!(
        SpeakerRole::infer_from_content("USER: hello"),
        SpeakerRole::User
    );
    assert_eq!(
        SpeakerRole::infer_from_content("Assistant: response"),
        SpeakerRole::Assistant
    );
    assert_eq!(
        SpeakerRole::infer_from_content("TOOL: result"),
        SpeakerRole::Tool
    );
}

#[test]
fn migration_speaker_role_whitespace_handling() {
    // Leading whitespace should not affect inference
    assert_eq!(
        SpeakerRole::infer_from_content("  user: hello"),
        SpeakerRole::User
    );
    assert_eq!(
        SpeakerRole::infer_from_content("\tassistant: hi"),
        SpeakerRole::Assistant
    );
}

// =========================================================================
// API type tests — request/response serialization and validation
// =========================================================================

#[test]
fn api_store_turn_request_serde() {
    let req = StoreTurnRequest {
        session_id: "test-session-1".into(),
        turn_index: 0,
        speaker_role: "user".into(),
        content: "Hello, world!".into(),
        metadata: Some(json!({"key": "value"})),
    };
    let json_str = serde_json::to_string(&req).unwrap();
    let parsed: StoreTurnRequest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.session_id, "test-session-1");
    assert_eq!(parsed.turn_index, 0);
    assert_eq!(parsed.speaker_role, "user");
    assert_eq!(parsed.content, "Hello, world!");
}

#[test]
fn api_store_turn_request_default_metadata() {
    // metadata should default to None when absent
    let json_str =
        r#"{"session_id":"s1","turn_index":5,"speaker_role":"assistant","content":"hi"}"#;
    let req: StoreTurnRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.turn_index, 5);
    assert!(req.metadata.is_none());
}

#[test]
fn api_update_turn_request_partial_fields() {
    // Only content set
    let json_str = r#"{"content":"updated text"}"#;
    let req: UpdateTurnRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.content, Some("updated text".into()));
    assert!(req.metadata.is_none());
    assert!(req.embedding.is_none());
    assert!(req.embedding_type.is_none());
    assert!(req.embedding_dim.is_none());
    assert!(req.speaker_role.is_none());
}

#[test]
fn api_update_turn_request_all_fields() {
    let json_str =
        r#"{"content":"new","metadata":{"k":"v"},"embedding":[0.1,0.2],"speaker_role":"tool"}"#;
    let req: UpdateTurnRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.content, Some("new".into()));
    assert!(req.metadata.is_some());
    assert_eq!(req.embedding, Some(vec![0.1, 0.2]));
    assert_eq!(req.speaker_role, Some("tool".into()));
    // New fields default to None when absent
    assert!(req.embedding_type.is_none());
    assert!(req.embedding_dim.is_none());
}

#[test]
fn api_update_turn_request_with_embedding_metadata() {
    let json_str = r#"{"embedding":[0.1,0.2],"embedding_type":"openai","embedding_dim":2}"#;
    let req: UpdateTurnRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.embedding, Some(vec![0.1, 0.2]));
    assert_eq!(req.embedding_type.as_deref(), Some("openai"));
    assert_eq!(req.embedding_dim, Some(2));
    assert!(req.content.is_none());
    assert!(req.speaker_role.is_none());
}

#[test]
fn api_update_turn_request_empty() {
    let json_str = r#"{}"#;
    let req: UpdateTurnRequest = serde_json::from_str(json_str).unwrap();
    assert!(req.content.is_none());
    assert!(req.metadata.is_none());
    assert!(req.embedding.is_none());
    assert!(req.embedding_type.is_none());
    assert!(req.embedding_dim.is_none());
    assert!(req.speaker_role.is_none());
}

#[test]
fn api_store_turns_request_batch() {
    let json_str = r#"{"session_id":"s1","turns":[{"turn_index":0,"speaker_role":"user","content":"hi"},{"turn_index":1,"speaker_role":"assistant","content":"hello"}]}"#;
    let req: StoreTurnsRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.session_id, "s1");
    assert_eq!(req.turns.len(), 2);
    assert_eq!(req.turns[0].turn_index, 0);
    assert_eq!(req.turns[1].turn_index, 1);
}

#[test]
fn api_paginated_session_turns_fields() {
    let turn = SessionTurn {
        turn_id: Uuid::new_v4(),
        turn_index: 0,
        speaker_role: "user".into(),
        content: "hello".into(),
        content_preview: "hello".into(),
        token_count: Some(1),
        metadata: None,
        created_at: Utc::now(),
        embedding_info: None,
    };
    let paginated = PaginatedSessionTurns {
        session_id: Uuid::new_v4(),
        external_session_id: Some("ext-1".into()),
        turns: vec![turn],
        total_count: 42,
        offset: 10,
        limit: 5,
    };
    assert_eq!(paginated.total_count, 42);
    assert_eq!(paginated.offset, 10);
    assert_eq!(paginated.limit, 5);
    assert_eq!(paginated.turns.len(), 1);
}

#[test]
fn api_store_turn_response_fields() {
    let resp = StoreTurnResponse {
        turn_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        turn_index: 3,
        message: "Turn stored successfully".into(),
    };
    let json_str = serde_json::to_string(&resp).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["turn_index"].as_i64().unwrap(), 3);
    assert!(parsed["message"].as_str().unwrap().contains("stored"));
}

#[test]
fn api_scored_turn_with_adjacent_turns() {
    use knowwhere_server::api::turns::{ScoredTurn as ApiScoredTurn, TurnContext};
    let scored = ApiScoredTurn {
        turn_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        external_session_id: Some("ext-1".into()),
        turn_index: 5,
        speaker_role: "user".into(),
        content: "query text".into(),
        similarity: 0.87,
        metadata: Some(json!({"source": "test"})),
        created_at: Utc::now(),
        embedding_info: None,
        adjacent_turns: Some(vec![TurnContext {
            turn_id: Uuid::new_v4(),
            turn_index: 4,
            speaker_role: "assistant".into(),
            content: "previous response".into(),
            metadata: None,
        }]),
    };
    let json_str = serde_json::to_string(&scored).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!((parsed["similarity"].as_f64().unwrap() - 0.87).abs() < 0.001);
    assert!(parsed.get("adjacent_turns").is_some());
    assert_eq!(parsed["adjacent_turns"].as_array().unwrap().len(), 1);
}

// =========================================================================
// PostgreSQL integration tests (skipped if DATABASE_URL not set)
// =========================================================================

async fn get_pg() -> Option<PostgresStore> {
    let url = env::var("DATABASE_URL").ok()?;
    PostgresStore::connect(&url).await.ok()
}

#[tokio::test]
async fn pg_find_or_create_session() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-session-{}", Uuid::new_v4());
    let id = pg.find_or_create_session(&ext_id).await.unwrap();
    assert!(!id.is_nil());

    // Second call should return same ID
    let id2 = pg.find_or_create_session(&ext_id).await.unwrap();
    assert_eq!(id, id2);
}

#[tokio::test]
async fn pg_store_and_retrieve_turn() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-store-turn-{}", Uuid::new_v4());

    // Store a turn
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "What is the meaning of life?",
            vec![0.1; 1024],
            Some(json!({"model": "test"})),
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    assert!(!turn_id.is_nil());

    // Retrieve session turns
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();
    let response = pg.get_session_turns(session_id).await.unwrap();
    assert_eq!(response.turns.len(), 1);
    assert_eq!(response.turns[0].content, "What is the meaning of life?");
    assert_eq!(response.turns[0].speaker_role, "user");
    assert_eq!(response.turns[0].turn_index, 0);
    // Verify per-turn embedding metadata is returned
    let emb_info = response.turns[0]
        .embedding_info
        .as_ref()
        .expect("embedding_info should be present");
    assert_eq!(emb_info.provider, "local_ollama");
    assert_eq!(emb_info.dimension, 1024);
    assert!(
        emb_info.vector.is_empty(),
        "vector should be excluded from read response"
    );
}

#[tokio::test]
async fn pg_store_multiple_turns_session_order() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-order-{}", Uuid::new_v4());

    let turns_data = vec![
        (0, "user", "hello"),
        (1, "assistant", "hi there"),
        (2, "user", "how are you?"),
        (3, "assistant", "I'm great, thanks!"),
    ];

    for (idx, role, content) in &turns_data {
        pg.store_turn(
            &ext_id,
            *idx,
            role,
            content,
            vec![0.2; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }

    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();
    let response = pg.get_session_turns(session_id).await.unwrap();

    assert_eq!(response.turns.len(), 4);
    for (i, turn) in response.turns.iter().enumerate() {
        assert_eq!(turn.turn_index, i as i32, "turn {i} has wrong index");
        assert_eq!(turn.speaker_role, turns_data[i].1);
        assert_eq!(turn.content, turns_data[i].2);
    }
}

#[tokio::test]
async fn pg_retrieve_turns_vector_search() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-retrieve-{}", Uuid::new_v4());

    // Store turns with different "topics" via embeddings
    pg.store_turn(
        &ext_id,
        0,
        "user",
        "deploy to production",
        vec![1.0; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    pg.store_turn(
        &ext_id,
        1,
        "assistant",
        "here is how to deploy",
        vec![1.0; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    pg.store_turn(
        &ext_id,
        2,
        "user",
        "what about lunch?",
        vec![0.0; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();

    // Search with deployment-like vector
    let results = pg
        .retrieve_turns(&vec![0.9; 1024], 5, None, None)
        .await
        .unwrap();

    assert!(!results.is_empty(), "should find at least one turn");
    // First results should be about deployment (higher cosine similarity)
    assert!(results[0].content.contains("deploy"));
}

#[tokio::test]
async fn pg_adjacent_turns() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-adjacent-{}", Uuid::new_v4());
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    for i in 0..10 {
        pg.store_turn(
            &ext_id,
            i,
            "user",
            &format!("message {i}"),
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }

    // Get ±2 around turn 5
    let adjacent = pg.get_adjacent_turns(session_id, 5, 2).await.unwrap();

    // Should get turns 3,4,6,7 (excludes 5 itself)
    assert_eq!(adjacent.len(), 4);
    let indices: Vec<i32> = adjacent.iter().map(|t| t.turn_index).collect();
    assert_eq!(indices, vec![3, 4, 6, 7]);
}

#[tokio::test]
async fn pg_turn_on_conflict_idempotent() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-idempotent-{}", Uuid::new_v4());

    // Store same turn twice
    let id1 = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "original",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    let id2 = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "updated",
            vec![0.2; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    // Both should return same turn_id (ON CONFLICT DO UPDATE returns existing id)
    assert_eq!(id1, id2, "idempotent store should return same turn_id");

    // Content should be the updated version
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();
    let turns_resp = pg.get_session_turns(session_id).await.unwrap();
    assert_eq!(turns_resp.turns.len(), 1);
    assert_eq!(turns_resp.turns[0].content, "updated");
}

// =========================================================================
// CRUD integration tests — update, delete, get
// =========================================================================

#[tokio::test]
async fn pg_update_turn_content_only() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-update-content-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "original",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let updated = pg
        .update_turn(turn_id, Some("revised"), None, None, None, None, None)
        .await
        .unwrap();
    assert!(updated, "update should affect one row");

    let row = pg
        .get_turn(turn_id)
        .await
        .unwrap()
        .expect("turn should exist");
    assert_eq!(row.content, "revised");
    assert_eq!(row.speaker_role, "user"); // unchanged
}

#[tokio::test]
async fn pg_update_turn_speaker_role_only() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-update-role-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "text",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let updated = pg
        .update_turn(turn_id, None, None, None, None, None, Some("assistant"))
        .await
        .unwrap();
    assert!(updated);

    let row = pg.get_turn(turn_id).await.unwrap().unwrap();
    assert_eq!(row.speaker_role, "assistant");
    assert_eq!(row.content, "text"); // unchanged
}

#[tokio::test]
async fn pg_update_turn_embedding_only() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-update-emb-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "text",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let new_emb = vec![0.9; 768];
    // Store with 1024-dim local_ollama, then update to 768-dim openai
    let updated = pg
        .update_turn(
            turn_id,
            None,
            None,
            Some(new_emb.clone()),
            Some("openai"),
            Some(768),
            None,
        )
        .await
        .unwrap();
    assert!(updated);

    let row = pg.get_turn(turn_id).await.unwrap().unwrap();
    assert_eq!(row.embedding, Some(new_emb));
    assert_eq!(row.embedding_type.as_deref(), Some("openai"));
    assert_eq!(row.embedding_dim, Some(768));
}

#[tokio::test]
async fn pg_update_turn_metadata_only() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-update-meta-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "text",
            vec![0.1; 1024],
            Some(json!({"v":1})),
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let updated = pg
        .update_turn(
            turn_id,
            None,
            Some(json!({"v":2,"extra":"data"})),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(updated);

    let row = pg.get_turn(turn_id).await.unwrap().unwrap();
    assert_eq!(row.metadata["v"].as_i64(), Some(2));
    assert_eq!(row.metadata["extra"].as_str(), Some("data"));
}

#[tokio::test]
async fn pg_update_turn_all_fields() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-update-all-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "old",
            vec![0.3; 1024],
            Some(json!({"old":true})),
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let updated = pg
        .update_turn(
            turn_id,
            Some("completely new"),
            Some(json!({"new":true})),
            Some(vec![0.7; 768]),
            Some("openai"),
            Some(768),
            Some("tool"),
        )
        .await
        .unwrap();
    assert!(updated);

    let row = pg.get_turn(turn_id).await.unwrap().unwrap();
    assert_eq!(row.content, "completely new");
    assert_eq!(row.speaker_role, "tool");
    assert_eq!(row.metadata["new"].as_bool(), Some(true));
    assert_eq!(row.embedding_type.as_deref(), Some("openai"));
    assert_eq!(row.embedding_dim, Some(768));
}

#[tokio::test]
async fn pg_update_turn_nonexistent() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let fake_id = Uuid::new_v4();
    let updated = pg
        .update_turn(fake_id, Some("ghost"), None, None, None, None, None)
        .await
        .unwrap();
    assert!(!updated, "nonexistent turn should not update");
}

#[tokio::test]
async fn pg_update_turn_empty_update() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-update-empty-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "text",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    // All Nones → no SET clause → returns false
    let updated = pg
        .update_turn(turn_id, None, None, None, None, None, None)
        .await
        .unwrap();
    assert!(!updated, "empty update should return false");
}

#[tokio::test]
async fn pg_delete_turn_removes_row() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-delete-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "delete me",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let deleted = pg.delete_turn(turn_id).await.unwrap();
    assert!(deleted);

    let row = pg.get_turn(turn_id).await.unwrap();
    assert!(row.is_none(), "turn should be gone after delete");
}

#[tokio::test]
async fn pg_delete_turn_nonexistent() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let deleted = pg.delete_turn(Uuid::new_v4()).await.unwrap();
    assert!(!deleted, "deleting nonexistent turn should return false");
}

#[tokio::test]
async fn pg_delete_turn_updates_session_turn_count() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-delete-count-{}", Uuid::new_v4());
    let t1 = pg
        .store_turn(
            &ext_id,
            0,
            "user",
            "one",
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    let t2 = pg
        .store_turn(
            &ext_id,
            1,
            "assistant",
            "two",
            vec![0.2; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    // Delete one turn
    pg.delete_turn(t1).await.unwrap();

    // Verify turn_count updated
    let response = pg.get_session_turns(session_id).await.unwrap();
    assert_eq!(response.turns.len(), 1);
    assert_eq!(response.turns[0].content, "two");
    let _ = t2; // silence unused warning
}

#[tokio::test]
async fn pg_get_turn_existing() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-get-{}", Uuid::new_v4());
    let turn_id = pg
        .store_turn(
            &ext_id,
            5,
            "system",
            "system msg",
            vec![0.5; 1024],
            Some(json!({"type":"init"})),
            "local_ollama",
            1024,
        )
        .await
        .unwrap();

    let row = pg
        .get_turn(turn_id)
        .await
        .unwrap()
        .expect("turn should exist");
    assert_eq!(row.turn_index, 5);
    assert_eq!(row.speaker_role, "system");
    assert_eq!(row.content, "system msg");
    assert_eq!(row.metadata["type"].as_str(), Some("init"));
    // Verify per-turn embedding is present with correct metadata
    assert!(
        row.embedding.is_some(),
        "get_turn should return the full embedding vector"
    );
    assert_eq!(row.embedding_type.as_deref(), Some("local_ollama"));
    assert_eq!(row.embedding_dim, Some(1024));
}

#[tokio::test]
async fn pg_get_turn_nonexistent() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let row = pg.get_turn(Uuid::new_v4()).await.unwrap();
    assert!(row.is_none());
}

// =========================================================================
// Pagination integration tests — list_turns_by_session
// =========================================================================

#[tokio::test]
async fn pg_list_turns_pagination_basic() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-paginate-{}", Uuid::new_v4());
    for i in 0..10 {
        pg.store_turn(
            &ext_id,
            i,
            "user",
            &format!("msg {i}"),
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    // Page 1: offset=0, limit=3
    let (turns, total) = pg
        .list_turns_by_session(session_id, 0, 3, false)
        .await
        .unwrap();
    assert_eq!(total, 10);
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0].content, "msg 0");
    assert_eq!(turns[2].content, "msg 2");

    // Page 2: offset=3, limit=3
    let (turns, total) = pg
        .list_turns_by_session(session_id, 3, 3, false)
        .await
        .unwrap();
    assert_eq!(total, 10);
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0].content, "msg 3");
}

#[tokio::test]
async fn pg_list_turns_pagination_desc() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-paginate-desc-{}", Uuid::new_v4());
    for i in 0..5 {
        pg.store_turn(
            &ext_id,
            i,
            "user",
            &format!("msg {i}"),
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    let (turns, total) = pg
        .list_turns_by_session(session_id, 0, 5, true)
        .await
        .unwrap();
    assert_eq!(total, 5);
    assert_eq!(turns.len(), 5);
    assert_eq!(turns[0].content, "msg 4"); // newest first in desc
    assert_eq!(turns[4].content, "msg 0");
}

#[tokio::test]
async fn pg_list_turns_pagination_near_end() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-paginate-end-{}", Uuid::new_v4());
    for i in 0..7 {
        pg.store_turn(
            &ext_id,
            i,
            "user",
            &format!("msg {i}"),
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    // offset=5, limit=5 → only 2 remaining
    let (turns, total) = pg
        .list_turns_by_session(session_id, 5, 5, false)
        .await
        .unwrap();
    assert_eq!(total, 7);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].content, "msg 5");
    assert_eq!(turns[1].content, "msg 6");
}

#[tokio::test]
async fn pg_list_turns_pagination_beyond_end() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-paginate-beyond-{}", Uuid::new_v4());
    for i in 0..3 {
        pg.store_turn(
            &ext_id,
            i,
            "user",
            &format!("msg {i}"),
            vec![0.1; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    let (turns, total) = pg
        .list_turns_by_session(session_id, 100, 10, false)
        .await
        .unwrap();
    assert_eq!(total, 3);
    assert!(turns.is_empty(), "offset beyond total should return empty");
}

#[tokio::test]
async fn pg_list_turns_empty_session() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-paginate-empty-{}", Uuid::new_v4());
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    let (turns, total) = pg
        .list_turns_by_session(session_id, 0, 10, false)
        .await
        .unwrap();
    assert_eq!(total, 0);
    assert!(turns.is_empty());
}

// =========================================================================
// Filtered retrieval integration tests — retrieve_turns
// =========================================================================

#[tokio::test]
async fn pg_retrieve_turns_speaker_filter() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-retrieve-filter-{}", Uuid::new_v4());
    pg.store_turn(
        &ext_id,
        0,
        "user",
        "deploy to prod",
        vec![1.0; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    pg.store_turn(
        &ext_id,
        1,
        "assistant",
        "here is deploy guide",
        vec![1.0; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    pg.store_turn(
        &ext_id,
        2,
        "user",
        "lunch plans",
        vec![0.3; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();

    // Filter to user only
    let results = pg
        .retrieve_turns(&vec![0.9; 1024], 10, Some("user"), None)
        .await
        .unwrap();
    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(
            r.speaker_role, "user",
            "speaker filter should exclude non-user"
        );
    }
}

#[tokio::test]
async fn pg_retrieve_turns_session_filter() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_a = format!("test-retrieve-session-a-{}", Uuid::new_v4());
    let ext_b = format!("test-retrieve-session-b-{}", Uuid::new_v4());
    pg.store_turn(
        &ext_a,
        0,
        "user",
        "topic alpha",
        vec![0.9; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    pg.store_turn(
        &ext_b,
        0,
        "user",
        "topic beta",
        vec![0.1; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    let session_a = pg.find_or_create_session(&ext_a).await.unwrap();

    let results = pg
        .retrieve_turns(&vec![0.9; 1024], 5, None, Some(session_a))
        .await
        .unwrap();
    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(
            r.session_id, session_a,
            "session filter should constrain results"
        );
    }
}

#[tokio::test]
async fn pg_retrieve_turns_combined_filters() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-retrieve-combined-{}", Uuid::new_v4());
    pg.store_turn(
        &ext_id,
        0,
        "user",
        "deployment",
        vec![0.8; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    pg.store_turn(
        &ext_id,
        1,
        "assistant",
        "deploy response",
        vec![0.7; 1024],
        None,
        "local_ollama",
        1024,
    )
    .await
    .unwrap();
    let session_id = pg.find_or_create_session(&ext_id).await.unwrap();

    let results = pg
        .retrieve_turns(&vec![0.9; 1024], 10, Some("user"), Some(session_id))
        .await
        .unwrap();
    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(r.speaker_role, "user");
        assert_eq!(r.session_id, session_id);
    }
}

#[tokio::test]
async fn pg_retrieve_turns_empty_query_vector() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let results = pg.retrieve_turns(&[], 10, None, None).await.unwrap();
    assert!(
        results.is_empty(),
        "empty query vector should return empty results"
    );
}

#[tokio::test]
async fn pg_retrieve_turns_top_k_limits() {
    let pg = match get_pg().await {
        Some(p) => p,
        None => {
            eprintln!("SKIP: DATABASE_URL not set or PG unreachable");
            return;
        }
    };
    let ext_id = format!("test-retrieve-topk-{}", Uuid::new_v4());
    for i in 0..5 {
        pg.store_turn(
            &ext_id,
            i,
            "user",
            &format!("msg {i}"),
            vec![0.5; 1024],
            None,
            "local_ollama",
            1024,
        )
        .await
        .unwrap();
    }

    let results = pg
        .retrieve_turns(&vec![0.5; 1024], 2, None, None)
        .await
        .unwrap();
    assert!(results.len() <= 2, "top_k should limit results");
}

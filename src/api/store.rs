use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::turns::BatchTurnItem;
use crate::api::types::{clean_for_embedding, parse_speaker_role_from_chunk, AppState};
use crate::embedding::{embed_document, embed_document_batch};
use crate::memory::fact_extraction::{FactExtractionContext, FactExtractor};
use crate::memory::types::{MemorySource, MemoryType, Sensitivity};
use crate::memory::FractalNode;
use crate::multimodal::MultimodalData;

pub(crate) fn chunk_into_rounds(text: &str, min_round_chars: usize) -> Vec<String> {
    let role_prefixes = [
        "user:",
        "assistant:",
        "human:",
        "ai:",
        "User:",
        "Assistant:",
        "Human:",
        "AI:",
    ];
    let lines: Vec<&str> = text.lines().collect();
    let mut rounds: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut has_role_prefixes = false;

    for line in &lines {
        let trimmed = line.trim();
        let is_role_start = role_prefixes.iter().any(|p| trimmed.starts_with(p));

        if is_role_start {
            has_role_prefixes = true;
            if !current.is_empty() {
                let c = current.trim().to_string();
                if !c.is_empty() {
                    rounds.push(c);
                }
                current.clear();
            }
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        rounds.push(current.trim().to_string());
    }

    // If no dialog turns detected, use smart semantic chunking
    if !has_role_prefixes && text.len() > 6000 {
        let chunker =
            crate::memory::TextChunker::new(crate::memory::ChunkerConfig::for_nomic_8192());
        let chunks = chunker.chunk(text);
        if chunks.len() > 1 {
            return chunks.into_iter().map(|c| c.content).collect();
        }
    }

    if rounds.len() <= 1 {
        return vec![text.to_string()];
    }

    // Merge tiny rounds into their predecessor to avoid near-empty chunks
    let mut merged: Vec<String> = Vec::new();
    for r in rounds {
        if let Some(last) = merged.last_mut() {
            if last.len() < min_round_chars {
                last.push('\n');
                last.push_str(&r);
                continue;
            }
        }
        merged.push(r);
    }
    if merged.len() <= 1 {
        return vec![text.to_string()];
    }
    merged
}

#[derive(Deserialize, ToSchema)]
pub struct StoreSessionRequest {
    pub content: String,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub metadata: HashMap<String, Value>,
    /// Memory type for this session node (default: episodic).
    #[serde(default = "default_memory_type_str")]
    pub memory_type: String,
    /// Source origin (default: conversation).
    #[serde(default = "default_source_str")]
    pub source: String,
    /// Optional importance 1–10 (default: type-specific).
    #[serde(default)]
    pub importance: Option<i32>,
    /// Optional sensitivity (default: normal).
    #[serde(default)]
    pub sensitivity: Option<Sensitivity>,
    /// Links turns together across a multi-turn session. Crash-safe: each turn
    /// is stored independently so a session crash only loses the current turn.
    #[serde(default)]
    pub session_id: Option<String>,
    /// 0-based turn index within the session. Allows reconstruction of turn order
    /// and detection of missing turns after a crash.
    #[serde(default)]
    pub turn_index: Option<usize>,
}

fn default_memory_type_str() -> String {
    "episodic".to_string()
}

fn default_source_str() -> String {
    "conversation".to_string()
}

fn metadata_text<'a>(metadata: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    metadata.get(key).and_then(Value::as_str)
}

fn metadata_matches(metadata: &HashMap<String, Value>, key: &str, values: &[&str]) -> bool {
    metadata_text(metadata, key).is_some_and(|value| {
        values
            .iter()
            .any(|candidate| value.eq_ignore_ascii_case(candidate))
    })
}

pub(crate) fn set_metadata_text(metadata: &mut HashMap<String, Value>, key: &str, value: &str) {
    metadata.insert(key.to_string(), Value::String(value.to_string()));
}

fn default_derivation(metadata: &HashMap<String, Value>) -> Option<&'static str> {
    if metadata_matches(
        metadata,
        "source",
        &["openclaw:agent_end", "openclaw:before_compaction"],
    ) {
        return Some("agent_transcript");
    }
    if metadata_matches(metadata, "role", &["assistant", "ai", "system", "mixed"]) {
        return Some("assistant_output");
    }
    metadata_matches(metadata, "role", &["user"]).then_some("user_input")
}

fn should_hide_from_user_retrieval(
    memory_type: MemoryType,
    metadata: &HashMap<String, Value>,
) -> bool {
    memory_type == MemoryType::Meta
        || metadata_matches(
            metadata,
            FractalNode::ROLE_KEY,
            &["assistant", "ai", "system", "mixed"],
        )
        || metadata_matches(
            metadata,
            "source",
            &["openclaw:agent_end", "openclaw:before_compaction"],
        )
        || metadata_matches(
            metadata,
            FractalNode::DERIVATION_KEY,
            &[
                "assistant_output",
                "retrieval_compose",
                "chat_query",
                "agent_transcript",
            ],
        )
}

fn default_trust_tier(
    memory_type: MemoryType,
    source: MemorySource,
    metadata: &HashMap<String, Value>,
) -> &'static str {
    if should_hide_from_user_retrieval(memory_type, metadata)
        || source == MemorySource::Consolidation
    {
        return FractalNode::TRUST_DERIVED;
    }
    let primary_import = metadata_text(metadata, "import_type").is_some_and(|import_type| {
        matches!(
            import_type,
            "openclaw_workspace" | "openclaw_session" | "langchain_memory" | "custom_import"
        )
    }) || metadata_text(metadata, "original_file")
        .is_some_and(|file| matches!(file, "MEMORY.md" | "USER.md" | "IDENTITY.md" | "SOUL.md"));
    if source == MemorySource::Import
        || metadata.contains_key("imported_from")
        || metadata.contains_key("import_type")
        || metadata_text(metadata, "source").is_some_and(|value| value.starts_with("import:"))
    {
        return if primary_import {
            FractalNode::TRUST_PRIMARY
        } else {
            FractalNode::TRUST_REFERENCE
        };
    }
    match source {
        MemorySource::Conversation => FractalNode::TRUST_PRIMARY,
        MemorySource::Document | MemorySource::Manual => FractalNode::TRUST_REFERENCE,
        MemorySource::Consolidation | MemorySource::AiSelfImprovement => FractalNode::TRUST_DERIVED,
        MemorySource::Import => FractalNode::TRUST_REFERENCE,
    }
}

fn default_claim_scope(memory_type: MemoryType, source: MemorySource) -> &'static str {
    match memory_type {
        MemoryType::Episodic => "episodic",
        MemoryType::Preference => "preference",
        MemoryType::Procedural => "procedural",
        MemoryType::Meta => "diagnostic",
        MemoryType::Decision => "decision",
        MemoryType::Semantic if source == MemorySource::Consolidation => "historical",
        MemoryType::Semantic => "current",
    }
}

fn normalize_node_metadata(
    memory_type: MemoryType,
    source: MemorySource,
    metadata: &mut HashMap<String, Value>,
) {
    if let Some(derivation) = default_derivation(metadata) {
        metadata
            .entry(FractalNode::DERIVATION_KEY.to_string())
            .or_insert_with(|| Value::String(derivation.to_string()));
    }
    let trust_tier = default_trust_tier(memory_type, source, metadata);
    metadata
        .entry("claim_scope".to_string())
        .or_insert_with(|| Value::String(default_claim_scope(memory_type, source).to_string()));
    if should_hide_from_user_retrieval(memory_type, metadata) {
        metadata
            .entry(FractalNode::TRUST_TIER_KEY.to_string())
            .or_insert_with(|| Value::String(trust_tier.to_string()));
        metadata
            .entry(FractalNode::RETRIEVAL_VISIBILITY_KEY.to_string())
            .or_insert_with(|| Value::String(FractalNode::INTERNAL_VISIBILITY.to_string()));
        return;
    }
    metadata
        .entry(FractalNode::TRUST_TIER_KEY.to_string())
        .or_insert_with(|| Value::String(trust_tier.to_string()));
}

#[derive(Serialize, ToSchema)]
pub struct StoreNodeResponse {
    pub id: Uuid,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_ids: Option<Vec<Uuid>>,
}

#[utoipa::path(
    post,
    path = "/store_session",
    tag = "memory",
    request_body(content = StoreSessionRequest, description = "JSON body for text; binary body with image/* or audio/* Content-Type for cross-modal embedding via EmbeddingRouter"),
    responses(
        (status = 201, description = "Session node created", body = StoreNodeResponse),
        (status = 400, description = "Bad request", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn store_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<StoreNodeResponse>), (StatusCode, String)> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    // Binary payloads: route through EmbeddingRouter for cross-modal embedding
    if content_type.starts_with("image/") || content_type.starts_with("audio/") {
        return store_session_binary(&state, content_type, &body).await;
    }

    // JSON payloads (existing flow): parse and embed as text
    let req: StoreSessionRequest = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")))?;
    store_session_json(state, req).await
}

/// Existing JSON-based store_session logic, extracted so binary and JSON paths
/// can share the same route while preserving backward compatibility.
async fn store_session_json(
    state: AppState,
    req: StoreSessionRequest,
) -> Result<(StatusCode, Json<StoreNodeResponse>), (StatusCode, String)> {
    let cleaned = clean_for_embedding(&req.content);
    if cleaned.len() < 4 {
        return Err((
            StatusCode::BAD_REQUEST,
            "content too short or empty after cleaning".into(),
        ));
    }
    // Reject highly repetitive content — Ollama rejects near-uniform strings
    {
        use std::collections::HashMap;
        let mut freq: HashMap<char, usize> = HashMap::new();
        let mut total = 0usize;
        for c in cleaned.chars() {
            if !c.is_whitespace() {
                *freq.entry(c).or_insert(0) += 1;
                total += 1;
            }
        }
        if total > 0 {
            if let Some(&max_count) = freq.values().max() {
                let ratio = max_count as f64 / total as f64;
                if ratio > 0.9 {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "content too repetitive for embedding".into(),
                    ));
                }
            }
        }
    }

    let memory_type = MemoryType::parse(&req.memory_type).unwrap_or(MemoryType::Episodic);
    let source = MemorySource::parse(&req.source).unwrap_or(MemorySource::Conversation);

    let min_round_chars: usize = std::env::var("KNOWWHERE_MIN_ROUND_CHARS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n >= 20)
        .unwrap_or(80);

    let chunks = chunk_into_rounds(&req.content, min_round_chars);

    if chunks.len() <= 1 {
        let vector = match req.vector {
            Some(v) if !v.is_empty() => v,
            _ => embed_document(&*state.embedding, &cleaned)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("auto-embed failed: {e}"),
                    )
                })?,
        };

        // ── Determine speaker role ──
        let speaker = parse_speaker_role_from_chunk(&req.content)
            .map(|(role, _)| role.to_string())
            .unwrap_or_else(|| "assistant".to_string());

        // ── Turn-level storage (postgres-storage) ──
        // When session_id is provided, store this as a single turn in
        // conversation_turns so the turn-level index is populated.
        // Must run BEFORE FractalNode creation to avoid moving req.content/metadata.
        #[cfg(feature = "postgres-storage")]
        if let (Some(pg), Some(ref sid)) = (state.pg_store.as_ref(), req.session_id.as_ref()) {
            let turn_idx = req.turn_index.map(|t| t as i32).unwrap_or(0);
            let turn_meta = Some(serde_json::to_value(&req.metadata).unwrap_or_default());
            let emb_type = state.embedding.name().to_string();
            let emb_dim = state.embedding.dimension() as i32;
            match pg
                .store_turn(
                    sid,
                    turn_idx,
                    &speaker,
                    &cleaned,
                    vector.clone(),
                    turn_meta,
                    &emb_type,
                    emb_dim,
                )
                .await
            {
                Ok(turn_id) => {
                    tracing::info!(%turn_id, %sid, turn_idx, %speaker, "turn stored (single-chunk session)")
                }
                Err(e) => tracing::warn!(%sid, "turn storage failed (non-fatal): {e}"),
            }
        }

        let mut metadata = req.metadata;
        metadata.insert(
            "speaker_role".to_string(),
            Value::String(speaker.to_string()),
        );
        metadata.insert("is_turn".to_string(), Value::Bool(true));
        if let Some(ref sid) = req.session_id {
            metadata.insert("session_id".to_string(), Value::String(sid.clone()));
        } else if let Some(Value::String(sid)) = metadata.get("session_id").cloned() {
            // Fallback: session_id passed inside metadata object (benchmark scripts)
            metadata.insert("session_id".to_string(), Value::String(sid));
        }
        if let Some(ti) = req.turn_index {
            metadata.insert("turn_index".to_string(), Value::Number(ti.into()));
        }
        normalize_node_metadata(memory_type, source, &mut metadata);
        let content = req.content.clone();
        let vector_for_node = vector.clone();
        let mut node = FractalNode::new_typed(
            Some(req.content),
            None,
            vector_for_node,
            metadata,
            memory_type,
            source,
        );
        if let Some(imp) = req.importance {
            node.importance = imp.clamp(1, 10);
        }
        if let Some(sens) = req.sensitivity {
            node.sensitivity = sens;
        }

        // ── Surprise-weighted salience boost (embedding-space novelty) ──
        // If the new embedding is dissimilar to all existing memories (max cosine
        // similarity < 0.4), boost importance and initial energy to make novel
        // information more salient in retrieval and delay its decay.
        let mut _surprise_boosted = false;
        #[cfg(feature = "postgres-storage")]
        if let Some(ref pg) = state.pg_store {
            match pg.compute_max_novelty_similarity(&vector, 5).await {
                Ok(max_sim) => {
                    if let Some((imp_boost, _initial_energy)) = compute_surprise_boost(max_sim) {
                        node.importance = (node.importance + imp_boost).clamp(1, 10);
                        surprise_boosted = true;
                        tracing::info!(
                            max_sim, importance = node.importance,
                            "surprise boost applied to single-turn node (novel memory)"
                        );
                    }
                }
                Err(e) => tracing::warn!("novelty check failed (non-fatal): {e}"),
            }
        }

        let id = state
            .store
            .insert(node)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Post-insert: set elevated initial energy for surprise-boosted memories
        #[cfg(feature = "postgres-storage")]
        if surprise_boosted {
            if let Some(ref pg) = state.pg_store {
                let _ = pg.set_memory_energy(id, SURPRISE_INITIAL_ENERGY).await;
                // Boost sibling memories in the same session (+1 importance)
                if let Some(ref sid_str) = req.session_id {
                    if let Ok(sid_uuid) = Uuid::parse_str(sid_str) {
                        let _ = pg.boost_sibling_importance(sid_uuid, 1, id).await;
                    }
                }
            }
        }

        tracing::info!(%id, %speaker, ?memory_type, "turn node stored (single-turn session)");
        // ── Inline fact extraction (regex-based, no LLM) ──
        // Extract obvious facts immediately so they're available
        // before async consolidation runs. Creates Decision-type nodes
        // with high weight (2.0) for retrieval boosting.
        if content.len() >= 20 {
            let dim = state.embedding.dimension();
            let facts = FactExtractor::extract_facts(&content);
            // ── Track schema frequencies (postgres-storage) ──
            #[cfg(feature = "postgres-storage")]
            if let Some(ref pool) = state.trajectory_pool {
                crate::memory::fact_extraction::track_fact_schema_frequencies(pool, &facts).await;
            }
            let ctx = FactExtractionContext {
                session_id: req.session_id.as_deref(),
                source_node_id: id,
                embedding_dim: dim,
            };
            let zero_vector = vec![0.0f32; dim];
            let fact_nodes: Vec<FractalNode> = facts
                .into_iter()
                .map(|f| f.to_fractal_node(ctx.source_node_id, ctx.session_id, zero_vector.clone()))
                .collect();
            let fact_count = fact_nodes.len();
            if fact_count > 0 {
                // Embed fact texts and store as Decision nodes
                for mut fact_node in fact_nodes {
                    let fact_content = fact_node.content.clone().unwrap_or_default();
                    match embed_document(&*state.embedding, &fact_content).await {
                        Ok(emb) => {
                            fact_node.vector = emb;
                            match state.store.insert(fact_node).await {
                                Ok(fact_id) => tracing::debug!(
                                    %fact_id, source_id = %id,
                                    "inline fact stored"
                                ),
                                Err(e) => tracing::debug!("inline fact store failed: {}", e),
                            }
                        }
                        Err(e) => tracing::debug!("inline fact embed failed: {}", e),
                    }
                }
                tracing::debug!(
                    %id, fact_count,
                    "inline facts extracted from turn content"
                );
            }
        }
        return Ok((
            StatusCode::CREATED,
            Json(StoreNodeResponse {
                id,
                message: "turn node created".to_string(),
                chunk_ids: None,
            }),
        ));
    }

    let turn_count = chunks.len();

    let cleaned: Vec<(usize, String)> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| (i, clean_for_embedding(c)))
        .filter(|(_, c)| c.len() >= 4)
        .collect();

    if cleaned.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "no embeddable turns".to_string(),
        ));
    }

    let refs: Vec<&str> = cleaned.iter().map(|(_, s)| s.as_str()).collect();
    let vectors = embed_document_batch(&*state.embedding, &refs)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("batch embed failed: {e}"),
            )
        })?;

    let mut all_ids: Vec<Uuid> = Vec::with_capacity(cleaned.len());
    for ((idx, _), vector) in cleaned.iter().zip(vectors) {
        let idx = *idx;
        let original_chunk = &chunks[idx];
        let (speaker, _) =
            parse_speaker_role_from_chunk(original_chunk).unwrap_or(("assistant", original_chunk));

        let mut metadata = req.metadata.clone();
        metadata.insert(
            "speaker_role".to_string(),
            Value::String(speaker.to_string()),
        );
        metadata.insert("is_turn".to_string(), Value::Bool(true));
        metadata.insert(
            "turn_index".to_string(),
            Value::Number(serde_json::Number::from(idx)),
        );
        metadata.insert(
            "turn_count".to_string(),
            Value::Number(serde_json::Number::from(turn_count)),
        );
        if let Some(ref sid) = req.session_id {
            metadata.insert("session_id".to_string(), Value::String(sid.clone()));
        } else if let Some(Value::String(sid)) = metadata.get("session_id").cloned() {
            metadata.insert("session_id".to_string(), Value::String(sid));
        }
        normalize_node_metadata(memory_type, source, &mut metadata);

        let content = original_chunk.clone();

        let mut node =
            FractalNode::new_typed(Some(content), None, vector, metadata, memory_type, source);
        if let Some(imp) = req.importance {
            node.importance = imp.clamp(1, 10);
        }
        if let Some(sens) = req.sensitivity {
            node.sensitivity = sens;
        }

        // ── Surprise-weighted salience boost (multi-turn path) ──
        let mut _turn_surprise = false;
        #[cfg(feature = "postgres-storage")]
        if let Some(ref pg) = state.pg_store {
            match pg.compute_max_novelty_similarity(&vector, 5).await {
                Ok(max_sim) => {
                    if let Some((imp_boost, _initial_energy)) = compute_surprise_boost(max_sim) {
                        node.importance = (node.importance + imp_boost).clamp(1, 10);
                        _turn_surprise = true;
                        tracing::info!(
                            turn = idx, max_sim, importance = node.importance,
                            "surprise boost applied to multi-turn node"
                        );
                    }
                }
                Err(e) => tracing::warn!("novelty check failed (non-fatal): {e}"),
            }
        }

        let id = state
            .store
            .insert(node)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Post-insert energy + sibling boost for surprising turns
        #[cfg(feature = "postgres-storage")]
        if _turn_surprise {
            if let Some(ref pg) = state.pg_store {
                let _ = pg.set_memory_energy(id, SURPRISE_INITIAL_ENERGY).await;
                // Boost siblings once per session if any turn is surprising
                if let Some(ref sid_str) = req.session_id {
                    if let Ok(sid_uuid) = Uuid::parse_str(sid_str) {
                        let _ = pg.boost_sibling_importance(sid_uuid, 1, id).await;
                    }
                }
            }
        }

        all_ids.push(id);
    }

    let primary_id = all_ids[0]; // Return first turn as primary

    tracing::info!(%primary_id, ?memory_type, turns = turn_count, "turn nodes stored ({} turns)", turn_count);
    // ── Inline fact extraction for multi-turn path ──
    if req.content.len() >= 20 {
        let dim = state.embedding.dimension();
        let facts = FactExtractor::extract_facts(&req.content);
        // ── Track schema frequencies (postgres-storage) ──
        #[cfg(feature = "postgres-storage")]
        if let Some(ref pool) = state.trajectory_pool {
            crate::memory::fact_extraction::track_fact_schema_frequencies(pool, &facts).await;
        }
        let ctx = FactExtractionContext {
            session_id: req.session_id.as_deref(),
            source_node_id: primary_id,
            embedding_dim: dim,
        };
        let zero_vector = vec![0.0f32; dim];
        let fact_nodes: Vec<FractalNode> = facts
            .into_iter()
            .map(|f| f.to_fractal_node(ctx.source_node_id, ctx.session_id, zero_vector.clone()))
            .collect();
        let fact_count = fact_nodes.len();
        if fact_count > 0 {
            for mut fact_node in fact_nodes {
                let fact_content = fact_node.content.clone().unwrap_or_default();
                match embed_document(&*state.embedding, &fact_content).await {
                    Ok(emb) => {
                        fact_node.vector = emb;
                        match state.store.insert(fact_node).await {
                            Ok(fact_id) => {
                                tracing::debug!(%fact_id, source_id = %primary_id, "inline fact stored (multi-turn)")
                            }
                            Err(e) => tracing::debug!("inline fact store failed: {}", e),
                        }
                    }
                    Err(e) => tracing::debug!("inline fact embed failed: {}", e),
                }
            }
            tracing::debug!(%primary_id, fact_count, "inline facts extracted from session content (multi-turn)");
        }
    }
    // ── Turn-level storage (postgres-storage, multi-turn) ──
    // Store each chunk as an individual turn in conversation_turns so
    // the turn-level HNSW index is populated for fine-grained retrieval.
    #[cfg(feature = "postgres-storage")]
    if let (Some(pg), Some(ref sid)) = (state.pg_store.as_ref(), req.session_id.as_ref()) {
        let mut turn_items: Vec<BatchTurnItem> = Vec::with_capacity(chunks.len());
        let mut turn_texts: Vec<&str> = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            let (speaker, _) = parse_speaker_role_from_chunk(chunk).unwrap_or(("assistant", ""));
            turn_items.push(BatchTurnItem {
                turn_index: i as i32,
                speaker_role: speaker.to_string(),
                content: chunk.clone(),
                metadata: Some(serde_json::to_value(&req.metadata).unwrap_or_default()),
            });
            turn_texts.push(chunk.as_str());
        }
        // Use cleaned texts for embedding (same as fractal node embeddings)
        let cleaned_texts: Vec<String> =
            turn_texts.iter().map(|t| clean_for_embedding(t)).collect();
        let cleaned_refs: Vec<&str> = cleaned_texts
            .iter()
            .map(|s| s.as_str())
            .filter(|s| s.len() >= 4)
            .collect();
        if !cleaned_refs.is_empty() {
            match embed_document_batch(&*state.embedding, &cleaned_refs).await {
                Ok(turn_embeddings) => {
                    let embeddable_items: Vec<BatchTurnItem> = turn_items
                        .iter()
                        .filter(|item| {
                            let cleaned = clean_for_embedding(&item.content);
                            cleaned.len() >= 4
                        })
                        .cloned()
                        .collect();
                    if embeddable_items.len() == turn_embeddings.len() {
                        let emb_type = state.embedding.name().to_string();
                        let emb_dim = state.embedding.dimension() as i32;
                        match pg
                            .store_turns_batch(
                                sid,
                                &embeddable_items,
                                turn_embeddings,
                                &emb_type,
                                emb_dim,
                            )
                            .await
                        {
                            Ok((session_uuid, turn_ids)) => {
                                tracing::info!(%session_uuid, turns = turn_ids.len(), "turn-level storage complete (multi-turn session)");
                            }
                            Err(e) => {
                                tracing::warn!(%sid, "turn batch storage failed (non-fatal): {e}")
                            }
                        }
                    } else {
                        tracing::warn!(%sid, expected = embeddable_items.len(), got = turn_embeddings.len(), "embedding count mismatch, storing individually");
                        let emb_type = state.embedding.name().to_string();
                        let emb_dim = state.embedding.dimension() as i32;
                        for (item, emb) in embeddable_items.iter().zip(turn_embeddings.iter()) {
                            let _ = pg
                                .store_turn(
                                    sid,
                                    item.turn_index,
                                    &item.speaker_role,
                                    &item.content,
                                    emb.clone(),
                                    item.metadata.clone(),
                                    &emb_type,
                                    emb_dim,
                                )
                                .await;
                        }
                    }
                }
                Err(e) => tracing::warn!(%sid, "turn-level embed failed (non-fatal): {e}"),
            }
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(StoreNodeResponse {
            id: primary_id,
            message: format!("turn nodes created ({turn_count} turns)"),
            chunk_ids: Some(all_ids),
        }),
    ))
}

/// Store a binary payload (image or audio) using the cross-modal EmbeddingRouter.
async fn store_session_binary(
    state: &AppState,
    content_type: &str,
    body: &[u8],
) -> Result<(StatusCode, Json<StoreNodeResponse>), (StatusCode, String)> {
    let router = state.router.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "cross-modal embedding router not configured".to_string(),
    ))?;

    let vector = router.route(content_type, body).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cross-modal embed failed: {e}"),
        )
    })?;

    let kind = if content_type.starts_with("image/") {
        "image"
    } else {
        "audio"
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "content_type".to_string(),
        Value::String(content_type.to_string()),
    );
    metadata.insert(
        "payload_size".to_string(),
        Value::Number(serde_json::Number::from(body.len())),
    );
    metadata.insert(
        "embedding_source".to_string(),
        Value::String("cross-modal-router".to_string()),
    );
    normalize_node_metadata(
        MemoryType::Episodic,
        MemorySource::Conversation,
        &mut metadata,
    );

    let content = format!(
        "[{}/{}] {} bytes binary payload",
        kind,
        content_type,
        body.len()
    );
    let mut node = FractalNode::new_typed(
        Some(content),
        None,
        vector,
        metadata,
        MemoryType::Episodic,
        MemorySource::Conversation,
    );
    node.importance = 5; // default for binary payloads

    // ── Surprise-weighted salience boost for binary payloads ──
    let mut _binary_surprise = false;
    #[cfg(feature = "postgres-storage")]
    if let Some(ref pg) = state.pg_store {
        match pg.compute_max_novelty_similarity(&vector, 5).await {
            Ok(max_sim) => {
                if let Some((imp_boost, _)) = compute_surprise_boost(max_sim) {
                    node.importance = (node.importance + imp_boost).clamp(1, 10);
                    _binary_surprise = true;
                    tracing::info!(
                        max_sim, importance = node.importance,
                        "surprise boost applied to binary payload node"
                    );
                }
            }
            Err(e) => tracing::warn!("novelty check failed (non-fatal): {e}"),
        }
    }

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    #[cfg(feature = "postgres-storage")]
    if _binary_surprise {
        if let Some(ref pg) = state.pg_store {
            let _ = pg.set_memory_energy(id, SURPRISE_INITIAL_ENERGY).await;
        }
    }

    tracing::info!(%id, %content_type, payload_bytes = body.len(), "binary session node stored");
    Ok((
        StatusCode::CREATED,
        Json(StoreNodeResponse {
            id,
            message: format!("{} payload node created", kind),
            chunk_ids: None,
        }),
    ))
}

// -- Store Session Batch (alle Sessions in EINEM Ollama-Embed-Call) --

#[derive(Deserialize, ToSchema)]
pub struct StoreSessionBatchRequest {
    pub sessions: Vec<StoreSessionRequest>,
}

#[derive(Serialize, ToSchema)]
pub struct StoreSessionBatchResponse {
    pub results: Vec<StoreNodeResponse>,
    pub total_turns: usize,
    pub total_sessions: usize,
}

#[utoipa::path(
    post,
    path = "/store_session_batch",
    tag = "memory",
    request_body = StoreSessionBatchRequest,
    responses(
        (status = 201, description = "All turn nodes created", body = StoreSessionBatchResponse),
        (status = 500, description = "Internal error")
    )
)]
pub async fn store_session_batch(
    State(state): State<AppState>,
    Json(req): Json<StoreSessionBatchRequest>,
) -> Result<(StatusCode, Json<StoreSessionBatchResponse>), (StatusCode, String)> {
    let sessions = req.sessions;
    if sessions.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "sessions array is empty".into()));
    }

    // Phase 1: Turn-split all sessions, collect (session_idx, cleaned_text, original_chunk) triples
    struct TurnWork {
        #[allow(dead_code)]
        session_idx: usize,
        cleaned: String,
        original: String,
    }
    let mut all_turns: Vec<TurnWork> = Vec::new();
    let mut session_turn_ranges: Vec<(usize, usize)> = Vec::with_capacity(sessions.len());

    for (s_idx, session) in sessions.iter().enumerate() {
        let cleaned = clean_for_embedding(&session.content);
        if cleaned.len() < 4 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("session {} content too short after cleaning", s_idx),
            ));
        }
        let chunks = chunk_into_rounds(&session.content, 80);
        let start = all_turns.len();
        for chunk in &chunks {
            let c = clean_for_embedding(chunk);
            if c.len() >= 4 {
                all_turns.push(TurnWork {
                    session_idx: s_idx,
                    cleaned: c,
                    original: chunk.clone(),
                });
            }
        }
        let end = all_turns.len();
        if end == start {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("session {} produced no embeddable turns", s_idx),
            ));
        }
        session_turn_ranges.push((start, end));
    }

    // Phase 2: ONE Ollama embed call for ALL turns
    let refs: Vec<&str> = all_turns.iter().map(|c| c.cleaned.as_str()).collect();
    let vectors = embed_document_batch(&*state.embedding, &refs)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("batch embed failed: {e}"),
            )
        })?;

    // Phase 3: Build turn nodes per session and insert — NO session aggregates
    let mut all_responses: Vec<StoreNodeResponse> = Vec::with_capacity(sessions.len());

    for (s_idx, session) in sessions.iter().enumerate() {
        let (turn_start, turn_end) = session_turn_ranges[s_idx];
        let memory_type = MemoryType::parse(&session.memory_type).unwrap_or(MemoryType::Episodic);
        let source = MemorySource::parse(&session.source).unwrap_or(MemorySource::Conversation);
        let turn_count = turn_end - turn_start;

        let mut turn_ids: Vec<Uuid> = Vec::with_capacity(turn_count);
        for turn_idx in turn_start..turn_end {
            let vector = vectors[turn_idx].clone();
            let work = &all_turns[turn_idx];
            let local_idx = turn_idx - turn_start;
            let (speaker, _) = parse_speaker_role_from_chunk(&work.original)
                .unwrap_or(("assistant", &work.original as &str));

            let mut metadata = session.metadata.clone();
            metadata.insert(
                "speaker_role".to_string(),
                Value::String(speaker.to_string()),
            );
            metadata.insert("is_turn".to_string(), Value::Bool(true));
            metadata.insert(
                "turn_index".to_string(),
                Value::Number(serde_json::Number::from(local_idx)),
            );
            metadata.insert(
                "turn_count".to_string(),
                Value::Number(serde_json::Number::from(turn_count)),
            );
            if let Some(ref sid) = session.session_id {
                metadata.insert("session_id".to_string(), Value::String(sid.clone()));
            }
            normalize_node_metadata(memory_type, source, &mut metadata);

            let mut node = FractalNode::new_typed(
                Some(work.original.clone()),
                None,
                vector.clone(),
                metadata,
                memory_type,
                source,
            );
            if let Some(imp) = session.importance {
                node.importance = imp.clamp(1, 10);
            }
            if let Some(sens) = session.sensitivity {
                node.sensitivity = sens;
            }

            // ── Surprise-weighted salience boost (batch path) ──
            let mut _batch_surprise = false;
            #[cfg(feature = "postgres-storage")]
            if let Some(ref pg) = state.pg_store {
                match pg.compute_max_novelty_similarity(&vector, 5).await {
                    Ok(max_sim) => {
                        if let Some((imp_boost, _)) = compute_surprise_boost(max_sim) {
                            node.importance = (node.importance + imp_boost).clamp(1, 10);
                            _batch_surprise = true;
                        }
                    }
                    Err(e) => tracing::warn!("novelty check failed (non-fatal): {e}"),
                }
            }

            let id = state
                .store
                .insert(node)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            // Post-insert energy boost for surprising batch turns
            #[cfg(feature = "postgres-storage")]
            if _batch_surprise {
                if let Some(ref pg) = state.pg_store {
                    let _ = pg.set_memory_energy(id, SURPRISE_INITIAL_ENERGY).await;
                }
            }
            turn_ids.push(id);

            // ── Turn-level PostgreSQL storage ──
            // Store each turn in conversation_turns so session_id-filtered
            // retrieval works via retrieve_turns_internal.
            #[cfg(feature = "postgres-storage")]
            if let (Some(pg), Some(ref sid)) =
                (state.pg_store.as_ref(), session.session_id.as_ref())
            {
                let emb_type = state.embedding.name().to_string();
                let emb_dim = state.embedding.dimension() as i32;
                let turn_meta = Some(serde_json::to_value(&session.metadata).unwrap_or_default());
                if let Err(e) = pg
                    .store_turn(
                        sid,
                        local_idx as i32,
                        speaker,
                        &work.cleaned,
                        vector,
                        turn_meta,
                        &emb_type,
                        emb_dim,
                    )
                    .await
                {
                    tracing::warn!(%sid, turn = local_idx, "turn storage in pg failed (non-fatal): {e}");
                }
            }
        }

        let primary_id = turn_ids[0]; // Return first turn as primary

        tracing::info!(%primary_id, s_idx, ?memory_type, turns = turn_count, "turn nodes stored (batch, {} turns)", turn_count);

        all_responses.push(StoreNodeResponse {
            id: primary_id,
            message: format!("turn nodes created ({} turns)", turn_count),
            chunk_ids: Some(turn_ids),
        });
    }

    Ok((
        StatusCode::CREATED,
        Json(StoreSessionBatchResponse {
            total_turns: all_turns.len(),
            total_sessions: sessions.len(),
            results: all_responses,
        }),
    ))
}

// -- Store External (Pointer-First: nie Rohdaten, nur Pointer) --

// -- Self-Improve Endpoint ------------------------------------------------
// POST /memory/self_improve
// AI→Memory feedback loop: stores a fact/decision/preference that the
// AI agent explicitly wants to remember for future retrievals.
// Lightweight wrapper over store_session with self-improvement metadata.
//

#[derive(Deserialize, ToSchema)]
pub struct SelfImproveRequest {
    /// The fact, decision, or insight to store.
    pub content: String,
    /// Memory type: decision, preference, semantic, procedural, episodic.
    #[serde(default = "default_semantic_type_str")]
    pub memory_type: String,
    /// Importance 1–10 (default: 5).
    #[serde(default = "default_importance")]
    pub importance: i32,
    /// Optional session_id override.
    #[serde(default)]
    pub session_id: Option<String>,
}

fn default_importance() -> i32 {
    5
}

#[derive(Serialize, ToSchema)]
pub struct SelfImproveResponse {
    pub id: Uuid,
    pub memory_type: String,
    pub importance: i32,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/memory/self_improve",
    tag = "memory",
    request_body = SelfImproveRequest,
    responses(
        (status = 201, description = "Self-improvement memory stored", body = SelfImproveResponse),
        (status = 400, description = "Invalid request", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn self_improve(
    State(state): State<AppState>,
    Json(req): Json<SelfImproveRequest>,
) -> Result<(StatusCode, Json<SelfImproveResponse>), (StatusCode, String)> {
    if req.content.trim().len() < 4 {
        return Err((StatusCode::BAD_REQUEST, "content too short".into()));
    }

    let memory_type = MemoryType::parse(&req.memory_type).unwrap_or(MemoryType::Semantic);
    let source = MemorySource::AiSelfImprovement;
    let importance = req.importance.clamp(1, 10);
    let session_id = req.session_id.unwrap_or_else(|| "standalone".to_string());
    let observed_at = chrono::Utc::now().to_rfc3339();

    let cleaned = clean_for_embedding(&req.content);
    let vector = embed_document(&*state.embedding, &cleaned)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("embed failed: {e}"),
            )
        })?;

    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert("session_id".to_string(), Value::String(session_id.clone()));
    metadata.insert(
        "source_system".to_string(),
        Value::String("hermes_self_improve".to_string()),
    );
    metadata.insert("agent".to_string(), Value::String("hermes".to_string()));
    metadata.insert("role".to_string(), Value::String("ai_agent".to_string()));
    metadata.insert("observed_at".to_string(), Value::String(observed_at));
    metadata.insert(
        "importance".to_string(),
        Value::Number(serde_json::Number::from(importance)),
    );
    normalize_node_metadata(memory_type, source, &mut metadata);

    let mut node = FractalNode::new_typed(
        Some(req.content),
        None,
        vector,
        metadata,
        memory_type,
        source,
    );
    node.importance = importance;

    // ── Surprise-weighted salience boost for self-improvement nodes ──
    let mut _si_surprise = false;
    #[cfg(feature = "postgres-storage")]
    if let Some(ref pg) = state.pg_store {
        match pg.compute_max_novelty_similarity(&vector, 5).await {
            Ok(max_sim) => {
                if let Some((imp_boost, _)) = compute_surprise_boost(max_sim) {
                    node.importance = (node.importance + imp_boost).clamp(1, 10);
                    _si_surprise = true;
                    tracing::info!(
                        max_sim, importance = node.importance,
                        "surprise boost applied to self-improvement node"
                    );
                }
            }
            Err(e) => tracing::warn!("novelty check failed (non-fatal): {e}"),
        }
    }

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    #[cfg(feature = "postgres-storage")]
    if _si_surprise {
        if let Some(ref pg) = state.pg_store {
            let _ = pg.set_memory_energy(id, SURPRISE_INITIAL_ENERGY).await;
        }
    }

    tracing::info!(%id, ?memory_type, importance, "self-improvement memory stored");

    Ok((
        StatusCode::CREATED,
        Json(SelfImproveResponse {
            id,
            memory_type: memory_type.label().to_string(),
            importance,
            message: format!(
                "self-improvement memory stored as {} (importance={})",
                memory_type.label(),
                importance
            ),
        }),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct StoreExternalRequest {
    pub pointer: String,
    /// Content text for embedding (if different from pointer).
    /// When provided, the vector is computed from this content,
    /// not the pointer URI. Falls back to pointer if absent.
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub metadata: HashMap<String, Value>,
    #[serde(default)]
    pub multimodal: Option<MultimodalData>,
    /// Memory type (default: semantic).
    #[serde(default = "default_semantic_type_str")]
    pub memory_type: String,
    /// Source origin (default: import).
    #[serde(default = "default_import_source_str")]
    pub source: String,
    /// Optional importance 1–10.
    #[serde(default)]
    pub importance: Option<i32>,
    /// Optional sensitivity.
    #[serde(default)]
    pub sensitivity: Option<Sensitivity>,
    /// Optional historical timestamp (ISO 8601).
    /// When provided, the node uses this timestamp instead of the current time.
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_semantic_type_str() -> String {
    "semantic".to_string()
}

fn default_import_source_str() -> String {
    "import".to_string()
}

#[utoipa::path(
    post,
    path = "/store_external",
    tag = "memory",
    request_body = StoreExternalRequest,
    responses(
        (status = 201, description = "External pointer node created", body = StoreNodeResponse),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn store_external(
    State(state): State<AppState>,
    Json(req): Json<StoreExternalRequest>,
) -> Result<(StatusCode, Json<StoreNodeResponse>), (StatusCode, String)> {
    let vector = match req.vector {
        Some(v) if !v.is_empty() => v,
        _ => {
            // Embed content if provided, otherwise fall back to pointer
            let text_to_embed = req
                .content
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&req.pointer);
            if let Some(ref mm) = req.multimodal {
                let emb = mm.embedding();
                if !emb.is_empty() {
                    emb.to_vec()
                } else {
                    embed_document(&*state.embedding, text_to_embed)
                        .await
                        .map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("auto-embed failed: {e}"),
                            )
                        })?
                }
            } else {
                embed_document(&*state.embedding, text_to_embed)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("auto-embed failed: {e}"),
                        )
                    })?
            }
        }
    };

    let memory_type = MemoryType::parse(&req.memory_type).unwrap_or(MemoryType::Semantic);
    let source = MemorySource::parse(&req.source).unwrap_or(MemorySource::Import);

    let mut metadata = req.metadata;
    normalize_node_metadata(memory_type, source, &mut metadata);
    let mut node = FractalNode::new_typed(
        req.content.clone(),
        Some(req.pointer.clone()),
        vector,
        metadata,
        memory_type,
        source,
    );
    if let Some(imp) = req.importance {
        node.importance = imp.clamp(1, 10);
    }
    if let Some(sens) = req.sensitivity {
        node.sensitivity = sens;
    }
    if let Some(ts) = req.created_at {
        node.created_at = ts;
    }
    if let Some(mm) = req.multimodal {
        node.multimodal = Some(mm);
    }

    // ── Surprise-weighted salience boost for external nodes ──
    let mut _external_surprise = false;
    #[cfg(feature = "postgres-storage")]
    if let Some(ref pg) = state.pg_store {
        match pg.compute_max_novelty_similarity(&vector, 5).await {
            Ok(max_sim) => {
                if let Some((imp_boost, _initial_energy)) = compute_surprise_boost(max_sim) {
                    node.importance = (node.importance + imp_boost).clamp(1, 10);
                    _external_surprise = true;
                    tracing::info!(
                        max_sim, importance = node.importance,
                        "surprise boost applied to external node"
                    );
                }
            }
            Err(e) => tracing::warn!("novelty check failed (non-fatal): {e}"),
        }
    }

    // ── Dedup: skip if node with same external_id already exists ──
    if let Some(meta) = node.metadata.get("external_id") {
        if let Some(external_id) = meta.as_str() {
            if let Some(existing_id) = state.store.find_by_external_id(external_id).await {
                tracing::info!(
                    %existing_id,
                    external_id,
                    "store_external: duplicate skipped (external_id already exists)"
                );
                return Ok((
                    StatusCode::OK,
                    Json(StoreNodeResponse {
                        id: existing_id,
                        message: "duplicate skipped — external_id already exists".to_string(),
                        chunk_ids: None,
                    }),
                ));
            }
        }
    }

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Post-insert energy boost for surprising external nodes
    #[cfg(feature = "postgres-storage")]
    if _external_surprise {
        if let Some(ref pg) = state.pg_store {
            let _ = pg.set_memory_energy(id, SURPRISE_INITIAL_ENERGY).await;
        }
    }

    tracing::info!(%id, ?memory_type, "external pointer node stored");
    // ── Inline fact extraction for external content ──
    if let Some(ref content) = req.content {
        if content.len() >= 20 {
            let dim = state.embedding.dimension();
            let facts = FactExtractor::extract_facts(content);
            // ── Track schema frequencies (postgres-storage) ──
            #[cfg(feature = "postgres-storage")]
            if let Some(ref pool) = state.trajectory_pool {
                crate::memory::fact_extraction::track_fact_schema_frequencies(pool, &facts).await;
            }
            let ctx = FactExtractionContext {
                session_id: None,
                source_node_id: id,
                embedding_dim: dim,
            };
            let zero_vector = vec![0.0f32; dim];
            let fact_nodes: Vec<FractalNode> = facts
                .into_iter()
                .map(|f| f.to_fractal_node(ctx.source_node_id, ctx.session_id, zero_vector.clone()))
                .collect();
            let fact_count = fact_nodes.len();
            if fact_count > 0 {
                for mut fact_node in fact_nodes {
                    let fact_content = fact_node.content.clone().unwrap_or_default();
                    match embed_document(&*state.embedding, &fact_content).await {
                        Ok(emb) => {
                            fact_node.vector = emb;
                            match state.store.insert(fact_node).await {
                                Ok(fact_id) => {
                                    tracing::debug!(%fact_id, source_id = %id, "inline fact stored (external)")
                                }
                                Err(e) => tracing::debug!("inline fact store failed: {}", e),
                            }
                        }
                        Err(e) => tracing::debug!("inline fact embed failed: {}", e),
                    }
                }
                tracing::debug!(%id, fact_count, "inline facts extracted from external content");
            }
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(StoreNodeResponse {
            id,
            message: "external pointer node created".to_string(),
            chunk_ids: None,
        }),
    ))
}

// ═══════════════════════════════════════════════════════════════════════
// Surprise-weighted salience boost
// ═══════════════════════════════════════════════════════════════════════

/// Threshold below which a new embedding is considered "surprising" (novel).
/// Cosine similarity to the nearest existing node must be below this value
/// to trigger the surprise boost.
const SURPRISE_SIMILARITY_THRESHOLD: f32 = 0.4;

/// Importance boost applied to novel (surprising) memories.
const SURPRISE_IMPORTANCE_BOOST: i32 = 3;

/// Initial energy for novel memories (higher than the DB default of 50),
/// delaying their decay relative to routine content.
const SURPRISE_INITIAL_ENERGY: i32 = 80;

/// Determine whether a new embedding is "surprising" relative to existing memories.
///
/// Returns `Some((importance_boost, initial_energy))` if the new memory is novel
/// (max cosine similarity to any existing node is below the threshold),
/// or `None` if it's similar enough to existing content that no boost is warranted.
///
/// This maps conceptually to Titans' "surprise" metric — embedding-space
/// prediction error (cosine distance to nearest neighbor) as a proxy for
/// gradient-based surprise without requiring gradients.
fn compute_surprise_boost(max_cosine_similarity: f32) -> Option<(i32, i32)> {
    if max_cosine_similarity < SURPRISE_SIMILARITY_THRESHOLD {
        Some((SURPRISE_IMPORTANCE_BOOST, SURPRISE_INITIAL_ENERGY))
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surprise_boost_novel_below_threshold() {
        let result = compute_surprise_boost(0.1);
        assert_eq!(result, Some((3, 80)), "max_sim=0.1 should trigger boost");
    }

    #[test]
    fn surprise_boost_familiar_above_threshold() {
        let result = compute_surprise_boost(0.5);
        assert_eq!(result, None, "max_sim=0.5 should not trigger boost");
    }

    #[test]
    fn surprise_boost_boundary_at_threshold() {
        let result = compute_surprise_boost(0.4);
        assert_eq!(result, None, "max_sim=0.4 (at threshold) should not trigger boost");
    }

    #[test]
    fn surprise_boost_completely_novel() {
        let result = compute_surprise_boost(0.0);
        assert_eq!(result, Some((3, 80)), "max_sim=0.0 (completely novel) should trigger boost");
    }

    #[test]
    fn surprise_boost_almost_identical() {
        let result = compute_surprise_boost(0.95);
        assert_eq!(result, None, "max_sim=0.95 (near-identical) should not trigger boost");
    }

    #[test]
    fn surprise_boost_just_below_threshold() {
        let result = compute_surprise_boost(0.399);
        assert_eq!(result, Some((3, 80)), "max_sim just below threshold should trigger boost");
    }
}

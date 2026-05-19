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
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerStatus};

#[path = "routes/governance_events.rs"]
mod governance_events;
pub use governance_events::*;
#[path = "routes/vlm_webhooks.rs"]
mod vlm_webhooks;
pub use vlm_webhooks::*;

use crate::api::subconscious_qa::{
    is_multi_session_type, is_temporal_question, openai_qa_answer, qa_answer, qa_context_limit,
    source_context_block, source_timestamp,
};

#[derive(Serialize, ToSchema)]
pub struct RetrievalScoreDebug {
    pub profile: RetrievalProfile,
    pub trust_tier: String,
    pub base_score: f32,
    pub multiplier: f32,
    pub final_score: f32,
    pub explanation: String,
    /// The multiplier applied based on the source type classification.
    /// e.g., 0.85 for synthetic, 1.0 for real.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_weight_applied: Option<f32>,
    /// The original source classification (e.g., "real", "synthetic", "derived", "unknown").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_source: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ScoredNode {
    pub score: f32,
    /// Softmax-normalized probability distribution over the candidate set.
    /// Populated when the backend uses distributional scoring (MCE-inspired).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution_scores: Option<Vec<f32>>,
    pub id: Uuid,
    #[serde(default)]
    pub memory_type: MemoryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    #[schema(value_type = Object)]
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub retrieval_profile: RetrievalProfile,
    pub trust_tier: String,
    /// Source-type weight multiplier applied during scoring (e.g., 0.85 for synthetic).
    /// Always present — computed from the node when debug info is unavailable.
    pub source_weight_applied: Option<f32>,
    /// Original source classification (e.g., "real", "synthetic", "derived", "unknown").
    /// Always present — computed from the node when debug info is unavailable.
    pub original_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_debug: Option<RetrievalScoreDebug>,
    /// Governance fields (populated when Stage 2 governance is applied)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Sensitivity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub governance_passed: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub governance_issues: Vec<crate::memory::governance::ValidationIssue>,
    // -- Fractal Hierarchy fields (populated from FractalNode) --
    /// Context tier: raw (L0), summary (L1), or overview (L2).
    /// Omitted from serialization when Raw (the default for 96%+ of nodes) to save bytes.
    #[serde(default)]
    pub context_tier: ContextTier,
    /// ID of the parent tier node (e.g. raw node → its summary).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_tier_id: Option<Uuid>,
    /// IDs of child tier nodes (reverse of parent_tier_id).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children_tier_ids: Vec<Uuid>,
    /// Lifecycle status (active, stale, archived, etc.).
    #[serde(default)]
    pub status: MemoryStatus,
    /// Importance score 1–10.
    #[serde(default)]
    pub importance: i32,
}

impl ScoredNode {
    fn from_storage(entry: crate::storage::ScoredNode, include_debug: bool) -> Self {
        let debug = entry.debug.clone();
        let dist = entry.distribution_scores.clone();
        let score_debug = include_debug.then(|| score_debug_response(debug.as_ref(), &entry.node));
        Self::from_parts(entry.score, entry.node, debug.as_ref(), score_debug, dist)
    }

    fn from_governed_storage(
        entry: crate::storage::ScoredNode,
        governance_passed: bool,
        issues: Vec<crate::memory::governance::ValidationIssue>,
        include_debug: bool,
    ) -> Self {
        let confidence = entry.node.confidence;
        let sensitivity = entry.node.sensitivity;
        let debug = entry.debug.clone();
        let dist = entry.distribution_scores.clone();
        let score_debug = include_debug.then(|| score_debug_response(debug.as_ref(), &entry.node));
        Self::from_parts(entry.score, entry.node, debug.as_ref(), score_debug, dist).with_governance(
            confidence,
            sensitivity,
            governance_passed,
            issues,
        )
    }

    fn from_parts(
        score: f32,
        n: FractalNode,
        debug: Option<&crate::storage::ScoreDebug>,
        score_debug: Option<RetrievalScoreDebug>,
        distribution_scores: Option<Vec<f32>>,
    ) -> Self {
        let trust_tier = debug
            .map(|entry| entry.trust_tier.clone())
            .unwrap_or_else(|| n.trust_tier().to_string());
        // Compute provenance fields: prefer debug info, fall back to node-level detection.
        let (source_weight_applied, original_source) = match debug {
            Some(d) => (d.source_weight_applied, d.original_source.clone()),
            None => {
                let st = crate::retrieval::source_weighting::detect_source_type(&n);
                let weights = crate::retrieval::source_weighting::SourceTypeWeights::default();
                (Some(weights.multiplier(st)), Some(st.to_string()))
            }
        };
        Self {
            score,
            id: n.id,
            memory_type: n.memory_type,
            source: Some(n.source),
            content: n.content,
            original_pointer: n.original_pointer,
            metadata: n.metadata,
            created_at: n.created_at,
            retrieval_profile: debug
                .map(|entry| entry.profile)
                .unwrap_or(RetrievalProfile::FullFidelity),
            trust_tier,
            source_weight_applied,
            original_source,
            score_debug,
            confidence: None,
            sensitivity: None,
            governance_passed: None,
            governance_issues: vec![],
            context_tier: n.context_tier,
            parent_tier_id: n.parent_tier_id,
            children_tier_ids: n.children_tier_ids,
            status: n.status,
            importance: n.importance,
            distribution_scores,
        }
    }

    fn with_governance(
        mut self,
        confidence: f64,
        sensitivity: Sensitivity,
        governance_passed: bool,
        issues: Vec<crate::memory::governance::ValidationIssue>,
    ) -> Self {
        self.confidence = Some(confidence);
        self.sensitivity = Some(sensitivity);
        self.governance_passed = Some(governance_passed);
        self.governance_issues = issues;
        self
    }
}

/// Strip markdown/table/emoji formatting for cleaner embeddings.
pub fn clean_for_embedding(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed == "---"
            || trimmed == "```"
            || trimmed.starts_with("| -")
            || trimmed.starts_with("|--")
        {
            continue;
        }
        let mut cleaned: String = trimmed
            .replace("**", "")
            .replace("##", "")
            .replace('#', "")
            .replace('|', " ")
            .replace("✅", "")
            .replace("❌", "")
            .replace("⚠️", "")
            .replace("🤖", "")
            .replace("🚀", "")
            .replace("🧠", "")
            .replace("```", "");
        // Collapse whitespace
        while cleaned.contains("  ") {
            cleaned = cleaned.replace("  ", " ");
        }
        let cleaned = cleaned.trim().trim_start_matches('-').trim();
        if cleaned.is_empty() || cleaned.len() < 3 {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(cleaned);
    }
    // Ollama-Embedder (z. B. nomic-embed-text-v2-moe) haben oft harte Token-Limits; inkl. Prefix
    // `search_document: ` muss der Prompt unter der Kontextlänge bleiben.
    let max_chars: usize = std::env::var("KNOWWHERE_EMBED_MAX_CHARS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n >= 64)
        .unwrap_or(512);
    if out.len() > max_chars {
        let original_len = out.len();
        let mut end = max_chars;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        tracing::debug!(
            original_len,
            truncated_to = out.len(),
            max_chars,
            "clean_for_embedding: text truncated for embedding"
        );
    }
    out
}
use crate::storage::{HybridQuery, RetrievalProfile, StorageBackend};

/// Parse the speaker role from a chunk's first-line prefix.
/// Returns the canonical role name ("user", "assistant") and strips the prefix
/// from the content. Returns None if no role prefix is detected.
fn parse_speaker_role_from_chunk(chunk: &str) -> Option<(&str, &str)> {
    let first_line = chunk.lines().next()?.trim();
    let role_map: &[(&str, &str)] = &[
        ("user:", "user"),
        ("assistant:", "assistant"),
        ("human:", "user"),
        ("ai:", "assistant"),
        ("User:", "user"),
        ("Assistant:", "assistant"),
        ("Human:", "user"),
        ("AI:", "assistant"),
    ];
    for (prefix, role) in role_map {
        if first_line.starts_with(prefix) {
            let content = first_line[prefix.len()..].trim();
            return Some((role, content));
        }
    }
    None
}

/// Split conversation text into rounds (user+assistant turn pairs).
/// Falls back to the full text as a single chunk when no role prefixes are detected.
/// Chunk text into rounds (dialog turns) or semantic chunks.
///
/// If the text contains role prefixes (user:, assistant:, etc.), splits on
/// turn boundaries. Otherwise falls back to the TextChunker for semantic
/// paragraph/sentence-boundary splitting.
///
fn chunk_into_rounds(text: &str, min_round_chars: usize) -> Vec<String> {
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
        let chunker = crate::memory::TextChunker::new(
            crate::memory::ChunkerConfig::for_nomic_8192(),
        );
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

#[derive(Clone)]
pub struct AppState {
    /// Primary storage backend (trait object for flexibility).
    pub store: Arc<dyn StorageBackend>,
    /// DreamMode and consolidation scheduler need a StorageBackend.
    pub dream_store: Arc<dyn StorageBackend>,
    pub dream: DreamMode,
    pub embedding: Arc<dyn EmbeddingProvider>,
    /// Cross-modal embedding router for content-type based dispatch.
    pub router: Option<Arc<EmbeddingRouter>>,
    /// Active governance policy for Stage 2 retrieval validation.
    pub governance_policy: Arc<RwLock<GovernancePolicy>>,
    /// In-memory event store for Layer 0 (appended to on each mutation).
    /// For production with multiple nodes, use PostgresStore instead.
    pub events: InMemoryEventStore,
    /// PostgreSQL connection pool for trajectory logging and tiered context (postgres-storage feature).
    #[cfg(feature = "postgres-storage")]
    pub trajectory_pool: Option<std::sync::Arc<sqlx::PgPool>>,
    /// PostgresStore handle for turn-level storage, retrieval trajectories, tiered context.
    #[cfg(feature = "postgres-storage")]
    pub pg_store: Option<std::sync::Arc<crate::storage::PostgresStore>>,
    /// VLM background worker handle for async summarization.
    pub vlm_worker: Option<crate::vlm::VlmWorkerHandle>,
    /// Consolidation scheduler for querying cycle_count in /dream/status.
    pub consolidation: Option<std::sync::Arc<crate::scheduler::ConsolidationScheduler>>,
    /// Cross-encoder reranker for two-stage retrieval (feature-gated).
    #[cfg(feature = "reranker")]
    pub reranker: Option<
        std::sync::Arc<std::sync::Mutex<crate::retrieval::cross_encoder::CrossEncoderReranker>>,
    >,
    /// Dedup cache for Frigate webhook events.
    pub frigate_dedup: DedupCache,
    /// Frigate webhook secret (read once at startup, not per-request).
    pub frigate_webhook_secret: Option<String>,
    /// Dedup cache for HomeAssistant webhook events.
    pub homeassistant_dedup: DedupCache,
    /// HomeAssistant webhook secret (read once at startup, not per-request).
    pub homeassistant_webhook_secret: Option<String>,
    /// Server-wide temporal_weight default for hybrid retrieval scoring.
    /// Per-query overrides via `temporal_weight` in RetrieveFractalRequest
    /// take precedence; this is the fallback when the request omits it.
    /// Editable at runtime via GET/POST /config/temporal_weight.
    pub temporal_weight: Arc<RwLock<Option<f32>>>,
    /// Server-wide default source-type weights for provenance-aware retrieval.
    /// Per-query overrides via `source_type_weights` in RetrieveFractalRequest
    /// take precedence; this is the fallback when the request omits it.
    /// Set via `KNOWWHERE_SOURCE_TYPE_WEIGHTS` env var, or `KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE`,
    /// or `source_weights.json` in the working directory (see SourceTypeWeights::from_config).
    pub default_source_type_weights: Option<crate::retrieval::source_weighting::SourceTypeWeights>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

// -- Health Check --

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub node_count: usize,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "Server health status", body = HealthResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let count = state.store.count().await;
    Json(HealthResponse {
        status: "ok".to_string(),
        node_count: count,
    })
}

// -- Embed Text --

#[derive(Deserialize, ToSchema)]
pub struct EmbedRequest {
    pub text: String,
}

#[derive(Serialize, ToSchema)]
pub struct EmbedResponse {
    pub vector: Vec<f32>,
    pub dimension: usize,
    pub provider: String,
}

#[utoipa::path(
    post,
    path = "/embed",
    tag = "embedding",
    request_body = EmbedRequest,
    responses(
        (status = 200, description = "Embedding vector", body = EmbedResponse),
        (status = 500, description = "Embedding failed", body = String)
    )
)]
pub async fn embed_text(
    State(state): State<AppState>,
    Json(req): Json<EmbedRequest>,
) -> Result<Json<EmbedResponse>, (StatusCode, String)> {
    let vector = embed_query(&*state.embedding, &req.text)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let dimension = vector.len();
    let provider = state.embedding.name().to_string();

    Ok(Json(EmbedResponse {
        vector,
        dimension,
        provider,
    }))
}

// -- Store Session --

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

fn set_metadata_text(metadata: &mut HashMap<String, Value>, key: &str, value: &str) {
    metadata.insert(key.to_string(), Value::String(value.to_string()));
}

fn retrieval_profile_hint(profile: RetrievalProfile) -> &'static str {
    match profile {
        RetrievalProfile::UserFacing => {
            "versteckt interne Artefakte und bevorzugt primaere Kontexte"
        }
        RetrievalProfile::AgentDebug => {
            "zeigt auch interne Agentenspuren, gewichtet sie aber leicht herunter"
        }
        RetrievalProfile::FullFidelity => "zeigt rohe Rankings ohne Provenance-Gewichtung",
    }
}

fn score_debug_response(
    debug: Option<&crate::storage::ScoreDebug>,
    node: &FractalNode,
) -> RetrievalScoreDebug {
    let profile = debug
        .map(|entry| entry.profile)
        .unwrap_or(RetrievalProfile::FullFidelity);
    let trust_tier = debug
        .map(|entry| entry.trust_tier.clone())
        .unwrap_or_else(|| node.trust_tier().to_string());
    let base_score = debug.map(|entry| entry.base_score).unwrap_or(1.0);
    let multiplier = debug.map(|entry| entry.multiplier).unwrap_or(1.0);
    let final_score = base_score * multiplier;
    let explanation = format!(
        "{}; trust={} => {:.2} x {:.2} = {:.2}",
        retrieval_profile_hint(profile),
        trust_tier,
        base_score,
        multiplier,
        final_score
    );
    RetrievalScoreDebug {
        profile,
        trust_tier,
        base_score,
        multiplier,
        final_score,
        explanation,
        source_weight_applied: debug.and_then(|d| d.source_weight_applied),
        original_source: debug.and_then(|d| d.original_source.clone()),
    }
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
        set_metadata_text(metadata, FractalNode::TRUST_TIER_KEY, trust_tier);
        metadata
            .entry(FractalNode::RETRIEVAL_VISIBILITY_KEY.to_string())
            .or_insert_with(|| Value::String(FractalNode::INTERNAL_VISIBILITY.to_string()));
        return;
    }
    metadata
        .entry(FractalNode::TRUST_TIER_KEY.to_string())
        .or_insert_with(|| Value::String(trust_tier.to_string()));
}

fn auth_context_or_full_access(auth: Option<Extension<AuthContext>>) -> AuthContext {
    auth.map(|Extension(context)| context)
        .unwrap_or_else(AuthContext::full_access)
}

fn allowed_profiles_list(auth: &AuthContext) -> String {
    auth.allowed_retrieval_profiles
        .iter()
        .map(|profile| profile.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn ensure_retrieval_profile_allowed(
    profile: RetrievalProfile,
    auth: &AuthContext,
) -> Result<(), (StatusCode, String)> {
    if auth.allows_profile(profile) {
        return Ok(());
    }
    Err((
        StatusCode::FORBIDDEN,
        format!(
            "retrieval profile '{}' not allowed for this token; allowed: {}",
            profile.as_str(),
            allowed_profiles_list(auth)
        ),
    ))
}

fn parse_memory_type_filter(
    raw: Option<&String>,
) -> Result<Option<MemoryType>, (StatusCode, String)> {
    match raw {
        Some(value) => MemoryType::parse(value).map(Some).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("unknown memory_type_filter '{}'", value),
            )
        }),
        None => Ok(None),
    }
}

fn retrieval_result_allowed(
    entry: &crate::storage::ScoredNode,
    profile: RetrievalProfile,
    type_filter: Option<MemoryType>,
) -> bool {
    let meta_allowed = if entry.node.memory_type == MemoryType::Meta {
        type_filter == Some(MemoryType::Meta)
    } else {
        true
    };
    profile.allows(&entry.node)
        && meta_allowed
        && type_filter.map_or(true, |filter| entry.node.memory_type == filter)
}

fn is_internal_meta_artifact(node: &ScoredNode) -> bool {
    let content = node.content.as_deref().unwrap_or("").trim().to_ascii_lowercase();
    let derivation = node
        .metadata
        .get(FractalNode::DERIVATION_KEY)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    node.memory_type == MemoryType::Meta
        || derivation == "instruction"
        || content.starts_with("<knowwhere_memory>")
        || content.starts_with("<knowwhere_reflect>")
        || content.starts_with("<memory-context>")
}

fn scrub_response_nodes(nodes: Vec<ScoredNode>, allow_meta: bool) -> Vec<ScoredNode> {
    if allow_meta {
        return nodes;
    }
    let before = nodes.len();
    let cleaned: Vec<ScoredNode> = nodes
        .into_iter()
        .filter(|n| !is_internal_meta_artifact(n))
        .collect();
    let removed = before.saturating_sub(cleaned.len());
    if removed > 0 {
        tracing::warn!(removed, "retrieve_fractal strict scrub removed internal artifacts");
    }
    cleaned
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryIntent {
    CurrentState,
    DecisionWhy,
    Procedure,
    Preference,
    Debug,
    Historical,
    OpenRecall,
}

fn parse_query_intent(raw: Option<&String>, query_text: Option<&String>) -> QueryIntent {
    if let Some(value) = raw {
        match value.trim().to_ascii_lowercase().as_str() {
            "current_state" | "current-state" | "current" => return QueryIntent::CurrentState,
            "decision_why" | "decision-why" | "why" | "decision" => {
                return QueryIntent::DecisionWhy
            }
            "procedure" | "procedural" | "how_to" | "how-to" => return QueryIntent::Procedure,
            "preference" => return QueryIntent::Preference,
            "debug" | "diagnostic" => return QueryIntent::Debug,
            "historical" | "history" => return QueryIntent::Historical,
            _ => {}
        }
    }

    let text = query_text
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if text.contains("gerade")
        || text.contains("aktuell")
        || text.contains("current")
        || text.contains("laeuft")
        || text.contains("läuft")
        || text.contains("status")
    {
        return QueryIntent::CurrentState;
    }
    if text.contains("warum")
        || text.contains("why")
        || text.contains("decision")
        || text.contains("entschied")
    {
        return QueryIntent::DecisionWhy;
    }
    if text.contains("wie starte")
        || text.contains("how to")
        || text.contains("workflow")
        || text.contains("verfahren")
    {
        return QueryIntent::Procedure;
    }
    if text.contains("praeferenz") || text.contains("präferenz") || text.contains("preference") {
        return QueryIntent::Preference;
    }
    QueryIntent::OpenRecall
}

fn scored_metadata_text<'a>(metadata: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    metadata.get(key).and_then(Value::as_str)
}

fn intent_metadata_multiplier(
    intent: QueryIntent,
    memory_type: MemoryType,
    metadata: &HashMap<String, Value>,
) -> f32 {
    let scope = scored_metadata_text(metadata, "claim_scope").unwrap_or("");
    match intent {
        QueryIntent::CurrentState => match scope {
            "current" | "diagnostic" => 1.8,
            "episodic" => 1.2,
            "historical" => 0.85,
            "decision" if memory_type == MemoryType::Decision => 0.35,
            _ if memory_type == MemoryType::Decision => 0.5,
            _ if memory_type == MemoryType::Semantic => 1.2,
            _ => 1.0,
        },
        QueryIntent::DecisionWhy => match memory_type {
            MemoryType::Decision => 1.7,
            MemoryType::Semantic => 1.15,
            MemoryType::Episodic => 0.9,
            _ => 1.0,
        },
        QueryIntent::Procedure => match memory_type {
            MemoryType::Procedural => 1.9,
            MemoryType::Semantic => 1.25,
            MemoryType::Decision => 0.55,
            _ => 1.0,
        },
        QueryIntent::Preference => match memory_type {
            MemoryType::Preference => 1.8,
            MemoryType::Episodic => 1.1,
            MemoryType::Decision => 0.75,
            _ => 1.0,
        },
        QueryIntent::Debug => 1.0,
        QueryIntent::Historical => {
            if scope == "historical" || memory_type == MemoryType::Decision {
                1.25
            } else {
                1.0
            }
        }
        QueryIntent::OpenRecall => 1.0,
    }
}

fn apply_intent_scoring_storage(
    scored: &mut [crate::storage::ScoredNode],
    intent: QueryIntent,
) {
    for entry in scored {
        entry.score *= intent_metadata_multiplier(
            intent,
            entry.node.memory_type,
            &entry.node.metadata,
        );
    }
}

fn evidence_pack_group_key(entry: &crate::storage::ScoredNode) -> String {
    let parent = entry
        .node
        .parent_tier_id
        .map(|u| u.to_string())
        .unwrap_or_default();
    let src0 = entry
        .node
        .metadata
        .get("source_node_ids")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .map(|x| x.to_string())
        .unwrap_or_default();
    let session = entry
        .node
        .metadata
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let ptr = entry.node.original_pointer.as_deref().unwrap_or("");
    if parent.is_empty() && src0.is_empty() && session.is_empty() && ptr.is_empty() {
        return entry.node.id.to_string();
    }
    format!("{parent}|{src0}|{session}|{ptr}")
}

fn evidence_dedupe_storage(mut scored: Vec<crate::storage::ScoredNode>) -> Vec<crate::storage::ScoredNode> {
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
    });
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for s in scored {
        let k = evidence_pack_group_key(&s);
        if seen.insert(k) {
            out.push(s);
        }
    }
    out
}

fn governance_score_multiplier(issues: &[crate::memory::governance::ValidationIssue]) -> f32 {
    issues
        .iter()
        .map(|i| i.score_impact)
        .fold(1.0_f64, |acc, m| acc * m) as f32
}

const MMR_LAMBDA: f32 = 0.65;

fn mmr_rel_score(entry: &crate::storage::ScoredNode, query_vector: &[f32]) -> f32 {
    // Use the entry's composite score (semantic + temporal + session boosts)
    // instead of recomputing raw cosine similarity.
    // The storage layer (PostgresStore or InMemoryStore) already applied
    // the full scoring pipeline: RRF fusion, profile multipliers, temporal
    // weighting, and session boosts.  Using entry.score preserves all of that.
    if !query_vector.is_empty() && !entry.node.vector.is_empty() {
        let raw_cos = crate::memory::fractal_node::cosine_similarity(
            &entry.node.vector, query_vector,
        )
        .clamp(0.0, 1.0);
        // Blend 50% composite score + 50% raw cosine similarity
        // so MMR diversity still has a signal to work with while
        // preserving temporal/session adjustments.
        0.5 * entry.score.max(0.0) + 0.5 * raw_cos
    } else {
        entry.score.max(0.0)
    }
}

fn mmr_max_sim_to_selected(
    cand: &crate::storage::ScoredNode,
    selected: &[crate::storage::ScoredNode],
    query_vector: &[f32],
) -> f32 {
    let mut max_s = 0.0f32;
    for s in selected {
        let mut sim = if !query_vector.is_empty()
            && !cand.node.vector.is_empty()
            && !s.node.vector.is_empty()
        {
            crate::memory::fractal_node::cosine_similarity(&cand.node.vector, &s.node.vector)
        } else {
            0.0
        };
        if evidence_pack_group_key(cand) == evidence_pack_group_key(s) {
            sim += 0.35;
        }
        max_s = max_s.max(sim);
    }
    max_s
}

fn mmr_finalize_storage(
    mut candidates: Vec<crate::storage::ScoredNode>,
    query_vector: &[f32],
    top_k: usize,
) -> Vec<crate::storage::ScoredNode> {
    if top_k == 0 {
        return vec![];
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
    });
    let pool_n = top_k.saturating_mul(10).max(top_k).min(candidates.len());
    let pool: Vec<_> = candidates.into_iter().take(pool_n).collect();
    if pool.len() <= top_k {
        return pool;
    }

    // Snapshot: what would pure-score ranking give?
    let score_top_k_ids: std::collections::HashSet<String> = pool.iter()
        .take(top_k)
        .map(|c| c.id.to_string())
        .collect();

    let max_rel = pool
        .iter()
        .map(|c| mmr_rel_score(c, query_vector))
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let rel: Vec<f32> = pool
        .iter()
        .map(|c| mmr_rel_score(c, query_vector) / max_rel)
        .collect();

    let mut selected: Vec<crate::storage::ScoredNode> = Vec::new();
    let mut cand_idx: Vec<usize> = (0..pool.len()).collect();

    while selected.len() < top_k && !cand_idx.is_empty() {
        let best = *cand_idx
            .iter()
            .max_by(|&&i, &&j| {
                let max_sim_i = mmr_max_sim_to_selected(&pool[i], &selected, query_vector);
                let max_sim_j = mmr_max_sim_to_selected(&pool[j], &selected, query_vector);
                let mmr_i = MMR_LAMBDA * rel[i] - (1.0 - MMR_LAMBDA) * max_sim_i;
                let mmr_j = MMR_LAMBDA * rel[j] - (1.0 - MMR_LAMBDA) * max_sim_j;
                mmr_i
                    .partial_cmp(&mmr_j)
                    .unwrap_or(Ordering::Equal)
            })
            .expect("cand_idx non-empty");

        cand_idx.retain(|&i| i != best);
        selected.push(pool[best].clone());
    }

    // Diagnostic: MMR vs pure-score overlap
    let mmr_top_k_ids: std::collections::HashSet<String> = selected.iter()
        .map(|c| c.id.to_string())
        .collect();
    let overlap: Vec<_> = score_top_k_ids.intersection(&mmr_top_k_ids).collect();
    let new_in_topk: Vec<_> = mmr_top_k_ids.difference(&score_top_k_ids).collect();
    
    // Avg age of top-k
    let now = chrono::Utc::now();
    let avg_age_days = selected.iter()
        .map(|c| (now - c.node.created_at).num_days() as f32)
        .sum::<f32>() / selected.len() as f32;
    
    tracing::info!(
        pool_size = pool_n,
        top_k,
        overlap = overlap.len(),
        displaced = new_in_topk.len(),
        avg_age_days = format!("{:.1}", avg_age_days),
        "MMR finalization — score→MMR overlap diagnostic"
    );

    selected
}

type GovernedStorage = (
    crate::storage::ScoredNode,
    bool,
    Vec<crate::memory::governance::ValidationIssue>,
);

fn evidence_dedupe_governed(items: Vec<GovernedStorage>) -> Vec<GovernedStorage> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tup in items {
        let k = evidence_pack_group_key(&tup.0);
        if seen.insert(k) {
            out.push(tup);
        }
    }
    out
}

fn mmr_finalize_governed(
    mut pool: Vec<GovernedStorage>,
    query_vector: &[f32],
    top_k: usize,
) -> Vec<GovernedStorage> {
    if top_k == 0 {
        return vec![];
    }
    if pool.len() <= top_k {
        return pool;
    }

    let pool_n = top_k.saturating_mul(10).max(top_k).min(pool.len());
    pool.truncate(pool_n);

    let pool_refs: Vec<crate::storage::ScoredNode> = pool.iter().map(|(s, _, _)| s.clone()).collect();

    let max_rel = pool_refs
        .iter()
        .map(|c| mmr_rel_score(c, query_vector))
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let rel: Vec<f32> = pool_refs
        .iter()
        .map(|c| mmr_rel_score(c, query_vector) / max_rel)
        .collect();

    let mut selected_idx: Vec<usize> = Vec::new();
    let mut cand_idx: Vec<usize> = (0..pool.len()).collect();

    while selected_idx.len() < top_k && !cand_idx.is_empty() {
        let best = *cand_idx
            .iter()
            .max_by(|&&i, &&j| {
                let sel_nodes: Vec<crate::storage::ScoredNode> =
                    selected_idx.iter().map(|&ix| pool[ix].0.clone()).collect();
                let max_sim_i = mmr_max_sim_to_selected(&pool[i].0, &sel_nodes, query_vector);
                let max_sim_j = mmr_max_sim_to_selected(&pool[j].0, &sel_nodes, query_vector);
                let mmr_i = MMR_LAMBDA * rel[i] - (1.0 - MMR_LAMBDA) * max_sim_i;
                let mmr_j = MMR_LAMBDA * rel[j] - (1.0 - MMR_LAMBDA) * max_sim_j;
                mmr_i
                    .partial_cmp(&mmr_j)
                    .unwrap_or(Ordering::Equal)
            })
            .expect("cand_idx non-empty");

        cand_idx.retain(|&i| i != best);
        selected_idx.push(best);
    }

    selected_idx.into_iter().map(|i| pool[i].clone()).collect()
}

fn finalize_governed_retrieval(
    mut governed: Vec<GovernedStorage>,
    query_vector: &[f32],
    top_k: usize,
    allow_meta: bool,
) -> Vec<GovernedStorage> {
    if !allow_meta {
        governed.retain(|(entry, _, _)| entry.node.memory_type != MemoryType::Meta);
    }
    governed.sort_by(|(a, _, ia), (b, _, ib)| {
        let ea = a.score * governance_score_multiplier(ia);
        let eb = b.score * governance_score_multiplier(ib);
        eb.partial_cmp(&ea).unwrap_or(Ordering::Equal)
    });
    let governed = evidence_dedupe_governed(governed);
    mmr_finalize_governed(governed, query_vector, top_k)
}

fn finalize_retrieval_storage(
    mut results: Vec<crate::storage::ScoredNode>,
    intent: QueryIntent,
    query_vector: &[f32],
    top_k: usize,
    allow_meta: bool,
) -> Vec<crate::storage::ScoredNode> {
    apply_intent_scoring_storage(&mut results, intent);
    if !allow_meta {
        results.retain(|entry| entry.node.memory_type != MemoryType::Meta);
    }
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
    });
    let results = evidence_dedupe_storage(results);
    mmr_finalize_storage(results, query_vector, top_k)
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
            match pg.store_turn(sid, turn_idx, &speaker, &cleaned, vector.clone(), turn_meta, &emb_type, emb_dim).await {
                Ok(turn_id) => tracing::info!(%turn_id, %sid, turn_idx, %speaker, "turn stored (single-chunk session)"),
                Err(e) => tracing::warn!(%sid, "turn storage failed (non-fatal): {e}"),
            }
        }

        let mut metadata = req.metadata;
        metadata.insert("speaker_role".to_string(), Value::String(speaker.to_string()));
        metadata.insert("is_turn".to_string(), Value::Bool(true));
        if let Some(ref sid) = req.session_id {
            metadata.insert("session_id".to_string(), Value::String(sid.clone()));
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
        let id = state
            .store
            .insert(node)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        tracing::info!(%id, %speaker, ?memory_type, "turn node stored (single-turn session)");
        // ── Inline fact extraction (regex-based, no LLM) ──
        // Extract obvious facts immediately so they're available
        // before async consolidation runs. Creates Decision-type nodes
        // with high weight (2.0) for retrieval boosting.
        if content.len() >= 20 {
            let dim = state.embedding.dimension();
            let ctx = FactExtractionContext {
                session_id: req.session_id.as_deref(),
                source_node_id: id,
                embedding_dim: dim,
            };
            let fact_nodes = FactExtractor::extract_and_create_nodes(&content, &ctx);
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
                                Err(e) => tracing::debug!(
                                    "inline fact store failed: {}",
                                    e
                                ),
                            }
                        }
                        Err(e) => tracing::debug!(
                            "inline fact embed failed: {}",
                            e
                        ),
                    }
                }
                tracing::debug!(
                    %id, fact_count,
                    "inline facts extracted from turn content"
                );
            }
        }
        // Event-driven consolidation: check if there's enough work to justify a run
        if let Some(ref sched) = state.consolidation {
            sched.trigger_if_needed().await;
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
        let (speaker, _) = parse_speaker_role_from_chunk(original_chunk)
            .unwrap_or(("assistant", original_chunk));

        let mut metadata = req.metadata.clone();
        metadata.insert("speaker_role".to_string(), Value::String(speaker.to_string()));
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
        let id = state
            .store
            .insert(node)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        all_ids.push(id);
    }

    let primary_id = all_ids[0]; // Return first turn as primary

    tracing::info!(%primary_id, ?memory_type, turns = turn_count, "turn nodes stored ({} turns)", turn_count);
    // ── Inline fact extraction for multi-turn path ──
    if req.content.len() >= 20 {
        let dim = state.embedding.dimension();
        let ctx = FactExtractionContext {
            session_id: req.session_id.as_deref(),
            source_node_id: primary_id,
            embedding_dim: dim,
        };
        let fact_nodes = FactExtractor::extract_and_create_nodes(&req.content, &ctx);
        let fact_count = fact_nodes.len();
        if fact_count > 0 {
            for mut fact_node in fact_nodes {
                let fact_content = fact_node.content.clone().unwrap_or_default();
                match embed_document(&*state.embedding, &fact_content).await {
                    Ok(emb) => {
                        fact_node.vector = emb;
                        match state.store.insert(fact_node).await {
                            Ok(fact_id) => tracing::debug!(%fact_id, source_id = %primary_id, "inline fact stored (multi-turn)"),
                            Err(e) => tracing::debug!("inline fact store failed: {}", e),
                        }
                    }
                    Err(e) => tracing::debug!("inline fact embed failed: {}", e),
                }
            }
            tracing::debug!(%primary_id, fact_count, "inline facts extracted from session content (multi-turn)");
        }
    }
    // Event-driven consolidation
    if let Some(ref sched) = state.consolidation {
        sched.trigger_if_needed().await;
    }

    // ── Turn-level storage (postgres-storage, multi-turn) ──
    // Store each chunk as an individual turn in conversation_turns so
    // the turn-level HNSW index is populated for fine-grained retrieval.
    #[cfg(feature = "postgres-storage")]
    if let (Some(pg), Some(ref sid)) = (state.pg_store.as_ref(), req.session_id.as_ref()) {
        let mut turn_items: Vec<BatchTurnItem> = Vec::with_capacity(chunks.len());
        let mut turn_texts: Vec<&str> = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            let (speaker, _) = parse_speaker_role_from_chunk(chunk)
                .unwrap_or(("assistant", ""));
            turn_items.push(BatchTurnItem {
                turn_index: i as i32,
                speaker_role: speaker.to_string(),
                content: chunk.clone(),
                metadata: Some(serde_json::to_value(&req.metadata).unwrap_or_default()),
            });
            turn_texts.push(chunk.as_str());
        }
        // Use cleaned texts for embedding (same as fractal node embeddings)
        let cleaned_texts: Vec<String> = turn_texts.iter()
            .map(|t| clean_for_embedding(t))
            .collect();
        let cleaned_refs: Vec<&str> = cleaned_texts.iter()
            .map(|s| s.as_str())
            .filter(|s| s.len() >= 4)
            .collect();
        if !cleaned_refs.is_empty() {
            match embed_document_batch(&*state.embedding, &cleaned_refs).await {
                Ok(turn_embeddings) => {
                    let embeddable_items: Vec<BatchTurnItem> = turn_items.iter()
                        .filter(|item| {
                            let cleaned = clean_for_embedding(&item.content);
                            cleaned.len() >= 4
                        })
                        .cloned()
                        .collect();
                    if embeddable_items.len() == turn_embeddings.len() {
                        let emb_type = state.embedding.name().to_string();
                        let emb_dim = state.embedding.dimension() as i32;
                        match pg.store_turns_batch(sid, &embeddable_items, turn_embeddings, &emb_type, emb_dim).await {
                            Ok((session_uuid, turn_ids)) => {
                                tracing::info!(%session_uuid, turns = turn_ids.len(), "turn-level storage complete (multi-turn session)");
                            }
                            Err(e) => tracing::warn!(%sid, "turn batch storage failed (non-fatal): {e}"),
                        }
                    } else {
                        tracing::warn!(%sid, expected = embeddable_items.len(), got = turn_embeddings.len(), "embedding count mismatch, storing individually");
                        let emb_type = state.embedding.name().to_string();
                        let emb_dim = state.embedding.dimension() as i32;
                        for (item, emb) in embeddable_items.iter().zip(turn_embeddings.iter()) {
                            let _ = pg.store_turn(sid, item.turn_index, &item.speaker_role, &item.content, emb.clone(), item.metadata.clone(), &emb_type, emb_dim).await;
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

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%id, %content_type, payload_bytes = body.len(), "binary session node stored");
    // Event-driven consolidation
    if let Some(ref sched) = state.consolidation {
        sched.trigger_if_needed().await;
    }

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
            metadata.insert("speaker_role".to_string(), Value::String(speaker.to_string()));
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
                vector,
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
            let id = state
                .store
                .insert(node)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            turn_ids.push(id);
        }

        let primary_id = turn_ids[0]; // Return first turn as primary

        tracing::info!(%primary_id, s_idx, ?memory_type, turns = turn_count, "turn nodes stored (batch, {} turns)", turn_count);

        all_responses.push(StoreNodeResponse {
            id: primary_id,
            message: format!("turn nodes created ({} turns)", turn_count),
            chunk_ids: Some(turn_ids),
        });
    }

    // Event-driven consolidation after batch store
    if let Some(ref sched) = state.consolidation {
        sched.trigger_if_needed().await;
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

fn default_importance() -> i32 { 5 }

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
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("embed failed: {e}")))?;

    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert("session_id".to_string(), Value::String(session_id.clone()));
    metadata.insert("source_system".to_string(), Value::String("hermes_self_improve".to_string()));
    metadata.insert("agent".to_string(), Value::String("hermes".to_string()));
    metadata.insert("role".to_string(), Value::String("ai_agent".to_string()));
    metadata.insert("observed_at".to_string(), Value::String(observed_at));
    metadata.insert("importance".to_string(), Value::Number(serde_json::Number::from(importance)));
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

    let id = state
        .store
        .insert(node)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(%id, ?memory_type, importance, "self-improvement memory stored");

    Ok((
        StatusCode::CREATED,
        Json(SelfImproveResponse {
            id,
            memory_type: memory_type.label().to_string(),
            importance,
            message: format!("self-improvement memory stored as {} (importance={})", memory_type.label(), importance),
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
            let text_to_embed = req.content.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(&req.pointer);
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

    // ── Dedup: skip if node with same external_id already exists ──
    if let Some(ref meta) = node.metadata.get("external_id") {
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

    tracing::info!(%id, ?memory_type, "external pointer node stored");
    // ── Inline fact extraction for external content ──
    if let Some(ref content) = req.content {
        if content.len() >= 20 {
            let dim = state.embedding.dimension();
            let ctx = FactExtractionContext {
                session_id: None,
                source_node_id: id,
                embedding_dim: dim,
            };
            let fact_nodes = FactExtractor::extract_and_create_nodes(content, &ctx);
            let fact_count = fact_nodes.len();
            if fact_count > 0 {
                for mut fact_node in fact_nodes {
                    let fact_content = fact_node.content.clone().unwrap_or_default();
                    match embed_document(&*state.embedding, &fact_content).await {
                        Ok(emb) => {
                            fact_node.vector = emb;
                            match state.store.insert(fact_node).await {
                                Ok(fact_id) => tracing::debug!(%fact_id, source_id = %id, "inline fact stored (external)"),
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
    // Event-driven consolidation
    if let Some(ref sched) = state.consolidation {
        sched.trigger_if_needed().await;
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

// -- Retrieve Node by ID --

#[utoipa::path(
    get,
    path = "/retrieve/{id}",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Node UUID")
    ),
    responses(
        (status = 200, description = "Node found", body = FractalNode),
        (status = 404, description = "Node not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn retrieve(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<FractalNode>, (StatusCode, String)> {
    tracing::info!(%id, "retrieving node");
    match state.store.get(&id).await {
        Ok(Some(node)) => Ok(Json(node)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("node {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// -- Fractal Retrieve (Zooming) --

#[derive(Deserialize, ToSchema)]
pub struct RetrieveFractalRequest {
    /// Dense query vector (optional — if omitted, query_text is embedded on-the-fly).
    pub query_vector: Option<Vec<f32>>,
    #[serde(default)]
    pub query_text: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// Apply Stage 2 governance filtering (default: true).
    #[serde(default = "default_governance_enabled")]
    pub governance_enabled: bool,
    /// Filter by memory type.
    #[serde(default)]
    pub memory_type_filter: Option<String>,
    /// Optional retrieval intent hint: current_state, decision_why, procedure, preference, debug, historical.
    #[serde(default)]
    pub query_intent: Option<String>,
    /// Maximum context tier to retrieve: "summary", "overview", or "raw".
    /// Only memories at or below this tier are returned (default: "overview").
    #[serde(default = "default_max_tier")]
    pub max_tier: Option<String>,
    #[serde(default = "default_retrieval_profile")]
    pub retrieval_profile: RetrievalProfile,
    #[serde(default)]
    pub include_debug: bool,
    /// Synthesize a coherent reflection from the top results (default: false).
    /// Uses a small local model (llama3.2:1b) to produce a query-tailored summary.
    #[serde(default)]
    pub reflect: bool,
    /// Max tokens for the reflection output (default: 600).
    #[serde(default = "default_reflect_max_tokens")]
    pub reflect_max_tokens: u32,
    /// Enable temporal diversity sampling to ensure results span multiple
    /// temporal phases (early/middle/late) rather than clustering around
    /// query sentiment. Fixes Retrieval Bias (Issue #1).
    #[serde(default)]
    pub diversity: bool,
    /// Optional contrastive query for explicit negative-phase retrieval.
    /// When set, nodes matching this query are boosted in diversity mode.
    #[serde(default)]
    pub contrastive_query: Option<String>,
    /// Optional user_id filter — scopes retrieval to a single persona's claims.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Enable multi-query expansion (2-3 reformulations, RRF-fused results).
    #[serde(default)]
    pub multi_query: bool,
    /// Temporal recency boost factor (0.0–0.20).
    /// When set, close-scoring results get a slight recency bonus.
    #[serde(default)]
    pub recency_boost: Option<f32>,
    /// Weight for temporal recency in hybrid scoring (0.0 = pure semantic, 1.0 = pure recency).
    /// Recommended: 0.15–0.35. Enables configurable temporal + semantic hybrid.
    #[serde(default)]
    pub temporal_weight: Option<f32>,
    /// Optional session_id for filtering/boosting to reduce session leakage.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional per-source-type score multipliers for provenance-aware retrieval.
    /// Overrides the default SourceTypeWeights (real=1.0, synthetic=0.85, derived=0.70, unknown=0.95).
    #[serde(default)]
    pub source_type_weights: Option<crate::retrieval::source_weighting::SourceTypeWeights>,
    /// Explicit fusion strategy for BM25+dense combination.
    /// When set, overrides auto-routing. Use "dense-only" for pure vector baseline.
    #[serde(default)]
    pub fusion_strategy: Option<crate::storage::FusionStrategy>,
}

fn default_top_k() -> usize {
    5
}
fn default_max_depth() -> usize {
    3
}
fn default_governance_enabled() -> bool {
    true
}
fn default_max_tier() -> Option<String> {
    // Default to None (show all tiers) — users can opt-in to tier filtering
    // by explicitly passing "summary" or "overview" in their request.
    None
}

fn default_reflect_max_tokens() -> u32 {
    600
}

fn default_retrieval_profile() -> RetrievalProfile {
    RetrievalProfile::UserFacing
}

// -- Subconscious Chat --

#[derive(Deserialize, ToSchema)]
pub struct SubconsciousChatRequest {
    pub message: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_governance_enabled")]
    pub governance_enabled: bool,
    #[serde(default)]
    pub persist: bool,
    #[serde(default = "default_retrieval_profile")]
    pub retrieval_profile: RetrievalProfile,
    #[serde(default)]
    pub include_debug: bool,
    #[serde(default)]
    pub question_type: Option<String>,
    #[serde(default)]
    pub question_date: Option<String>,
    #[serde(default = "default_answer_mode")]
    pub answer_mode: String,
    /// Optional user_id filter — scopes retrieval to a single persona's claims.
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct SubconsciousSource {
    pub id: Uuid,
    pub score: f32,
    pub memory_type: MemoryType,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub snippet: String,
    pub retrieval_profile: RetrievalProfile,
    pub trust_tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_debug: Option<RetrievalScoreDebug>,
}

#[derive(Serialize, ToSchema)]
pub struct SubconsciousChatResponse {
    pub answer: String,
    pub sources: Vec<SubconsciousSource>,
    pub stored: bool,
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    match value.char_indices().nth(max_chars) {
        Some((idx, _)) => format!("{}...", &value[..idx]),
        None => value.to_string(),
    }
}

fn default_answer_mode() -> String {
    "context".to_string()
}

fn is_qa_mode(mode: &str) -> bool {
    mode.eq_ignore_ascii_case("qa")
}

fn source_snippet(node: &FractalNode) -> String {
    let raw = node
        .content
        .as_deref()
        .or(node.original_pointer.as_deref())
        .unwrap_or("(no content)");
    truncate_chars(raw, 180)
}

fn chat_persist_metadata(role: &str, derivation: &str) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    set_metadata_text(&mut metadata, FractalNode::ROLE_KEY, role);
    set_metadata_text(&mut metadata, FractalNode::DERIVATION_KEY, derivation);
    set_metadata_text(
        &mut metadata,
        FractalNode::TRUST_TIER_KEY,
        FractalNode::TRUST_DERIVED,
    );
    set_metadata_text(
        &mut metadata,
        FractalNode::RETRIEVAL_VISIBILITY_KEY,
        FractalNode::INTERNAL_VISIBILITY,
    );
    set_metadata_text(&mut metadata, "channel", "subconscious_chat");
    metadata
}

fn compose_subconscious_answer(message: &str, sources: &[SubconsciousSource]) -> String {
    if sources.is_empty() {
        return format!(
            "Ich finde dazu noch keine passende Memory-Spur: \"{}\".",
            message
        );
    }
    let mut lines = vec!["Ich antworte aus deinem aktuellen Memory-Kontext:".to_string()];
    for (idx, source) in sources.iter().enumerate() {
        lines.push(format!("{}. {}", idx + 1, source.snippet));
    }
    lines.join("\n")
}

async fn persist_chat_exchange(
    state: &AppState,
    question: &str,
    answer: &str,
) -> Result<(), StatusCode> {
    let question_vec = embed_document(&*state.embedding, question)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let answer_vec = embed_document(&*state.embedding, answer)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let question_node = FractalNode::new_typed(
        Some(format!("USER: {question}")),
        None,
        question_vec,
        chat_persist_metadata("user", "chat_query"),
        MemoryType::Episodic,
        MemorySource::Conversation,
    );
    let answer_node = FractalNode::new_typed(
        Some(format!("ASSISTANT: {answer}")),
        None,
        answer_vec,
        chat_persist_metadata("assistant", "retrieval_compose"),
        MemoryType::Meta,
        MemorySource::Conversation,
    );
    state
        .store
        .insert(question_node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .store
        .insert(answer_node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

#[utoipa::path(
    post,
    path = "/chat/subconscious",
    tag = "chat",
    request_body = SubconsciousChatRequest,
    responses(
        (status = 200, description = "Subconscious answer", body = SubconsciousChatResponse),
        (status = 400, description = "Invalid request", body = String),
        (status = 500, description = "Server error", body = String)
    )
)]
pub async fn subconscious_chat(
    State(state): State<AppState>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SubconsciousChatRequest>,
) -> Result<Json<SubconsciousChatResponse>, (StatusCode, String)> {
    let auth = auth_context_or_full_access(auth);
    ensure_retrieval_profile_allowed(req.retrieval_profile, &auth)?;
    if req.message.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "message must not be empty".into()));
    }

    let top_k = req.top_k.clamp(1, 20);
    let qa_limit = if is_qa_mode(&req.answer_mode) {
        qa_context_limit(top_k, &req.message, req.question_type.as_deref())
    } else {
        top_k
    };
    let cleaned_message = clean_for_embedding(&req.message);
    let query_vector = embed_query(&*state.embedding, &cleaned_message)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut query = HybridQuery::hybrid(
        req.message.clone(),
        query_vector,
        qa_limit.saturating_mul(2),
        req.max_depth.clamp(1, 6),
    )
    .with_profile(req.retrieval_profile);
    if let Some(ref uid) = req.user_id {
        query = query.with_user_id(uid.clone());
    }
    let results = state
        .store
        .hybrid_retrieve(&query)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let validator = GovernanceValidator::new(state.governance_policy.read().await.clone());
    let filtered_results: Vec<crate::storage::ScoredNode> = results
        .into_iter()
        .filter(|entry| {
            if !req.governance_enabled {
                return true;
            }
            let validation = validator.validate(&entry.node.to_governance_candidate());
            !validation.has_hard_block()
        })
        .take(qa_limit)
        .collect();

    let sources: Vec<SubconsciousSource> = filtered_results
        .iter()
        .take(top_k)
        .cloned()
        .map(|entry| {
            let score_debug = req
                .include_debug
                .then(|| score_debug_response(entry.debug.as_ref(), &entry.node));
            let retrieval_profile = entry
                .debug
                .as_ref()
                .map(|debug| debug.profile)
                .unwrap_or(req.retrieval_profile);
            let trust_tier = entry
                .debug
                .as_ref()
                .map(|debug| debug.trust_tier.clone())
                .unwrap_or_else(|| entry.node.trust_tier().to_string());
            SubconsciousSource {
                id: entry.node.id,
                score: entry.score,
                memory_type: entry.node.memory_type,
                created_at: entry.node.created_at,
                snippet: source_snippet(&entry.node),
                retrieval_profile,
                trust_tier,
                score_debug,
            }
        })
        .collect();

    let answer = if is_qa_mode(&req.answer_mode) {
        let temporal = is_temporal_question(&req.message, req.question_type.as_deref());
        let mut qa_results = filtered_results.clone();
        let sort_chrono = temporal || is_multi_session_type(req.question_type.as_deref());
        if sort_chrono {
            qa_results.sort_by_key(|entry| source_timestamp(&entry.node));
        }
        let contexts: Vec<String> = qa_results
            .iter()
            .map(|entry| {
                source_context_block(&req.message, req.question_type.as_deref(), temporal, entry)
            })
            .collect();
        match qa_answer(
            &req.message,
            req.question_type.as_deref(),
            req.question_date.as_deref(),
            &contexts,
        )
        .await
        {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("qa mode fallback to context answer: {e}");
                compose_subconscious_answer(&req.message, &sources)
            }
        }
    } else {
        compose_subconscious_answer(&req.message, &sources)
    };
    let mut stored = false;
    if req.persist {
        persist_chat_exchange(&state, &req.message, &answer)
            .await
            .map_err(|e| (e, "failed to persist chat exchange".into()))?;
        stored = true;
    }

    Ok(Json(SubconsciousChatResponse {
        answer,
        sources,
        stored,
    }))
}

#[utoipa::path(
    post,
    path = "/retrieve_fractal",
    tag = "memory",
    request_body = RetrieveFractalRequest,
    responses(
        (status = 200, description = "Fractal retrieval results", body = Vec<ScoredNode>)
    )
)]
pub async fn retrieve_fractal(
    State(state): State<AppState>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<RetrieveFractalRequest>,
) -> Result<Json<Vec<ScoredNode>>, (StatusCode, String)> {
    let auth = auth_context_or_full_access(auth);
    ensure_retrieval_profile_allowed(req.retrieval_profile, &auth)?;
    tracing::info!(
        top_k = req.top_k,
        max_depth = req.max_depth,
        has_query_text = req.query_text.is_some(),
        has_query_vector = req.query_vector.is_some(),
        governance = req.governance_enabled,
        max_tier = ?req.max_tier,
        "fractal retrieve"
    );

    // Resolve query vector: use provided vector, or embed query_text on-the-fly
    let query_vector = match &req.query_vector {
        Some(v) => v.clone(),
        None => {
            if let Some(text) = &req.query_text {
                if text.trim().is_empty() {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "query_text must not be empty".into(),
                    ));
                }
                let cleaned = clean_for_embedding(text);
                tracing::info!(query_text = %text, cleaned_len = cleaned.len(), "embedding query text");
                embed_query(&*state.embedding, &cleaned).await.map_err(|e| {
                    tracing::error!("embedding failed: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                })?
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "query_text or query_vector required".into(),
                ));
            }
        }
    };
    tracing::info!(query_vector_dim = query_vector.len(), "using query vector");

    // Parse max_tier filter (default: overview)
    let max_tier = req.max_tier.as_ref().and_then(|s| ContextTier::parse(s));

    let type_filter = parse_memory_type_filter(req.memory_type_filter.as_ref())?;
    let query_intent = parse_query_intent(req.query_intent.as_ref(), req.query_text.as_ref());

    // Resolve temporal_weight: per-query override > server-wide config default.
    let temporal_weight = match req.temporal_weight {
        Some(w) => Some(w),
        None => *state.temporal_weight.read().await,
    };
    tracing::debug!(?temporal_weight, "resolved temporal_weight");

    // Resolve source_type_weights: per-query override > server-wide config default.
    let source_type_weights = req
        .source_type_weights
        .or(state.default_source_type_weights);
    tracing::debug!(?source_type_weights, "resolved source_type_weights");

    // Stage 1: Hybrid retrieval (with optional multi-query expansion)
    let query_vector_for_expand = query_vector.clone();
    let effective_top_k = if req.diversity {
        (req.top_k * 3).max(15)
    } else {
        req.top_k
    };

    let query_vector_for_turns = query_vector.clone();
    let mut results = if req.multi_query {
        // Multi-Query: expand into 2-3 reformulations, retrieve each, RRF-fuse
        let query_text = req.query_text.clone().unwrap_or_default();
        let expansions = crate::retrieval::query_expansion::expand_query(&query_text);
        let mut all_scored: Vec<crate::storage::ScoredNode> = Vec::new();

        for (i, expanded_text) in expansions.iter().enumerate() {
            let expanded_vector = if expanded_text == &query_text {
                query_vector.clone()
            } else {
                let cleaned = clean_for_embedding(expanded_text);
                embed_query(&*state.embedding, &cleaned).await.map_err(|e| {
                    tracing::error!("embed query expansion {i} failed: {e}");
                    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                })?
            };

            let q = HybridQuery {
                query_text: Some(expanded_text.clone()),
                query_vector: Some(expanded_vector),
                top_k: effective_top_k,
                max_depth: req.max_depth,
                profile: req.retrieval_profile,
                memory_type_filter: type_filter,
                user_id: req.user_id.clone(),
                multi_query: false, // prevent recursion
                recency_boost: req.recency_boost,
                temporal_weight,
                fusion_strategy: req.fusion_strategy,
                query_type_routing: false,
                source_type_weights,
            };
            let r = state.store.hybrid_retrieve(&q).await.map_err(|e| {
                tracing::error!("expansion {} hybrid_retrieve failed: {}", i, e);
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            })?;
            tracing::debug!(expansion=i, count=r.len(), query=%expanded_text, "multi-query result");
            all_scored.extend(r);
        }

        // RRF-fuse all expansion results
        let mut scores: std::collections::HashMap<Uuid, f32> = std::collections::HashMap::new();
        for (rank, node) in all_scored.iter().enumerate() {
            *scores.entry(node.id).or_default() += 1.0 / (5.0 + rank as f32 + 1.0);
        }
        let mut fused: Vec<(Uuid, f32)> = scores.into_iter().collect();
        fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let all_nodes = state.store.list_all().await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
        let by_id: std::collections::HashMap<Uuid, FractalNode> =
            all_nodes.into_iter().map(|n| (n.id, n)).collect();
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<crate::storage::ScoredNode> = Vec::new();
        for (id, score) in fused {
            if seen.insert(id) {
                if let Some(node) = by_id.get(&id).cloned() {
                    merged.push(req.retrieval_profile.score_node(score, node, source_type_weights));
                }
            }
        }
        // Distributional scoring over fused RRF candidates (MCE-inspired softmax).
        // Preserved from hybrid_retrieve's own scoring; recomputed here because
        // score_node() rebuilds ScoredNode from scratch, discarding per-expansion scores.
        if !merged.is_empty() {
            let max_score = merged
                .iter()
                .map(|n| n.score)
                .fold(f32::NEG_INFINITY, f32::max);
            let exps: Vec<f32> = merged.iter().map(|n| (n.score - max_score).exp()).collect();
            let sum: f32 = exps.iter().sum();
            if sum > 0.0 {
                let dist: Vec<f32> = exps.iter().map(|e| e / sum).collect();
                for (item, prob) in merged.iter_mut().zip(dist.iter()) {
                    item.distribution_scores = Some(vec![*prob]);
                }
            }
        }

        // Keep as backend::ScoredNode for downstream processing
        // (expand_fractal, reranker, governance all expect backend type)
        merged
    } else {
        // Single-query: unchanged path
        let query = HybridQuery {
            query_text: req.query_text.clone(),
            query_vector: Some(query_vector),
            top_k: effective_top_k,
            max_depth: req.max_depth,
            profile: req.retrieval_profile,
            memory_type_filter: type_filter,
            user_id: req.user_id.clone(),
            multi_query: false,
            recency_boost: req.recency_boost,
            temporal_weight,
            fusion_strategy: req.fusion_strategy,
            query_type_routing: false,
            source_type_weights,
        };
        let r = state.store.hybrid_retrieve(&query).await.map_err(|e| {
            tracing::error!("hybrid_retrieve failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
        r
    };

    // ── Turn-level retrieval (postgres-storage) ──
    // Primary retrieval path against the turn-level embedding index.
    // When session_id is provided, results are scoped to that conversation.
    // This replaces the old WP2 session-level filtering in hybrid_retrieve.
    #[cfg(feature = "postgres-storage")]
    if let Some(pg) = state.pg_store.as_ref() {
        if let Some(ref query_text) = req.query_text {
            if !query_text.trim().is_empty() {
                // Resolve session_id string to UUID for turn-level filtering
                let session_uuid_filter: Option<Uuid> = if let Some(ref sid) = req.session_id {
                    if let Ok(u) = Uuid::parse_str(sid) {
                        Some(u)
                    } else {
                        pg.find_or_create_session(sid).await.ok()
                    }
                } else {
                    None
                };

                match pg.retrieve_turns_internal(&query_vector_for_turns, req.top_k, None, session_uuid_filter).await {
                    Ok(turn_rows) => {
                        if !turn_rows.is_empty() {
                            tracing::info!(
                                turn_count = turn_rows.len(),
                                "turn-level retrieval from conversation_turns index"
                            );
                            for row in turn_rows {
                                // Build turn-level metadata with session identity
                                let mut metadata: HashMap<String, Value> = row
                                    .metadata
                                    .as_ref()
                                    .and_then(|v| v.as_object())
                                    .map(|o| {
                                        o.iter()
                                            .map(|(k, v)| (k.clone(), v.clone()))
                                            .collect()
                                    })
                                    .unwrap_or_default();

                                // Turn identity markers — required for temporal ranking
                                // and turn-level deduplication.
                                metadata.insert("session_id".to_string(), Value::String(row.session_id.to_string()));
                                metadata.insert("speaker_role".to_string(), Value::String(row.speaker_role.clone()));
                                metadata.insert("turn_index".to_string(), Value::Number(serde_json::Number::from(row.turn_index)));
                                metadata.insert("is_turn".to_string(), Value::Bool(true));
                                if let Some(ref ext_id) = row.external_session_id {
                                    metadata.insert("external_session_id".to_string(), Value::String(ext_id.clone()));
                                }

                                // Carry the turn embedding vector for downstream ranking
                                // (fractal zoom, cross-encoder reranker)
                                let vector = row.embedding.unwrap_or_default();

                                let node = FractalNode::new_typed(
                                    Some(row.content),
                                    None,
                                    vector,
                                    metadata,
                                    MemoryType::Episodic,
                                    MemorySource::Conversation,
                                );

                                results.push(crate::storage::ScoredNode {
                                    id: row.turn_id,
                                    score: row.similarity,
                                    distribution_scores: None,
                                    debug: None,
                                    node,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("turn-level internal retrieval failed (non-fatal): {e}");
                    }
                }
            }
        }
    }

    // Stage 1.5: Expand flat results via fractal zoom (children_tier_ids).
    // Uses the query vector to compute child similarity, prunes branches
    // below ZOOM_PRUNING_THRESHOLD (0.7), and follows children up to
    // max_depth levels deep. Default impl returns nodes unchanged.
    let results = state
        .store
        .expand_fractal(
            results,
            &query_vector_for_expand,
            req.max_depth,
            FractalNode::ZOOM_PRUNING_THRESHOLD,
        )
        .await
        .map_err(|e| {
            tracing::error!("expand_fractal failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    // Stage 2: Optional Cross-Encoder reranking (feature-gated)
    #[cfg(feature = "reranker")]
    let (results, _rerank_timing_ms) = {
        if let Some(ref reranker_arc) = state.reranker {
            let rerank_start = std::time::Instant::now();
            let candidates: Vec<crate::retrieval::cross_encoder::RerankCandidate> = results
                .into_iter()
                .map(|s| crate::retrieval::cross_encoder::RerankCandidate {
                    node_id: s.node.id.to_string(),
                    content: s.node.content.clone().unwrap_or_default(),
                    bi_encoder_score: s.score,
                })
                .collect();

            let query_text = req.query_text.as_deref().unwrap_or("");
            // Lock, rerank synchronously, then DROP the guard before any .await
            let reranked_result = {
                let mut reranker = reranker_arc.lock().unwrap();
                reranker.rerank(
                    query_text,
                    candidates,
                    req.top_k,
                    crate::retrieval::cross_encoder::RerankStrategy::default(),
                )
            }; // MutexGuard dropped here — safe to .await now

            let wall_ms = rerank_start.elapsed().as_secs_f64() * 1000.0;

            match reranked_result {
                Ok((reranked, timing)) => {
                    tracing::info!(
                        wall_ms = %format!("{:.1}", wall_ms),
                        inference_ms = %format!("{:.1}", timing.inference_ms),
                        tokenize_ms = %format!("{:.1}", timing.tokenize_ms),
                        candidates = timing.candidate_count,
                        batches = timing.batch_count,
                        "cross-encoder reranking complete"
                    );
                    let mut mapped = Vec::with_capacity(reranked.len());
                    for r in reranked {
                        if let Ok(Some(node)) = state
                            .store
                            .get(&uuid::Uuid::parse_str(&r.node_id).unwrap_or_default())
                            .await
                        {
                            mapped.push(crate::storage::ScoredNode {
                                id: node.id,
                                score: r.cross_encoder_score,
                                distribution_scores: None,
                                debug: None,
                                node,
                            });
                        }
                    }
                    (mapped, Some(wall_ms))
                }
                Err(e) => {
                    tracing::warn!(
                        wall_ms = %format!("{:.1}", wall_ms),
                        "reranking failed, falling back to bi-encoder: {}", e
                    );
                    // Re-retrieve (results was consumed by .into_iter() above)
                    let query_text = req.query_text.clone().unwrap_or_default();
                    let fallback_query = HybridQuery {
                        query_text: Some(query_text.clone()),
                        query_vector: Some(query_vector_for_expand.clone()),
                        top_k: req.top_k,
                        max_depth: req.max_depth,
                        profile: req.retrieval_profile,
                        memory_type_filter: type_filter.clone(),
                        user_id: req.user_id.clone(),
                        multi_query: false,
                        recency_boost: req.recency_boost,
                        temporal_weight,
                        fusion_strategy: None,
                        query_type_routing: false,
                        source_type_weights,
                    };
                    let r = state.store.hybrid_retrieve(&fallback_query).await.map_err(|e| {
                        tracing::error!("fallback hybrid_retrieve after rerank failure: {}", e);
                        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                    })?;
                    (r, None)
                }
            }
        } else {
            (results, None)
        }
    };
    #[cfg(not(feature = "reranker"))]
    let (results, _rerank_timing_ms) = (results, None::<f64>);

    // Stage 2.5: Temporal diversity + contrastive retrieval.
    let mut final_results = results;
    if req.diversity {
        // Run contrastive query if provided, to surface negative/change claims
        if let Some(cq_text) = req.contrastive_query.as_deref() {
            if !cq_text.trim().is_empty() {
                if let Ok(cq_vector) = embed_query(&*state.embedding, cq_text).await {
                    let cq_query = HybridQuery {
                        query_text: Some(cq_text.to_string()),
                        query_vector: Some(cq_vector),
                        top_k: req.top_k,
                        max_depth: 0,
                        profile: req.retrieval_profile,
                        memory_type_filter: type_filter,
                        user_id: req.user_id.clone(),
                        multi_query: false,
                        recency_boost: req.recency_boost,
                        temporal_weight,
                        fusion_strategy: None,
                        query_type_routing: false,
                        source_type_weights,
                    };
                    if let Ok(extra) = state.store.hybrid_retrieve(&cq_query).await {
                        tracing::info!(contrastive = extra.len(), "contrastive results");
                        final_results.extend(extra);
                    }
                }
            }
        }
        let pre_len = final_results.len();
        if final_results.len() > req.top_k {
            final_results = apply_temporal_diversity(final_results, req.top_k, None, state.embedding.as_ref()).await;
            tracing::info!(pre = pre_len, post = final_results.len(), "diversity applied");
        }
    }

    // Apply max_tier filter: only include nodes at or below max_tier
    let max_tier_filter = max_tier;
    let results: Vec<crate::storage::ScoredNode> = if let Some(max_t) = max_tier_filter {
        final_results
            .into_iter()
            .filter(|s| {
                // Higher ordinal = lower tier (Raw=2, Overview=1, Summary=0)
                // Keep node if its tier ordinal <= max_tier ordinal
                s.node.context_tier as usize <= max_t as usize
            })
            .collect()
    } else {
        final_results
    };

    let results: Vec<crate::storage::ScoredNode> = results
        .into_iter()
        .filter(|s| retrieval_result_allowed(s, req.retrieval_profile, type_filter))
        .collect();

    if !req.governance_enabled {
        let allow_meta = type_filter == Some(MemoryType::Meta);
        // Pre-MMR diagnostic: snapshot top-k scores/recency before MMR
        if !results.is_empty() {
            let now = chrono::Utc::now();
            let top_n = req.top_k.min(results.len());
            let avg_score = results.iter().take(top_n)
                .map(|s| s.score).sum::<f32>() / top_n as f32;
            let avg_age = results.iter().take(top_n)
                .map(|s| (now - s.node.created_at).num_days() as f32)
                .sum::<f32>() / top_n as f32;
            let newest_age = results.iter().take(top_n)
                .map(|s| (now - s.node.created_at).num_days() as f32)
                .fold(f32::INFINITY, f32::min);
            tracing::info!(
                top_n,
                avg_score = format!("{:.4}", avg_score),
                avg_age_days = format!("{:.1}", avg_age),
                newest_age_days = format!("{:.1}", newest_age),
                temporal_weight = ?req.temporal_weight,
                "pre-MMR snapshot — scores/recency before finalization"
            );
        }
        let results = finalize_retrieval_storage(
            results,
            query_intent,
            &query_vector_for_expand,
            req.top_k,
            allow_meta,
        );
        let scored: Vec<ScoredNode> = results
            .into_iter()
            .map(|entry| ScoredNode::from_storage(entry, req.include_debug))
            .filter(|s| allow_meta || s.memory_type != MemoryType::Meta)
            .collect();
        return Ok(Json(scrub_response_nodes(scored, allow_meta)));
    }

    // Stage 2: Governance validation
    let validator = GovernanceValidator::new(state.governance_policy.read().await.clone());
    let mut governed: Vec<GovernedStorage> = results
        .into_iter()
        .filter_map(|s| {
            // Apply optional memory type filter
            if let Some(ref filter) = type_filter {
                if s.node.memory_type != *filter {
                    return None;
                }
            }

            let candidate = s.node.to_governance_candidate();
            let validation = validator.validate(&candidate);

            // Hard-blocked nodes (superseded, restricted, invalid status, irrelevant)
            // are excluded from results entirely.
            if validation.has_hard_block() {
                tracing::debug!(node_id = %s.node.id, "excluded by governance: hard block");
                return None;
            }

            Some((
                s,
                validation.passed,
                validation.issues,
            ))
        })
        .collect();

    for (entry, _, _) in &mut governed {
        entry.score *= intent_metadata_multiplier(
            query_intent,
            entry.node.memory_type,
            &entry.node.metadata,
        );
    }

    let allow_meta = type_filter == Some(MemoryType::Meta);
    let governed = finalize_governed_retrieval(governed, &query_vector_for_expand, req.top_k, allow_meta);

    let scored: Vec<ScoredNode> = governed
        .into_iter()
        .map(|(s, passed, issues)| {
            ScoredNode::from_governed_storage(s, passed, issues, req.include_debug)
        })
        .filter(|s| allow_meta || s.memory_type != MemoryType::Meta)
        .collect();
    let scored: Vec<ScoredNode> = scored
        .into_iter()
        .filter(|s| type_filter.map_or(true, |t| s.memory_type == t))
        .collect();
    let scored = scrub_response_nodes(scored, allow_meta);

    // Stage 3: Optional Reflect — synthesize coherent summary from top results
    if req.reflect && !scored.is_empty() && type_filter.is_none() {
        let reflector = crate::reflector::Reflector::new();
        if let Some(ref reflector) = reflector {
            let query = req.query_text.as_deref().unwrap_or("");
            if !query.is_empty() {
                match {
                    // Build chunk summaries for the reflector.
                    // Skip Episodic nodes — raw transcripts add too much noise
                    // and dilute synthesis quality of the small reflect model.
                    // Kept: Decision, Semantic, Preference, Procedural, Meta.
                    let chunks: Vec<crate::storage::ScoredNode> = scored
                        .iter()
                        .filter(|s| s.memory_type != MemoryType::Episodic)
                        .map(|s| {
                            use crate::storage::ScoredNode as StorageScoredNode;
                            let mut meta_map: HashMap<String, serde_json::Value> = HashMap::new();
                            for (k, v) in &s.metadata {
                                if let Some(vs) = v.as_str() {
                                    meta_map.insert(
                                        k.clone(),
                                        serde_json::Value::String(vs.to_string()),
                                    );
                                }
                            }
                            let mut node = crate::memory::FractalNode::new_typed(
                                s.content.clone(),
                                None,
                                vec![0.0; 1024],
                                meta_map,
                                s.memory_type,
                                crate::memory::MemorySource::Consolidation,
                            );
                            node.id = s.id;
                            StorageScoredNode {
                                id: s.id,
                                score: s.score,
                                distribution_scores: None,
                                node,
                                debug: None,
                            }
                        })
                        .collect();

                    // Fallback: if all results are episodic (rare), pass all nodes
                    // so the reflector doesn't produce an empty synthesis.
                    let chunks = if chunks.is_empty() {
                        scored
                            .iter()
                            .map(|s| {
                                use crate::storage::ScoredNode as StorageScoredNode;
                                let mut meta_map: HashMap<String, serde_json::Value> =
                                    HashMap::new();
                                for (k, v) in &s.metadata {
                                    if let Some(vs) = v.as_str() {
                                        meta_map.insert(
                                            k.clone(),
                                            serde_json::Value::String(vs.to_string()),
                                        );
                                    }
                                }
                                let mut node = crate::memory::FractalNode::new_typed(
                                    s.content.clone(),
                                    None,
                                    vec![0.0; 1024],
                                    meta_map,
                                    s.memory_type,
                                    crate::memory::MemorySource::Consolidation,
                                );
                                node.id = s.id;
                                StorageScoredNode {
                                    id: s.id,
                                    score: s.score,
                                    distribution_scores: None,
                                    node,
                                    debug: None,
                                }
                            })
                            .collect()
                    } else {
                        chunks
                    };

                    reflector.reflect_on_chunks(&chunks, query).await
                } {
                    Ok(reflection) if !reflection.is_empty() => {
                        // Prepend synthetic reflection node with max score
                        let reflection_node = ScoredNode {
                            id: uuid::Uuid::new_v4(),
                            score: 1.0,
                            memory_type: MemoryType::Meta,
                            source: Some(MemorySource::Consolidation),
                            content: Some(reflection),
                            original_pointer: None,
                            metadata: {
                                let mut m = HashMap::new();
                                m.insert(
                                    "derivation".to_string(),
                                    serde_json::Value::String("reflected".to_string()),
                                );
                                m
                            },
                            created_at: chrono::Utc::now(),
                            retrieval_profile: RetrievalProfile::UserFacing,
                            trust_tier: "primary".to_string(),
                            source_weight_applied: Some(1.0),
                            original_source: Some("synthetic".to_string()),
                            score_debug: None,
                            confidence: Some(0.98),
                            sensitivity: Some(Sensitivity::Normal),
                            governance_passed: Some(true),
                            governance_issues: vec![],
                            context_tier: ContextTier::Raw,
                            parent_tier_id: None,
                            children_tier_ids: vec![],
                            status: MemoryStatus::Active,
                            importance: 5,
                            distribution_scores: None,
                        };
                        let mut with_reflection = vec![reflection_node];
                        with_reflection.extend(scored);
                        return Ok(Json(with_reflection));
                    }
                    Ok(_) => {
                        tracing::debug!(
                            "reflect produced empty output, returning unscored results"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("reflect failed (non-fatal): {}", e);
                    }
                }
            }
        } else {
            tracing::debug!("reflector not available (Ollama not reachable)");
        }
    }

    let scored = scrub_response_nodes(scored, allow_meta);
    tracing::info!(
        response_len = scored.len(),
        response_meta = scored.iter().filter(|n| n.memory_type == MemoryType::Meta).count(),
        recency_boost = req.recency_boost.map(|r| format!("{:.2}", r)),
        "retrieve_fractal response stats"
    );
    Ok(Json(scored))
}

/// Apply temporal diversity to retrieval results (Issue #1 — Retrieval Bias fix).
///
/// Vector search clusters around query sentiment. A positive query retrieves
/// positive nodes, missing problems from earlier phases. This function groups
/// candidates by temporal phase (early/middle/late based on turn_index/claim_index)
/// and ensures at least one node from each phase appears in the final top_k.
///
/// Algorithm:
/// 1. Extract turn_index from each node's metadata
/// 2. Group into temporal buckets (no phase = uncategorized)
/// 3. Take top nodes from each bucket proportionally
/// 4. Fill remaining slots with highest-scoring overall
async fn apply_temporal_diversity(
    candidates: Vec<crate::storage::ScoredNode>,
    top_k: usize,
    _contrastive_query: Option<&str>,
    _embedding: &(dyn crate::embedding::EmbeddingProvider + Send + Sync),
) -> Vec<crate::storage::ScoredNode> {
    if candidates.len() <= top_k {
        return candidates;
    }

    // Extract temporal phase from metadata
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TemporalPhase {
        Early,
        Middle,
        Late,
        Unknown,
    }

    fn get_phase(node: &crate::memory::FractalNode) -> (TemporalPhase, i64) {
        let meta = &node.metadata;
        // Try claim_index first (preferred), then turn_index (legacy)
        let ti = meta
            .get("claim_index")
            .or_else(|| meta.get("turn_index"))
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);

        if ti < 0 {
            return (TemporalPhase::Unknown, ti);
        }

        // Heuristic: < 3 = early, 3-5 = middle, > 5 = late
        let phase = if ti < 3 {
            TemporalPhase::Early
        } else if ti <= 5 {
            TemporalPhase::Middle
        } else {
            TemporalPhase::Late
        };
        (phase, ti)
    }

    // Group by phase
    let mut early: Vec<(f32, usize)> = Vec::new();
    let mut middle: Vec<(f32, usize)> = Vec::new();
    let mut late: Vec<(f32, usize)> = Vec::new();
    let mut unknown: Vec<(f32, usize)> = Vec::new();

    for (idx, candidate) in candidates.iter().enumerate() {
        let (phase, _) = get_phase(&candidate.node);
        let entry = (candidate.score, idx);
        match phase {
            TemporalPhase::Early => early.push(entry),
            TemporalPhase::Middle => middle.push(entry),
            TemporalPhase::Late => late.push(entry),
            TemporalPhase::Unknown => unknown.push(entry),
        }
    }

    // Sort each group by score descending
    early.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    middle.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    late.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let groups: Vec<(&[(f32, usize)], &str)> = vec![
        (&early, "early"),
        (&middle, "middle"),
        (&late, "late"),
    ];

    // Count non-empty groups for proportional allocation
    let active_groups: Vec<_> = groups.iter().filter(|(g, _)| !g.is_empty()).collect();
    let _n_groups = active_groups.len().max(1);

    // Allocate at least 1 slot per non-empty group, then proportional fill
    let mut selected_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Guarantee at least 1 from each non-empty phase
    for (group, _) in &active_groups {
        if let Some((_, idx)) = group.first() {
            selected_indices.insert(*idx);
        }
    }

    // Fill remaining slots: merge all groups, sort by score, pick unselected
    let mut all_ranked: Vec<(f32, usize)> = Vec::new();
    for (group, _) in &groups {
        all_ranked.extend_from_slice(group);
    }
    all_ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    for (_, idx) in &all_ranked {
        if selected_indices.len() >= top_k {
            break;
        }
        selected_indices.insert(*idx);
    }

    // Build result preserving original order within phases
    let mut result: Vec<crate::storage::ScoredNode> = Vec::with_capacity(top_k);
    let mut sorted_indices: Vec<usize> = selected_indices.into_iter().collect();
    sorted_indices.sort(); // preserve original ordering

    for idx in sorted_indices {
        if idx < candidates.len() {
            result.push(candidates[idx].clone());
        }
    }

    result
}

pub async fn retrieve_fractal_safe(
    state: State<AppState>,
    auth: Option<Extension<AuthContext>>,
    req: Json<RetrieveFractalRequest>,
) -> Result<Json<Vec<ScoredNode>>, (StatusCode, String)> {
    let allow_meta = req.0.memory_type_filter.as_deref() == Some("meta");
    let Json(nodes) = retrieve_fractal(state, auth, req).await?;
    Ok(Json(scrub_response_nodes(nodes, allow_meta)))
}

// ---------------------------------------------------------------------------
//  POST /rerank — Standalone Cross-Encoder Reranking
// ---------------------------------------------------------------------------

/// Request body for standalone reranking.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RerankRequest {
    /// The query text to score against.
    pub query: String,
    /// Candidates from a prior retrieval step.
    pub candidates: Vec<RerankCandidateInput>,
    /// Number of top results to return (default: 10).
    #[serde(default = "default_rerank_top_n")]
    pub top_n: usize,
    /// Merge strategy for bi-encoder + cross-encoder scores.
    #[serde(default)]
    pub strategy: RerankStrategyParam,
}

fn default_rerank_top_n() -> usize {
    10
}

/// A candidate from prior retrieval, as JSON input.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RerankCandidateInput {
    /// Unique node identifier.
    pub node_id: String,
    /// Text content of the candidate.
    pub content: String,
    /// Original Bi-Encoder similarity score (0.0–1.0).
    #[serde(default)]
    pub bi_encoder_score: f32,
}

/// Reranking strategy parameter (deserialized from JSON).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RerankStrategyParam {
    /// Use Cross-Encoder score only (default).
    CrossEncoderOnly,
    /// Weighted merge: alpha * cross_encoder + (1-alpha) * normalized_bi_encoder.
    MergedRrf {
        /// Weight for cross-encoder score (0.0–1.0, default: 0.7).
        #[serde(default = "default_alpha")]
        alpha: f32,
    },
}

fn default_alpha() -> f32 {
    0.7
}

impl Default for RerankStrategyParam {
    fn default() -> Self {
        RerankStrategyParam::CrossEncoderOnly
    }
}

/// Response from the reranking endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct RerankResponse {
    /// Reranked results.
    pub results: Vec<RerankedResultOutput>,
    /// Strategy used for scoring.
    pub strategy: String,
    /// Total wall-clock time in milliseconds.
    pub timing_ms: f64,
}

/// A single reranked result.
#[derive(Debug, Serialize, ToSchema)]
pub struct RerankedResultOutput {
    pub node_id: String,
    pub content: String,
    pub bi_encoder_score: f32,
    pub cross_encoder_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_score: Option<f32>,
}

#[utoipa::path(
    post,
    path = "/rerank",
    tag = "memory",
    request_body = RerankRequest,
    responses(
        (status = 200, description = "Reranked results", body = RerankResponse),
        (status = 400, description = "Bad request (reranker not available)"),
        (status = 500, description = "Reranking failed")
    )
)]
pub async fn rerank(
    State(state): State<AppState>,
    Json(req): Json<RerankRequest>,
) -> Result<Json<RerankResponse>, (StatusCode, String)> {
    let start = std::time::Instant::now();

    // Feature-gated: reranker must be loaded
    #[cfg(not(feature = "reranker"))]
    {
        let _ = (&state, &req, &start);
        return Err((
            StatusCode::BAD_REQUEST,
            "reranker feature not enabled. Rebuild with --features reranker".into(),
        ));
    }

    #[cfg(feature = "reranker")]
    {
        let reranker_arc = state.reranker.as_ref().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "reranker not loaded. Run scripts/export_reranker_model.py and restart the server."
                    .into(),
            )
        })?;

        let candidates: Vec<crate::retrieval::cross_encoder::RerankCandidate> = req
            .candidates
            .into_iter()
            .map(|c| crate::retrieval::cross_encoder::RerankCandidate {
                node_id: c.node_id,
                content: c.content,
                bi_encoder_score: c.bi_encoder_score,
            })
            .collect();

        let strategy = match req.strategy {
            RerankStrategyParam::CrossEncoderOnly => {
                crate::retrieval::cross_encoder::RerankStrategy::CrossEncoderOnly
            }
            RerankStrategyParam::MergedRrf { alpha } => {
                crate::retrieval::cross_encoder::RerankStrategy::MergedRrf { alpha }
            }
        };

        let strategy_name = match strategy {
            crate::retrieval::cross_encoder::RerankStrategy::CrossEncoderOnly => {
                "cross_encoder_only"
            }
            crate::retrieval::cross_encoder::RerankStrategy::MergedRrf { .. } => "merged_rrf",
        };

        tracing::info!(
            query = %req.query,
            candidate_count = candidates.len(),
            top_n = req.top_n,
            strategy = strategy_name,
            "reranking"
        );

        let mut reranker = reranker_arc.lock().unwrap();
        let (results, ce_timing) = reranker
            .rerank(&req.query, candidates, req.top_n, strategy)
            .map_err(|e| {
                tracing::error!("reranking failed: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            })?;

        let timing_ms = start.elapsed().as_secs_f64() * 1000.0;

        let output: Vec<RerankedResultOutput> = results
            .into_iter()
            .map(|r| {
                let final_score = Some(r.cross_encoder_score);
                RerankedResultOutput {
                    node_id: r.node_id,
                    content: r.content,
                    bi_encoder_score: r.bi_encoder_score,
                    cross_encoder_score: r.cross_encoder_score,
                    final_score,
                }
            })
            .collect();

        tracing::info!(
            result_count = output.len(),
            wall_ms = %format!("{:.1}", timing_ms),
            inference_ms = %format!("{:.1}", ce_timing.inference_ms),
            tokenize_ms = %format!("{:.1}", ce_timing.tokenize_ms),
            candidates = ce_timing.candidate_count,
            batches = ce_timing.batch_count,
            "reranking complete"
        );

        Ok(Json(RerankResponse {
            results: output,
            strategy: strategy_name.to_string(),
            timing_ms,
        }))
    }
}

// -- Delete Node --

#[utoipa::path(
    delete,
    path = "/nodes/{id}",
    tag = "memory",
    params(
        ("id" = Uuid, Path, description = "Node UUID to delete")
    ),
    responses(
        (status = 200, description = "Node deleted"),
        (status = 404, description = "Node not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn delete_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StoreNodeResponse>, (StatusCode, String)> {
    tracing::info!(%id, "deleting node");
    match state.store.delete(&id).await {
        Ok(true) => Ok(Json(StoreNodeResponse {
            id,
            message: "node deleted".to_string(),
            chunk_ids: None,
        })),
        Ok(false) => Err((StatusCode::NOT_FOUND, format!("node {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// -- Batch Delete Nodes --

#[derive(Deserialize, ToSchema)]
pub struct BatchDeleteRequest {
    pub ids: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct BatchDeleteResponse {
    pub deleted: usize,
    pub not_found: usize,
}

#[utoipa::path(
    post,
    path = "/nodes/batch_delete",
    tag = "memory",
    request_body = BatchDeleteRequest,
    responses(
        (status = 200, description = "Nodes deleted", body = BatchDeleteResponse),
        (status = 500, description = "Internal error")
    )
)]
pub async fn batch_delete_nodes(
    State(state): State<AppState>,
    Json(req): Json<BatchDeleteRequest>,
) -> Result<Json<BatchDeleteResponse>, (StatusCode, String)> {
    let mut deleted = 0usize;
    let mut not_found = 0usize;
    for id_str in &req.ids {
        match Uuid::parse_str(id_str) {
            Ok(id) => match state.store.delete(&id).await {
                Ok(true) => deleted += 1,
                Ok(false) => not_found += 1,
                Err(_) => not_found += 1,
            },
            Err(_) => not_found += 1,
        }
    }
    tracing::info!(
        deleted,
        not_found,
        total = req.ids.len(),
        "batch delete complete"
    );
    Ok(Json(BatchDeleteResponse { deleted, not_found }))
}

// -- Deduplicate by external_id --

#[derive(Serialize, ToSchema)]
pub struct DedupResponse {
    pub total_nodes: usize,
    pub groups: usize,
    pub duplicates_removed: usize,
    pub errors: usize,
}

#[utoipa::path(
    post,
    path = "/nodes/deduplicate",
    tag = "memory",
    responses(
        (status = 200, description = "Deduplication complete", body = DedupResponse)
    )
)]
pub async fn deduplicate_nodes(
    State(state): State<AppState>,
) -> Result<Json<DedupResponse>, (StatusCode, String)> {
    let all = state
        .store
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = all.len();

    // Group by external_id
    let mut by_eid: std::collections::HashMap<String, Vec<(Uuid, chrono::DateTime<chrono::Utc>)>> =
        std::collections::HashMap::new();
    for node in &all {
        if let Some(eid) = node
            .metadata
            .get("external_id")
            .and_then(|v| v.as_str())
        {
            by_eid
                .entry(eid.to_string())
                .or_default()
                .push((node.id, node.created_at));
        }
    }

    let groups = by_eid.len();
    let mut to_delete = Vec::new();

    for (_eid, mut nodes) in by_eid {
        if nodes.len() > 1 {
            nodes.sort_by_key(|(_, ts)| *ts);
            // Keep first (oldest), delete rest
            for (id, _) in &nodes[1..] {
                to_delete.push(id.to_string());
            }
        }
    }

    let dup_count = to_delete.len();
    let mut removed = 0usize;
    let mut errors = 0usize;

    for id_str in &to_delete {
        match Uuid::parse_str(id_str) {
            Ok(id) => match state.store.delete(&id).await {
                Ok(true) => removed += 1,
                _ => errors += 1,
            },
            Err(_) => errors += 1,
        }
    }

    tracing::info!(
        total,
        groups,
        duplicates = dup_count,
        removed,
        errors,
        "deduplicate complete"
    );

    Ok(Json(DedupResponse {
        total_nodes: total,
        groups,
        duplicates_removed: removed,
        errors,
    }))
}

// -- Purge Dummy Nodes --

#[derive(Serialize, ToSchema)]
pub struct PurgeResponse {
    pub removed: usize,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/nodes/purge_dummy",
    tag = "memory",
    responses(
        (status = 200, description = "Dummy nodes purged", body = PurgeResponse)
    )
)]
pub async fn purge_dummy(State(state): State<AppState>) -> Json<PurgeResponse> {
    let removed = state.store.purge_dummy_vectors().await;
    tracing::info!(removed, "purged dummy-vector nodes");
    Json(PurgeResponse {
        removed,
        message: format!("{removed} dummy nodes removed"),
    })
}

// -- Recent Nodes --

#[derive(Deserialize, IntoParams)]
pub struct RecentQuery {
    #[serde(default = "default_recent_limit")]
    pub limit: usize,
}

fn default_recent_limit() -> usize {
    20
}

#[utoipa::path(
    get,
    path = "/nodes/recent",
    tag = "memory",
    params(RecentQuery),
    responses(
        (status = 200, description = "Recent nodes", body = Vec<FractalNode>)
    )
)]
pub async fn recent_nodes(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RecentQuery>,
) -> Result<Json<Vec<FractalNode>>, StatusCode> {
    let limit = q.limit.min(100);
    let nodes = state.store.recent(limit).await.map_err(|e| {
        tracing::error!("recent failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(nodes))
}

// -- Re-embed All Nodes --

#[derive(Serialize, ToSchema)]
pub struct ReembedResponse {
    pub updated: usize,
    pub failed: usize,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/nodes/reembed_all",
    tag = "memory",
    responses(
        (status = 200, description = "Re-embedding complete", body = ReembedResponse)
    )
)]
pub async fn reembed_all(State(state): State<AppState>) -> Json<ReembedResponse> {
    let all_nodes = state.store.list_all().await.unwrap_or_default();
    let mut updated = 0usize;
    let mut failed = 0usize;

    for node in &all_nodes {
        let text = match (&node.content, &node.original_pointer) {
            (Some(c), _) => clean_for_embedding(c),
            (_, Some(p)) => p.clone(),
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }

        match embed_document(&*state.embedding, &text).await {
            Ok(vec) => {
                if state
                    .store
                    .update_vector(&node.id, vec)
                    .await
                    .unwrap_or(false)
                {
                    updated += 1;
                } else {
                    failed += 1;
                }
            }
            Err(e) => {
                tracing::warn!(%node.id, "re-embed failed: {}", e);
                failed += 1;
            }
        }
    }

    Json(ReembedResponse {
        updated,
        failed,
        message: format!("re-embedded {updated} nodes, {failed} failed"),
    })
}

// -- Repair Embedding Dimensions (1536 → 1024) --

#[derive(Serialize, ToSchema)]
pub struct RepairEmbeddingsResponse {
    pub scanned: usize,
    pub repaired: usize,
    pub skipped: usize,
    pub target_dimension: usize,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/maintenance/repair_embeddings",
    tag = "maintenance",
    responses(
        (status = 200, description = "Embedding repair complete", body = RepairEmbeddingsResponse),
        (status = 403, description = "Admin only")
    )
)]
pub async fn repair_embeddings(
    State(state): State<AppState>,
    auth: axum::Extension<AuthContext>,
) -> Result<Json<RepairEmbeddingsResponse>, StatusCode> {
    // Admin-only: require admin API key
    if !auth.is_admin() {
        return Err(StatusCode::FORBIDDEN);
    }

    match state
        .store
        .repair_embedding_dimensions(&*state.embedding)
        .await
    {
        Ok(report) => Ok(Json(RepairEmbeddingsResponse {
            scanned: report.scanned,
            repaired: report.repaired,
            skipped: report.skipped,
            target_dimension: report.target_dimension,
            message: format!(
                "Repaired {} of {} memories ({} skipped). Target dimension: {}",
                report.repaired, report.scanned, report.skipped, report.target_dimension
            ),
        })),
        Err(e) => {
            tracing::error!("Embedding repair failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// -- Dream Status --

#[utoipa::path(
    get,
    path = "/dream/status",
    tag = "system",
    responses(
        (status = 200, description = "Dream mode status", body = DreamStatus)
    )
)]
pub async fn dream_status(State(state): State<AppState>) -> Json<DreamStatus> {
    let mut status = state.dream.status().await;
    if let Some(ref scheduler) = state.consolidation {
        status.cycle_count = scheduler.cycle_count();
        scheduler.populate_dream_status(&mut status);
    }
    Json(status)
}

// -- Consolidation Force --

/// Response for POST /consolidation/force.
#[derive(Serialize, ToSchema)]
pub struct ForceConsolidationResponse {
    pub accepted: bool,
    pub candidates_found: usize,
    pub total_nodes: usize,
    pub message: String,
}

/// POST /consolidation/force — trigger full re-consolidation of all pending nodes.
///
/// Bypasses the space-amplification ratio and timer safety-net. Processes ALL
/// eligible candidates (no cap). The consolidation runs in a background task;
/// this endpoint returns immediately with 202 Accepted.
///
/// Use GET /dream/status to monitor progress via `cycle_count`.
#[utoipa::path(
    post,
    path = "/consolidation/force",
    tag = "system",
    responses(
        (status = 202, description = "Consolidation started in background", body = ForceConsolidationResponse),
        (status = 200, description = "No pending candidates", body = ForceConsolidationResponse),
        (status = 503, description = "Consolidation scheduler not available", body = String)
    )
)]
pub async fn force_consolidation(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<ForceConsolidationResponse>), (StatusCode, String)> {
    let scheduler = match &state.consolidation {
        Some(s) => s.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "consolidation scheduler not available (DREAM_ENABLED=false?)".into(),
            ));
        }
    };

    let (candidates, total) = scheduler.pending_count().await;

    if candidates == 0 {
        return Ok((
            StatusCode::OK,
            Json(ForceConsolidationResponse {
                accepted: false,
                candidates_found: 0,
                total_nodes: total,
                message: "no pending consolidation candidates".into(),
            }),
        ));
    }

    // Spawn consolidation in background — the HTTP response returns immediately
    tokio::spawn(async move {
        let (enqueued, failed, elapsed_ms) = scheduler.force_run().await;
        tracing::info!(
            enqueued,
            failed,
            elapsed_ms,
            "force_consolidation: background task complete"
        );
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ForceConsolidationResponse {
            accepted: true,
            candidates_found: candidates,
            total_nodes: total,
            message: format!(
                "consolidation started — {} candidates out of {} total nodes. \
                 Monitor via GET /dream/status",
                candidates, total
            ),
        }),
    ))
}

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
        (status = 200, description = "Memory compacted", body = CompactMemoryResponse),
        (status = 404, description = "Memory not found", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn compact_memory(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<CompactMemoryQuery>,
) -> Result<Json<CompactMemoryResponse>, (StatusCode, String)> {
    use crate::memory::{ContextTier, TieredCompactionWorker};

    let pool = match &state.trajectory_pool {
        Some(p) => p.clone(),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "postgres-storage not configured".into(),
            ))
        }
    };

    let target_tier = q.tier.as_ref().and_then(|s| ContextTier::parse(s));

    let worker = TieredCompactionWorker::new(
        (*pool).clone(),
        state.embedding.clone(),
        state.vlm_worker.clone(),
    );
    match worker.compact_memory(id, target_tier).await {
        Ok(new_id) => {
            let tier_str = if new_id == id {
                id.to_string() // no new tier created
            } else {
                target_tier
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "next".to_string())
            };
            Ok(Json(CompactMemoryResponse {
                id: new_id,
                tier: tier_str,
                message: if new_id == id {
                    "memory already at target tier".to_string()
                } else {
                    format!(
                        "compacted to {}",
                        target_tier
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "next tier".to_string())
                    )
                },
            }))
        }
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

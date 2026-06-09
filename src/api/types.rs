use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::webhooks::DedupCache;
use crate::embedding::router::EmbeddingRouter;
use crate::embedding::EmbeddingProvider;
use crate::memory::types::{ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity};
use crate::memory::{DreamMode, FractalNode, GovernancePolicy, InMemoryEventStore};
use crate::storage::{RetrievalProfile, StorageBackend};

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
    pub(crate) fn from_storage(entry: crate::storage::ScoredNode, include_debug: bool) -> Self {
        let debug = entry.debug.clone();
        let dist = entry.distribution_scores.clone();
        let score_debug = include_debug.then(|| score_debug_response(debug.as_ref(), &entry.node));
        Self::from_parts(entry.score, entry.node, debug.as_ref(), score_debug, dist)
    }

    pub(crate) fn from_governed_storage(
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
        Self::from_parts(entry.score, entry.node, debug.as_ref(), score_debug, dist)
            .with_governance(confidence, sensitivity, governance_passed, issues)
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

/// Parse the speaker role from a chunk's first-line prefix.
/// Returns the canonical role name ("user", "assistant") and strips the prefix
/// from the content. Returns None if no role prefix is detected.
pub(crate) fn parse_speaker_role_from_chunk(chunk: &str) -> Option<(&str, &str)> {
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

pub(crate) fn score_debug_response(
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

#[derive(Clone)]
pub struct AppState {
    /// Primary storage backend (trait object for flexibility).
    pub store: Arc<dyn StorageBackend>,
    /// DreamMode needs a StorageBackend.
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

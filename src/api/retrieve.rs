use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::store::set_metadata_text;
use crate::api::auth::AuthContext;
use crate::api::subconscious_qa::{
    is_multi_session_type, is_temporal_question, qa_answer, qa_context_limit, source_context_block,
    source_timestamp,
};
use crate::api::types::{
    clean_for_embedding, score_debug_response, AppState, RetrievalScoreDebug, ScoredNode,
};
use crate::embedding::{embed_document, embed_query};
use crate::memory::types::{ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity};
use crate::memory::{FractalNode, GovernanceValidator};
use crate::storage::{HybridQuery, RetrievalProfile};

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
        && type_filter.is_none_or(|filter| entry.node.memory_type == filter)
}

fn is_internal_meta_artifact(node: &ScoredNode) -> bool {
    let content = node
        .content
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
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
        tracing::warn!(
            removed,
            "retrieve_fractal strict scrub removed internal artifacts"
        );
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

fn apply_intent_scoring_storage(scored: &mut [crate::storage::ScoredNode], intent: QueryIntent) {
    for entry in scored {
        entry.score *=
            intent_metadata_multiplier(intent, entry.node.memory_type, &entry.node.metadata);
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

fn evidence_dedupe_storage(
    mut scored: Vec<crate::storage::ScoredNode>,
) -> Vec<crate::storage::ScoredNode> {
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
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
        let raw_cos =
            crate::memory::fractal_node::cosine_similarity(&entry.node.vector, query_vector)
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
    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let pool_n = top_k.saturating_mul(10).max(top_k).min(candidates.len());
    let pool: Vec<_> = candidates.into_iter().take(pool_n).collect();
    if pool.len() <= top_k {
        return pool;
    }

    // Snapshot: what would pure-score ranking give?
    let score_top_k_ids: std::collections::HashSet<String> =
        pool.iter().take(top_k).map(|c| c.id.to_string()).collect();

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
                mmr_i.partial_cmp(&mmr_j).unwrap_or(Ordering::Equal)
            })
            .expect("cand_idx non-empty");

        cand_idx.retain(|&i| i != best);
        selected.push(pool[best].clone());
    }

    // Diagnostic: MMR vs pure-score overlap
    let mmr_top_k_ids: std::collections::HashSet<String> =
        selected.iter().map(|c| c.id.to_string()).collect();
    let overlap: Vec<_> = score_top_k_ids.intersection(&mmr_top_k_ids).collect();
    let new_in_topk: Vec<_> = mmr_top_k_ids.difference(&score_top_k_ids).collect();

    // Avg age of top-k
    let now = chrono::Utc::now();
    let avg_age_days = selected
        .iter()
        .map(|c| (now - c.node.created_at).num_days() as f32)
        .sum::<f32>()
        / selected.len() as f32;

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

    let pool_refs: Vec<crate::storage::ScoredNode> =
        pool.iter().map(|(s, _, _)| s.clone()).collect();

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
                mmr_i.partial_cmp(&mmr_j).unwrap_or(Ordering::Equal)
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
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let results = evidence_dedupe_storage(results);
    mmr_finalize_storage(results, query_vector, top_k)
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
                embed_query(&*state.embedding, &cleaned)
                    .await
                    .map_err(|e| {
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

    // Clone for turn-level retrieval (postgres-storage feature)
    let query_vector_for_turns = query_vector.clone();

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

    let _query_vector_for_turns = query_vector.clone();
    let results = if req.multi_query {
        // Multi-Query: expand into 2-3 reformulations, retrieve each, RRF-fuse
        let query_text = req.query_text.clone().unwrap_or_default();
        let expansions = crate::retrieval::query_expansion::expand_query(&query_text);
        let mut all_scored: Vec<crate::storage::ScoredNode> = Vec::new();

        for (i, expanded_text) in expansions.iter().enumerate() {
            let expanded_vector = if expanded_text == &query_text {
                query_vector.clone()
            } else {
                let cleaned = clean_for_embedding(expanded_text);
                embed_query(&*state.embedding, &cleaned)
                    .await
                    .map_err(|e| {
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

        let all_nodes = state
            .store
            .list_all()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let by_id: std::collections::HashMap<Uuid, FractalNode> =
            all_nodes.into_iter().map(|n| (n.id, n)).collect();
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<crate::storage::ScoredNode> = Vec::new();
        for (id, score) in fused {
            if seen.insert(id) {
                if let Some(node) = by_id.get(&id).cloned() {
                    merged.push(
                        req.retrieval_profile
                            .score_node(score, node, source_type_weights),
                    );
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
    // Turn results are collected separately so they can REPLACE hybrid_retrieve
    // results when session_id filtering is active.
    let mut turn_results: Vec<crate::storage::ScoredNode> = Vec::new();
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

                match pg
                    .retrieve_turns_internal(
                        &query_vector_for_turns,
                        req.top_k,
                        None,
                        session_uuid_filter,
                    )
                    .await
                {
                    Ok(turn_rows) => {
                        if !turn_rows.is_empty() {
                            tracing::info!(
                                turn_count = turn_rows.len(),
                                has_session_filter = req.session_id.is_some(),
                                "turn-level retrieval from conversation_turns index"
                            );
                            for row in turn_rows {
                                // Build turn-level metadata with session identity
                                let mut metadata: HashMap<String, Value> = row
                                    .metadata
                                    .as_ref()
                                    .and_then(|v| v.as_object())
                                    .map(|o| {
                                        o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                                    })
                                    .unwrap_or_default();

                                // Turn identity markers — required for temporal ranking
                                // and turn-level deduplication.
                                metadata.insert(
                                    "session_id".to_string(),
                                    Value::String(row.session_id.to_string()),
                                );
                                metadata.insert(
                                    "speaker_role".to_string(),
                                    Value::String(row.speaker_role.clone()),
                                );
                                metadata.insert(
                                    "turn_index".to_string(),
                                    Value::Number(serde_json::Number::from(row.turn_index)),
                                );
                                metadata.insert("is_turn".to_string(), Value::Bool(true));
                                if let Some(ref ext_id) = row.external_session_id {
                                    metadata.insert(
                                        "external_session_id".to_string(),
                                        Value::String(ext_id.clone()),
                                    );
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

                                turn_results.push(crate::storage::ScoredNode {
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
                let mut reranker = reranker_arc.lock().map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Reranker lock poisoned: {}", e),
                    )
                })?;
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
                        memory_type_filter: type_filter,
                        user_id: req.user_id.clone(),
                        multi_query: false,
                        recency_boost: req.recency_boost,
                        temporal_weight,
                        fusion_strategy: None,
                        query_type_routing: false,
                        source_type_weights,
                    };
                    let r = state
                        .store
                        .hybrid_retrieve(&fallback_query)
                        .await
                        .map_err(|e| {
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
            final_results =
                apply_temporal_diversity(final_results, req.top_k, None, state.embedding.as_ref())
                    .await;
            tracing::info!(
                pre = pre_len,
                post = final_results.len(),
                "diversity applied"
            );
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
            let avg_score = results.iter().take(top_n).map(|s| s.score).sum::<f32>() / top_n as f32;
            let avg_age = results
                .iter()
                .take(top_n)
                .map(|s| (now - s.node.created_at).num_days() as f32)
                .sum::<f32>()
                / top_n as f32;
            let newest_age = results
                .iter()
                .take(top_n)
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
        let results = if req.retrieval_profile == RetrievalProfile::FullFidelity {
            // FullFidelity: no deduplication, no MMR diversity — pure core scores.
            // Rationale: FullFidelity is the raw retrieval signal. Dedupe and MMR
            // are policy decisions (Reduce-to-Core Phase 2).
            results // pure core — no intent multiplication, no MMR
        } else {
            finalize_retrieval_storage(
                results,
                query_intent,
                &query_vector_for_expand,
                req.top_k,
                allow_meta,
            )
        };
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

            Some((s, validation.passed, validation.issues))
        })
        .collect();

    if req.retrieval_profile != RetrievalProfile::FullFidelity {
        for (entry, _, _) in &mut governed {
            entry.score *= intent_metadata_multiplier(
                query_intent,
                entry.node.memory_type,
                &entry.node.metadata,
            );
        }
    }

    let allow_meta = type_filter == Some(MemoryType::Meta);
    let governed = if req.retrieval_profile == RetrievalProfile::FullFidelity {
        // Pure core — no governance multiplier, no MMR
        governed.sort_by(|(a, _, _), (b, _, _)| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        governed.truncate(req.top_k);
        governed
    } else {
        finalize_governed_retrieval(governed, &query_vector_for_expand, req.top_k, allow_meta)
    };

    let scored: Vec<ScoredNode> = governed
        .into_iter()
        .map(|(s, passed, issues)| {
            ScoredNode::from_governed_storage(s, passed, issues, req.include_debug)
        })
        .filter(|s| allow_meta || s.memory_type != MemoryType::Meta)
        .collect();
    let scored: Vec<ScoredNode> = scored
        .into_iter()
        .filter(|s| type_filter.is_none_or(|t| s.memory_type == t))
        .collect();
    let scored = scrub_response_nodes(scored, allow_meta);

    // Stage 3: Optional Reflect — synthesize coherent summary from top results
    if req.reflect && !scored.is_empty() && type_filter.is_none() {
        let reflector = crate::reflector::Reflector::new();
        if let Some(ref reflector) = reflector {
            let query = req.query_text.as_deref().unwrap_or("");
            if !query.is_empty() {
                let res = {
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
                };
                match res {
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

    let mut scored = scrub_response_nodes(scored, allow_meta);

    // Post-hoc session_id filter: applied at the very end, after all
    // expansion (Stage 1.5), reranking (Stage 2), and diversity (Stage 2.5).
    // Early-filtering before expansion is undone by fractal zoom adding
    // children from other sessions.
    if let Some(ref sid) = req.session_id {
        let before = scored.len();
        scored.retain(|sn| {
            sn.metadata
                .get("session_id")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s == sid.as_str())
        });
        tracing::info!(
            before = before,
            after = scored.len(),
            session_id = %sid,
            "post-hoc session_id filter applied (final)"
        );
    }

    tracing::info!(
        response_len = scored.len(),
        response_meta = scored
            .iter()
            .filter(|n| n.memory_type == MemoryType::Meta)
            .count(),
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

    let groups: Vec<(&[(f32, usize)], &str)> =
        vec![(&early, "early"), (&middle, "middle"), (&late, "late")];

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

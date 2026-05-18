use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::memory::types::{
    ConflictState, ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity,
};
use crate::multimodal::MultimodalData;

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a.len() != b.len() {
        tracing::debug!(
            a_dim = a.len(),
            b_dim = b.len(),
            "cosine_similarity skipped due to dimension mismatch"
        );
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let mag_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Truncate a vector to the first `dim` dimensions (Matryoshka embedding).
/// Returns a copy of the truncated prefix.
pub fn truncate_vector(vector: &[f32], dim: usize) -> Option<Vec<f32>> {
    if vector.len() >= dim {
        Some(vector[..dim].to_vec())
    } else {
        None
    }
}

/// Compute the mean of multiple equal-length vectors (TST bag-of-claims averaging).
/// Returns None if input is empty or vectors have mismatched lengths.
pub fn mean_vector(vectors: &[&[f32]]) -> Option<Vec<f32>> {
    if vectors.is_empty() {
        return None;
    }
    let dim = vectors[0].len();
    if vectors.iter().any(|v| v.len() != dim) {
        return None;
    }
    let mut sum = vec![0.0f32; dim];
    for v in vectors {
        for (i, &x) in v.iter().enumerate() {
            sum[i] += x;
        }
    }
    let n = vectors.len() as f32;
    for x in &mut sum {
        *x /= n;
    }
    Some(sum)
}

/// Matryoshka geometric continuity check: truncated cosine similarity should
/// approximate full-dimensional cosine similarity for Matryoshka embeddings.
/// Returns (full_sim, truncated_sim) for the given truncation dimension.
pub fn matryoshka_continuity(a: &[f32], b: &[f32], trunc_dim: usize) -> Option<(f32, f32)> {
    let full_sim = cosine_similarity(a, b);
    if full_sim == 0.0 && (a.iter().all(|x| *x == 0.0) || b.iter().all(|x| *x == 0.0)) {
        return None;
    }
    let trunc_a = truncate_vector(a, trunc_dim)?;
    let trunc_b = truncate_vector(b, trunc_dim)?;
    let trunc_sim = cosine_similarity(&trunc_a, &trunc_b);
    Some((full_sim, trunc_sim))
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Relation {
    pub target_id: Uuid,
    pub relation_type: String,
    pub strength: f64,
}

/// A fractal memory node.
///
/// # Type System
///
/// - `memory_type`: What kind of memory this is (episodic/semantic/preference/procedural/meta)
/// - `source`: Where the memory originated (conversation/document/import/manual/consolidation)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FractalNode {
    pub id: Uuid,
    /// Memory type — replaces the old NodeType enum.
    #[serde(default = "default_memory_type")]
    pub memory_type: MemoryType,
    /// Where this memory came from.
    #[serde(default)]
    pub source: MemorySource,
    pub vector: Vec<f32>,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    #[schema(value_type = Object)]
    pub metadata: HashMap<String, Value>,
    pub weight: f64,
    pub multimodal: Option<MultimodalData>,
    #[schema(value_type = Vec<Object>)]
    pub children: Vec<FractalNode>,
    pub relations: Vec<Relation>,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,

    // -- Governance fields (Layer 4) --
    /// Confidence score 0.0–1.0. Default is type-specific (see MemoryType::default_confidence).
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Sensitivity level for access control.
    #[serde(default)]
    pub sensitivity: Sensitivity,
    /// If Some, this memory has been superseded by the given ID.
    #[serde(default)]
    pub superseded_by: Option<Uuid>,
    /// Conflict state for semantic/meta memories.
    #[serde(default)]
    pub conflict_state: ConflictState,
    /// Provenance tracking: how we know this.
    #[serde(default)]
    pub provenance: Value,
    /// Importance 1–10, used for scoring.
    #[serde(default = "default_importance")]
    pub importance: i32,
    /// Lifecycle status.
    #[serde(default)]
    pub status: MemoryStatus,
    /// How many times this memory has been accessed.
    #[serde(default)]
    pub access_count: i32,

    // -- Tiered Context fields (L0/L1/L2) --
    /// Context tier for tiered loading (default: Raw/L2 for existing memories).
    #[serde(default = "default_context_tier")]
    pub context_tier: ContextTier,
    /// ID of the parent tier memory (e.g., summary → overview → raw chain).
    /// L2 (Raw) → L1 (Overview) → L0 (Summary)
    #[serde(default)]
    pub parent_tier_id: Option<Uuid>,
    /// IDs of child tier memories (reverse of parent_tier_id).
    /// Enables fractal zooming: L0 → [L1 nodes] → [L2 nodes]
    #[serde(default)]
    pub children_tier_ids: Vec<Uuid>,
    /// L0 summary content (one-sentence).
    #[serde(default)]
    pub summary_content: Option<String>,
    /// L1 overview content (paragraph).
    #[serde(default)]
    pub overview_content: Option<String>,
}

fn default_context_tier() -> ContextTier {
    ContextTier::Raw
}

fn default_memory_type() -> MemoryType {
    MemoryType::Episodic
}

fn default_confidence() -> f64 {
    0.8
}

fn default_importance() -> i32 {
    5
}

impl FractalNode {
    pub const DERIVATION_KEY: &'static str = "derivation";
    pub const RETRIEVAL_VISIBILITY_KEY: &'static str = "retrieval_visibility";
    pub const ROLE_KEY: &'static str = "role";
    pub const TRUST_TIER_KEY: &'static str = "trust_tier";
    pub const TRUST_WEIGHT_KEY: &'static str = "trust_weight";
    pub const INTERNAL_VISIBILITY: &'static str = "internal";
    pub const TRUST_PRIMARY: &'static str = "primary";
    pub const TRUST_REFERENCE: &'static str = "reference";
    pub const TRUST_DERIVED: &'static str = "derived";
    pub const TRUST_VOLATILE: &'static str = "volatile";

    fn metadata_text(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).and_then(Value::as_str)
    }

    fn metadata_number(&self, key: &str) -> Option<f64> {
        self.metadata.get(key).and_then(Value::as_f64)
    }

    fn metadata_matches(&self, key: &str, values: &[&str]) -> bool {
        self.metadata_text(key).is_some_and(|value| {
            values
                .iter()
                .any(|candidate| value.eq_ignore_ascii_case(candidate))
        })
    }

    fn content_prefix_matches(&self, prefixes: &[&str]) -> bool {
        self.content.as_deref().is_some_and(|text| {
            let trimmed = text.trim_start();
            prefixes.iter().any(|prefix| trimmed.starts_with(prefix))
        })
    }

    fn has_explicit_visibility_metadata(&self) -> bool {
        self.metadata.contains_key(Self::ROLE_KEY)
            || self.metadata.contains_key(Self::DERIVATION_KEY)
            || self.metadata.contains_key(Self::RETRIEVAL_VISIBILITY_KEY)
    }

    fn is_legacy_chat_artifact(&self) -> bool {
        self.source == MemorySource::Conversation
            && !self.has_explicit_visibility_metadata()
            && self.content_prefix_matches(&["USER:", "ASSISTANT:", "AI:"])
    }

    fn is_imported_artifact(&self) -> bool {
        self.source == MemorySource::Import
            || self.metadata.contains_key("imported_from")
            || self.metadata.contains_key("import_type")
            || self
                .metadata_text("source")
                .is_some_and(|value| value.starts_with("import:"))
    }

    fn is_primary_import(&self) -> bool {
        self.metadata_text("import_type")
            .is_some_and(|import_type| {
                matches!(
                    import_type,
                    "openclaw_workspace"
                        | "openclaw_session"
                        | "langchain_memory"
                        | "custom_import"
                )
            })
            || self.metadata_text("original_file").is_some_and(|file| {
                matches!(file, "MEMORY.md" | "USER.md" | "IDENTITY.md" | "SOUL.md")
            })
    }

    pub fn set_metadata_text(&mut self, key: &str, value: &str) {
        self.metadata
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    pub fn is_internal_only(&self) -> bool {
        self.memory_type == MemoryType::Meta
            || self.metadata_matches(Self::RETRIEVAL_VISIBILITY_KEY, &[Self::INTERNAL_VISIBILITY])
            || self.metadata_matches(Self::ROLE_KEY, &["assistant", "ai", "system", "mixed"])
            || self.metadata_matches(
                Self::DERIVATION_KEY,
                &[
                    "assistant_output",
                    "retrieval_compose",
                    "chat_query",
                    "agent_transcript",
                ],
            )
            || self.is_legacy_chat_artifact()
    }

    pub fn explicit_trust_weight(&self) -> Option<f32> {
        self.metadata_number(Self::TRUST_WEIGHT_KEY)
            .map(|value| value as f32)
    }

    pub fn trust_tier(&self) -> &'static str {
        // Decision nodes are PRIMARY facts — explicitly extracted claims with reasons.
        // They must rank above ephemeral conversation turns in retrieval.
        if self.memory_type == MemoryType::Decision {
            return Self::TRUST_PRIMARY;
        }
        if self.is_internal_only()
            || self.source == MemorySource::Consolidation
            || self.metadata_matches(Self::DERIVATION_KEY, &["system_summary"])
        {
            return Self::TRUST_DERIVED;
        }
        if let Some(value) = self.metadata_text(Self::TRUST_TIER_KEY) {
            if value.eq_ignore_ascii_case(Self::TRUST_PRIMARY) {
                return Self::TRUST_PRIMARY;
            }
            if value.eq_ignore_ascii_case(Self::TRUST_REFERENCE) {
                return Self::TRUST_REFERENCE;
            }
            if value.eq_ignore_ascii_case(Self::TRUST_DERIVED) {
                return Self::TRUST_DERIVED;
            }
            if value.eq_ignore_ascii_case(Self::TRUST_VOLATILE) {
                return Self::TRUST_VOLATILE;
            }
        }
        if self.is_imported_artifact() {
            if self.is_primary_import() {
                return Self::TRUST_PRIMARY;
            }
            return Self::TRUST_REFERENCE;
        }
        if self.source == MemorySource::Document || self.source == MemorySource::Manual {
            return Self::TRUST_REFERENCE;
        }
        if self.metadata_matches(Self::DERIVATION_KEY, &["user_input"])
            || self.metadata_matches(Self::ROLE_KEY, &["user"])
            || self.source == MemorySource::Conversation
        {
            return Self::TRUST_PRIMARY;
        }
        Self::TRUST_REFERENCE
    }

    /// Session-Knoten: speichert den vollen Text + Embedding.
    pub fn new_session(
        content: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Episodic,
            source: MemorySource::Conversation,
            vector,
            content: Some(content),
            original_pointer: None,
            metadata,
            weight: 1.0,
            multimodal: None,
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
            confidence: MemoryType::Episodic.default_confidence(),
            sensitivity: Sensitivity::Normal,
            superseded_by: None,
            conflict_state: ConflictState::None,
            provenance: serde_json::json!({"method": "session"}),
            importance: MemoryType::Episodic.default_importance(),
            status: MemoryStatus::Active,
            access_count: 0,
            context_tier: ContextTier::Raw,
            parent_tier_id: None,
            children_tier_ids: Vec::new(),
            summary_content: None,
            overview_content: None,
        }
    }

    /// Externer Knoten: speichert NUR den Pointer + Embedding, nie Rohdaten.
    pub fn new_external(
        pointer: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
        created_at: Option<DateTime<Utc>>,
    ) -> Self {
        let now = created_at.unwrap_or_else(Utc::now);
        Self {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            source: MemorySource::Import,
            vector,
            content: None,
            original_pointer: Some(pointer),
            metadata,
            weight: 1.0,
            multimodal: None,
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
            confidence: MemoryType::Semantic.default_confidence(),
            sensitivity: Sensitivity::Normal,
            superseded_by: None,
            conflict_state: ConflictState::None,
            provenance: serde_json::json!({"method": "external"}),
            importance: MemoryType::Semantic.default_importance(),
            status: MemoryStatus::Active,
            access_count: 0,
            context_tier: ContextTier::Raw,
            parent_tier_id: None,
            children_tier_ids: Vec::new(),
            summary_content: None,
            overview_content: None,
        }
    }

    /// Externer Knoten mit multimodalen Daten (Image/Audio/Sensor).
    pub fn new_external_multimodal(
        pointer: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
        multimodal: MultimodalData,
        created_at: Option<DateTime<Utc>>,
    ) -> Self {
        let now = created_at.unwrap_or_else(Utc::now);
        Self {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            source: MemorySource::Import,
            vector,
            content: None,
            original_pointer: Some(pointer),
            metadata,
            weight: 1.0,
            multimodal: Some(multimodal),
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
            confidence: MemoryType::Semantic.default_confidence(),
            sensitivity: Sensitivity::Normal,
            superseded_by: None,
            conflict_state: ConflictState::None,
            provenance: serde_json::json!({"method": "external_multimodal"}),
            importance: MemoryType::Semantic.default_importance(),
            status: MemoryStatus::Active,
            access_count: 0,
            context_tier: ContextTier::Raw,
            parent_tier_id: None,
            children_tier_ids: Vec::new(),
            summary_content: None,
            overview_content: None,
        }
    }

    /// Create a node with explicit memory type (new API).
    pub fn new_typed(
        content: Option<String>,
        pointer: Option<String>,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
        memory_type: MemoryType,
        source: MemorySource,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            memory_type,
            source,
            vector,
            content,
            original_pointer: pointer,
            metadata,
            weight: 1.0,
            multimodal: None,
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
            confidence: memory_type.default_confidence(),
            sensitivity: Sensitivity::Normal,
            superseded_by: None,
            conflict_state: ConflictState::None,
            provenance: serde_json::json!({}),
            importance: memory_type.default_importance(),
            status: MemoryStatus::Active,
            access_count: 0,
            context_tier: ContextTier::Raw,
            parent_tier_id: None,
            children_tier_ids: Vec::new(),
            summary_content: None,
            overview_content: None,
        }
    }

    pub fn find_best_child(&self, query_vector: &[f32]) -> Option<&FractalNode> {
        self.children.iter().max_by(|a, b| {
            let sim_a = cosine_similarity(&a.vector, query_vector);
            let sim_b = cosine_similarity(&b.vector, query_vector);
            sim_a.partial_cmp(&sim_b).unwrap_or(Ordering::Equal)
        })
    }

    /// Default pruning threshold for hierarchical zoom.
    /// Only children are explored if parent's similarity >= this threshold.
    pub const ZOOM_PRUNING_THRESHOLD: f32 = 0.7;

    /// Rekursives Zoomen mit Hierarchical Pruning.
    ///
    /// Sammelt (similarity, node) Paare entlang des besten Pfads.
    /// Nur wenn der Parents-Score >= `pruning_threshold` werden Kinder durchsucht.
    ///
    /// **Pruning-Logik:**
    /// - `sim >= pruning_threshold` → Kinder werden rekursiv durchsucht
    /// - `sim < pruning_threshold` → Ast wird abgeschnitten (PRUNED)
    ///
    /// Dies reduziert die Anzahl der Vektor-Distanzberechnungen massiv
    /// bei tiefen Graphen und erhöht die Retrieval-Geschwindigkeit.
    pub fn zoom_retrieve<'a>(
        &'a self,
        query_vector: &[f32],
        max_depth: usize,
        pruning_threshold: f32,
    ) -> Vec<(f32, &'a FractalNode)> {
        let sim = cosine_similarity(&self.vector, query_vector);
        let mut results = vec![(sim, self)];

        if max_depth > 0 && sim >= pruning_threshold {
            if let Some(best) = self.find_best_child(query_vector) {
                results.extend(best.zoom_retrieve(query_vector, max_depth - 1, pruning_threshold));
            }
        }
        // Wenn sim < pruning_threshold: Kinder werden NICHT durchsucht → PRUNED

        results
    }

    /// Convert to GovernanceCandidate for Stage 2 validation.
    pub fn to_governance_candidate(&self) -> crate::memory::GovernanceCandidate {
        crate::memory::GovernanceCandidate {
            id: self.id,
            memory_type: self.memory_type,
            confidence: self.confidence,
            sensitivity: self.sensitivity,
            status: self.status,
            superseded_by: self.superseded_by,
            conflict_state: self.conflict_state,
            created_at: self.created_at,
            importance: self.importance,
            access_count: self.access_count,
            last_accessed: Some(self.last_accessed),
        }
    }
}

/// Backward-compatible alias — NodeType is deprecated in favor of MemoryType + MemorySource.
#[allow(deprecated)]
#[deprecated(since = "0.2.0", note = "Use MemoryType + MemorySource instead")]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    #[default]
    #[serde(alias = "session", alias = "Session")]
    Session,
    #[serde(alias = "external", alias = "External")]
    External,
}

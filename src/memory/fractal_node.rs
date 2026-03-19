use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::memory::types::{ConflictState, MemorySource, MemoryStatus, MemoryType, Sensitivity};
use crate::multimodal::MultimodalData;

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let mag_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
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
    /// Session-Knoten: speichert den vollen Text + Embedding.
    pub fn new_session(content: String, vector: Vec<f32>, metadata: HashMap<String, Value>) -> Self {
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
        }
    }

    /// Externer Knoten: speichert NUR den Pointer + Embedding, nie Rohdaten.
    pub fn new_external(
        pointer: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
    ) -> Self {
        let now = Utc::now();
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
        }
    }

    /// Externer Knoten mit multimodalen Daten (Image/Audio/Sensor).
    pub fn new_external_multimodal(
        pointer: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
        multimodal: MultimodalData,
    ) -> Self {
        let now = Utc::now();
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
        }
    }

    pub fn find_best_child(&self, query_vector: &[f32]) -> Option<&FractalNode> {
        self.children.iter().max_by(|a, b| {
            let sim_a = cosine_similarity(&a.vector, query_vector);
            let sim_b = cosine_similarity(&b.vector, query_vector);
            sim_a.partial_cmp(&sim_b).unwrap_or(Ordering::Equal)
        })
    }

    /// Rekursives Zoomen: sammelt (similarity, node) Paare entlang des besten Pfads.
    pub fn zoom_retrieve(
        &self,
        query_vector: &[f32],
        max_depth: usize,
    ) -> Vec<(f32, FractalNode)> {
        let sim = cosine_similarity(&self.vector, query_vector);
        let mut results = vec![(sim, self.clone())];
        if max_depth > 0 {
            if let Some(best) = self.find_best_child(query_vector) {
                results.extend(best.zoom_retrieve(query_vector, max_depth - 1));
            }
        }
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
#[deprecated(since = "0.2.0", note = "Use MemoryType + MemorySource instead")]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    #[default]
    #[serde(alias = "session", alias = "Session")]
    Session,
    #[serde(alias = "external", alias = "External")]
    External,
}

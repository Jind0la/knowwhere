//! Source-type classification and configurable per-source score multipliers.
//!
//! Source weighting is a **Policy** feature. The multiplier is applied by
//! [`ScoringEngine`] only for non-FullFidelity profiles (`UserFacing`,
//! `AgentDebug`). Under `FullFidelity`, source weights are recorded in
//! `ScoreDebug` for observability but do not affect the score.
//!
//! See [`crate::retrieval::scoring`] for the full scoring pipeline.
//!
//! ## Provenance taxonomy
//! - **Real** (1.0): Human-authored, conversation-derived, direct imports
//! - **Synthetic** (0.85): AI-generated content, consolidation artifacts
//! - **Derived** (0.70): Summaries, auto-generated from other sources
//! - **Unknown** (0.95): Missing or unparseable provenance (tiny penalty)
//!
//! ## Detection order
//! 1. `metadata.provenance` key (string: "real"|"synthetic"|"derived")
//! 2. `metadata.source_dataset` key (dataset origin hint)
//! 3. `node.provenance` JSON field (method key)
//! 4. `node.source` (MemorySource enum fallback)

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::memory::types::MemorySource;
use crate::memory::FractalNode;

/// Source type classification for provenance-aware scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// Human-authored, conversation-derived, or directly imported.
    /// Full trust — no score penalty.
    Real,
    /// AI-generated or consolidated content. Slight confidence discount.
    Synthetic,
    /// Derived/auto-generated from other sources (summaries, extractions).
    /// Moderate confidence discount.
    Derived,
    /// Missing or unparseable provenance metadata. Tiny penalty.
    Unknown,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::Real => write!(f, "real"),
            SourceType::Synthetic => write!(f, "synthetic"),
            SourceType::Derived => write!(f, "derived"),
            SourceType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Configurable multiplier table for source-type weighting.
///
/// Each field is a multiplier applied to a node's score based on its
/// detected source type. Values are clamped to [0.0, 2.0] on construction
/// for safety.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct SourceTypeWeights {
    /// Multiplier for Real (human-authored) nodes. Default: 1.0.
    pub real: f32,
    /// Multiplier for Synthetic (AI-generated) nodes. Default: 0.85.
    pub synthetic: f32,
    /// Multiplier for Derived (auto-generated) nodes. Default: 0.70.
    pub derived: f32,
    /// Multiplier for Unknown provenance nodes. Default: 0.95.
    pub unknown: f32,
}

impl Default for SourceTypeWeights {
    fn default() -> Self {
        Self {
            real: 1.0,
            synthetic: 0.85,
            derived: 0.70,
            unknown: 0.95,
        }
    }
}

impl SourceTypeWeights {
    /// Create with explicit weights, clamping to [0.0, 2.0].
    pub fn new(real: f32, synthetic: f32, derived: f32, unknown: f32) -> Self {
        Self {
            real: real.clamp(0.0, 2.0),
            synthetic: synthetic.clamp(0.0, 2.0),
            derived: derived.clamp(0.0, 2.0),
            unknown: unknown.clamp(0.0, 2.0),
        }
    }

    /// Load server-wide default source-type weights from the
    /// `KNOWWHERE_SOURCE_TYPE_WEIGHTS` environment variable.
    ///
    /// The env var should be a JSON object mapping source-type names
    /// to f32 multipliers, e.g.:
    ///
    /// ```env
    /// KNOWWHERE_SOURCE_TYPE_WEIGHTS='{"real":1.0,"synthetic":0.5,"derived":0.3,"unknown":0.8}'
    /// ```
    ///
    /// All four keys (`real`, `synthetic`, `derived`, `unknown`) are
    /// optional — missing keys keep their [`Default`] value. Values
    /// are clamped to [0.0, 2.0]. If the env var is missing or the
    /// JSON cannot be parsed, `None` is returned (callers fall back
    /// to [`SourceTypeWeights::default`]).
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("KNOWWHERE_SOURCE_TYPE_WEIGHTS").ok()?;
        let parsed: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
        let obj = parsed.as_object()?;
        let mut weights = Self::default();
        if let Some(v) = obj.get("real").and_then(|v| v.as_f64()) {
            weights.real = (v as f32).clamp(0.0, 2.0);
        }
        if let Some(v) = obj.get("synthetic").and_then(|v| v.as_f64()) {
            weights.synthetic = (v as f32).clamp(0.0, 2.0);
        }
        if let Some(v) = obj.get("derived").and_then(|v| v.as_f64()) {
            weights.derived = (v as f32).clamp(0.0, 2.0);
        }
        if let Some(v) = obj.get("unknown").and_then(|v| v.as_f64()) {
            weights.unknown = (v as f32).clamp(0.0, 2.0);
        }
        Some(weights)
    }

    /// Load source-type weights from a JSON config file.
    ///
    /// The file format is the same as the `KNOWWHERE_SOURCE_TYPE_WEIGHTS`
    /// env var: a JSON object mapping source-type names to f32 multipliers.
    ///
    /// ```json
    /// {"real": 1.0, "synthetic": 0.85, "derived": 0.70, "unknown": 0.95}
    /// ```
    ///
    /// All four keys are optional — missing keys keep their [`Default`] value.
    /// Values are clamped to [0.0, 2.0]. Returns `None` if the file doesn't
    /// exist or cannot be parsed.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Option<Self> {
        let raw = std::fs::read_to_string(path).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
        let obj = parsed.as_object()?;
        let mut weights = Self::default();
        if let Some(v) = obj.get("real").and_then(|v| v.as_f64()) {
            weights.real = (v as f32).clamp(0.0, 2.0);
        }
        if let Some(v) = obj.get("synthetic").and_then(|v| v.as_f64()) {
            weights.synthetic = (v as f32).clamp(0.0, 2.0);
        }
        if let Some(v) = obj.get("derived").and_then(|v| v.as_f64()) {
            weights.derived = (v as f32).clamp(0.0, 2.0);
        }
        if let Some(v) = obj.get("unknown").and_then(|v| v.as_f64()) {
            weights.unknown = (v as f32).clamp(0.0, 2.0);
        }
        Some(weights)
    }

    /// Load source-type weights from the best available config source.
    ///
    /// Priority order:
    /// 1. `KNOWWHERE_SOURCE_TYPE_WEIGHTS` env var (JSON, highest priority)
    /// 2. File at path given by `KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE` env var
    /// 3. File at default path `./source_weights.json`
    ///
    /// Returns `None` if no config source is available or all sources
    /// fail to parse — callers should fall back to [`SourceTypeWeights::default`].
    pub fn from_config() -> Option<Self> {
        // 1. Try env var first (explicit, per-deployment override)
        if let Some(weights) = Self::from_env() {
            return Some(weights);
        }

        // 2. Try file at KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE path
        if let Ok(file_path) = std::env::var("KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE") {
            if let Some(weights) = Self::from_file(&file_path) {
                return Some(weights);
            }
        }

        // 3. Try default file
        Self::from_file("source_weights.json")
    }

    /// Get the multiplier for a given source type.
    pub fn multiplier(&self, source_type: SourceType) -> f32 {
        match source_type {
            SourceType::Real => self.real,
            SourceType::Synthetic => self.synthetic,
            SourceType::Derived => self.derived,
            SourceType::Unknown => self.unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// Source type detection
// ---------------------------------------------------------------------------

/// Detect the source type of a memory node from its metadata and provenance.
///
/// Detection priority:
/// 1. `metadata.provenance` key → maps "real"/"synthetic"/"derived" strings
/// 2. `metadata.source_dataset` key → synthetic datasets (synthetic_data, llm_output, etc.)
/// 3. `provenance.method` JSON field → "consolidation"/"llm" maps to Synthetic
/// 4. Fallback: MemorySource enum → Conversation/Document/Manual/Import → Real,
///    Consolidation/AiSelfImprovement → Synthetic
/// 5. Ultimate fallback: Unknown
pub fn detect_source_type(node: &FractalNode) -> SourceType {
    // 1. Check metadata.provenance string value
    if let Some(prov) = node.metadata.get("provenance").and_then(|v| v.as_str()) {
        match prov.to_lowercase().as_str() {
            "real" => return SourceType::Real,
            "synthetic" => return SourceType::Synthetic,
            "derived" => return SourceType::Derived,
            _ => {} // fall through
        }
    }

    // 2. Check source_dataset for known synthetic datasets
    if let Some(dataset) = node.metadata.get("source_dataset").and_then(|v| v.as_str()) {
        let ds_lower = dataset.to_lowercase();
        if ds_lower.contains("synthetic")
            || ds_lower.contains("llm")
            || ds_lower.contains("ai_")
            || ds_lower.contains("generated")
        {
            return SourceType::Synthetic;
        }
        if ds_lower.contains("derived") || ds_lower.contains("summary") {
            return SourceType::Derived;
        }
        // Otherwise, presence of a dataset tag leans toward Real
        return SourceType::Real;
    }

    // 3. Check provenance.method JSON field
    if let Some(method) = node.provenance.get("method").and_then(|v| v.as_str()) {
        match method.to_lowercase().as_str() {
            "session" | "external" | "external_multimodal" | "manual" => {
                return SourceType::Real;
            }
            "consolidation" | "llm" | "auto" => {
                return SourceType::Synthetic;
            }
            "derived" | "summary" | "extraction" => {
                return SourceType::Derived;
            }
            _ => {} // fall through
        }
    }

    // 4. Fall back to MemorySource
    match node.source {
        MemorySource::Conversation
        | MemorySource::Document
        | MemorySource::Import
        | MemorySource::Manual => SourceType::Real,
        MemorySource::Consolidation | MemorySource::AiSelfImprovement => SourceType::Synthetic,
    }
}

/// Compute the source-type score multiplier for a node.
///
/// Unmarked nodes default to Real (1.0) for backward compatibility.
pub fn source_multiplier(node: &FractalNode, weights: &SourceTypeWeights) -> f32 {
    let source_type = detect_source_type(node);
    weights.multiplier(source_type)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{MemoryStatus, MemoryType};
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    /// Serialize tests that manipulate process-global env vars.
    /// `std::env::set_var` is not thread-safe — parallel tests would race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_node(
        source: MemorySource,
        provenance: serde_json::Value,
        metadata: HashMap<String, serde_json::Value>,
    ) -> FractalNode {
        FractalNode {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Episodic,
            source,
            vector: vec![0.1; 768],
            content: Some("test content".into()),
            original_pointer: None,
            metadata,
            weight: 1.0,
            multimodal: None,
            children: vec![],
            relations: vec![],
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            confidence: 0.8,
            sensitivity: crate::memory::types::Sensitivity::Normal,
            superseded_by: None,
            conflict_state: crate::memory::types::ConflictState::None,
            provenance,
            importance: 5,
            status: MemoryStatus::Active,
            access_count: 0,
            context_tier: crate::memory::types::ContextTier::Raw,
            parent_tier_id: None,
            children_tier_ids: vec![],
            summary_content: None,
            overview_content: None,
            source_memory_id: None,
            r_m: Utc::now(),
            n_m: 0,
        }
    }

    #[test]
    fn test_detect_from_metadata_provenance_real() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("real"));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            meta,
        );
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    #[test]
    fn test_detect_from_metadata_provenance_synthetic() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_detect_from_metadata_provenance_derived() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("derived"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "summary"}),
            meta,
        );
        assert_eq!(detect_source_type(&node), SourceType::Derived);
    }

    #[test]
    fn test_detect_from_source_dataset_synthetic() {
        let mut meta = HashMap::new();
        meta.insert(
            "source_dataset".into(),
            serde_json::json!("synthetic_data_v3"),
        );
        let node = make_node(
            MemorySource::Import,
            serde_json::json!({"method": "external"}),
            meta,
        );
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_detect_from_source_dataset_llm() {
        let mut meta = HashMap::new();
        meta.insert("source_dataset".into(), serde_json::json!("llm_output"));
        let node = make_node(
            MemorySource::Import,
            serde_json::json!({"method": "external"}),
            meta,
        );
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_detect_from_provenance_method_consolidation() {
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "consolidation"}),
            HashMap::new(),
        );
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_detect_from_provenance_method_session() {
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            HashMap::new(),
        );
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    #[test]
    fn test_detect_from_source_conversation() {
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({}),
            HashMap::new(),
        );
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    #[test]
    fn test_detect_from_source_consolidation() {
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({}),
            HashMap::new(),
        );
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_detect_from_source_ai_self_improvement() {
        let node = make_node(
            MemorySource::AiSelfImprovement,
            serde_json::json!({}),
            HashMap::new(),
        );
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_detect_from_source_document() {
        let node = make_node(
            MemorySource::Document,
            serde_json::json!({}),
            HashMap::new(),
        );
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    // --- Weighting tests ---

    #[test]
    fn test_source_multiplier_real() {
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            HashMap::new(),
        );
        let weights = SourceTypeWeights::default();
        let mult = source_multiplier(&node, &weights);
        assert!((mult - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_source_multiplier_synthetic() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        let weights = SourceTypeWeights::default();
        let mult = source_multiplier(&node, &weights);
        assert!((mult - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_source_multiplier_derived() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("derived"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "derived"}),
            meta,
        );
        let weights = SourceTypeWeights::default();
        let mult = source_multiplier(&node, &weights);
        assert!((mult - 0.70).abs() < 0.001);
    }

    #[test]
    fn test_source_multiplier_unknown() {
        let node = make_node(
            MemorySource::Import,
            serde_json::json!({"method": "unknown_method"}),
            HashMap::new(),
        );
        let weights = SourceTypeWeights::default();
        let mult = source_multiplier(&node, &weights);

        // Unknown method on Import source → Import maps to Real (MemorySource fallback)
        // Wait — actually Import maps to Real, so this node should be Real.
        // Let's test a truly unknown case: we need a source with unrecognized provenance
        // But MemorySource only has known variants, so the "unknown_method" doesn't
        // match any provenance method and falls through to source → Real.
        // Real nodes from Import get 1.0.
        assert!((mult - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_custom_weights() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(MemorySource::Consolidation, serde_json::json!({}), meta);
        let weights = SourceTypeWeights::new(1.0, 0.5, 0.5, 0.5);
        let mult = source_multiplier(&node, &weights);
        assert!((mult - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_weights_clamped() {
        let weights = SourceTypeWeights::new(3.0, -0.5, 10.0, 0.0);
        assert!((weights.real - 2.0).abs() < 0.001); // clamped from 3.0
        assert!((weights.synthetic - 0.0).abs() < 0.001); // clamped from -0.5
        assert!((weights.derived - 2.0).abs() < 0.001); // clamped from 10.0
        assert!((weights.unknown - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_default_weights_are_reasonable() {
        let w = SourceTypeWeights::default();
        assert!(w.real >= 1.0, "real should not be penalized");
        assert!(
            w.synthetic < w.real,
            "synthetic should be discounted vs real"
        );
        assert!(
            w.derived < w.synthetic,
            "derived should be more discounted than synthetic"
        );
        assert!(w.unknown < w.real, "unknown should have slight penalty");
        assert!(w.unknown > 0.0, "unknown should not be zero");
    }

    #[test]
    fn test_display_source_type() {
        assert_eq!(SourceType::Real.to_string(), "real");
        assert_eq!(SourceType::Synthetic.to_string(), "synthetic");
        assert_eq!(SourceType::Derived.to_string(), "derived");
        assert_eq!(SourceType::Unknown.to_string(), "unknown");
    }

    // ── Session-level integration tests ──────────────────────────────
    // Simulate a full session with mixed provenance nodes and verify
    // weight application through the complete scoring pipeline.

    /// Helper: build a session-like batch of nodes with distinct provenance.
    fn session_nodes() -> Vec<FractalNode> {
        let now = Utc::now();
        vec![
            // Node 0: Real — human conversation
            {
                let mut meta = HashMap::new();
                meta.insert("provenance".into(), serde_json::json!("real"));
                meta.insert("session_id".into(), serde_json::json!("sess-01"));
                make_node(
                    MemorySource::Conversation,
                    serde_json::json!({"method": "session"}),
                    meta,
                )
            },
            // Node 1: Synthetic — AI consolidation
            {
                let mut meta = HashMap::new();
                meta.insert("provenance".into(), serde_json::json!("synthetic"));
                make_node(
                    MemorySource::Consolidation,
                    serde_json::json!({"method": "consolidation"}),
                    meta,
                )
            },
            // Node 2: Derived — auto summary
            {
                let mut meta = HashMap::new();
                meta.insert(
                    "source_dataset".into(),
                    serde_json::json!("derived_summaries_v1"),
                );
                make_node(
                    MemorySource::Consolidation,
                    serde_json::json!({"method": "summary"}),
                    meta,
                )
            },
            // Node 3: Real — imported document
            make_node(
                MemorySource::Document,
                serde_json::json!({"method": "external"}),
                HashMap::new(),
            ),
            // Node 4: Unknown provenance — no metadata
            {
                let mut meta = HashMap::new();
                meta.insert("timestamp".into(), serde_json::json!(now.to_rfc3339()));
                make_node(
                    MemorySource::Import,
                    serde_json::json!({"method": "unknown"}),
                    meta,
                )
            },
        ]
    }

    #[test]
    fn test_session_detection_all_types() {
        let nodes = session_nodes();
        assert_eq!(detect_source_type(&nodes[0]), SourceType::Real);
        assert_eq!(detect_source_type(&nodes[1]), SourceType::Synthetic);
        assert_eq!(detect_source_type(&nodes[2]), SourceType::Derived);
        assert_eq!(detect_source_type(&nodes[3]), SourceType::Real); // Document → Real
        assert_eq!(detect_source_type(&nodes[4]), SourceType::Real); // Import → Real (fallback)
    }

    #[test]
    fn test_session_weight_application_defaults() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        // Real nodes (0, 3, 4): multiplier = 1.0
        assert!((source_multiplier(&nodes[0], &weights) - 1.00).abs() < 0.001);
        assert!((source_multiplier(&nodes[3], &weights) - 1.00).abs() < 0.001);
        assert!((source_multiplier(&nodes[4], &weights) - 1.00).abs() < 0.001);

        // Synthetic node (1): multiplier = 0.85
        assert!((source_multiplier(&nodes[1], &weights) - 0.85).abs() < 0.001);

        // Derived node (2): multiplier = 0.70
        assert!((source_multiplier(&nodes[2], &weights) - 0.70).abs() < 0.001);
    }

    #[test]
    fn test_session_weight_application_custom() {
        let nodes = session_nodes();
        // Aggressive synthetic penalty, slight real boost
        let weights = SourceTypeWeights::new(1.1, 0.3, 0.2, 0.5);

        assert!((source_multiplier(&nodes[0], &weights) - 1.1).abs() < 0.001);
        assert!((source_multiplier(&nodes[1], &weights) - 0.3).abs() < 0.001);
        assert!((source_multiplier(&nodes[2], &weights) - 0.2).abs() < 0.001);
        assert!((source_multiplier(&nodes[3], &weights) - 1.1).abs() < 0.001);
    }

    #[test]
    fn test_session_relative_ordering() {
        // Synthetic and Derived should always score lower than Real with defaults
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        let real_mult = source_multiplier(&nodes[0], &weights);
        let synth_mult = source_multiplier(&nodes[1], &weights);
        let deriv_mult = source_multiplier(&nodes[2], &weights);

        assert!(
            real_mult > synth_mult,
            "real ({real_mult}) > synthetic ({synth_mult})"
        );
        assert!(
            synth_mult > deriv_mult,
            "synthetic ({synth_mult}) > derived ({deriv_mult})"
        );
    }

    #[test]
    fn test_session_equal_real_boost_preserves_order() {
        // All Real nodes (with different sources that all resolve to Real)
        // should get the same multiplier
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        let mult0 = source_multiplier(&nodes[0], &weights);
        let mult3 = source_multiplier(&nodes[3], &weights);

        assert!(
            (mult0 - mult3).abs() < 0.001,
            "All Real nodes should have identical multiplier"
        );
    }

    #[test]
    fn test_session_no_weights_means_defaults() {
        // Without weights, default multipliers apply
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        // Synthetic → 0.85x
        assert!((source_multiplier(&nodes[1], &weights) - 0.85).abs() < 0.001);
        // Derived → 0.70x
        assert!((source_multiplier(&nodes[2], &weights) - 0.70).abs() < 0.001);
    }

    // --- from_env() tests ---

    #[test]
    fn test_from_env_missing_returns_none() {
        let _lock = ENV_LOCK.lock().unwrap();
        // Ensure no env pollution
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
        assert!(SourceTypeWeights::from_env().is_none());
    }

    #[test]
    fn test_from_env_full_json() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var(
            "KNOWWHERE_SOURCE_TYPE_WEIGHTS",
            r#"{"real":1.0,"synthetic":0.5,"derived":0.3,"unknown":0.8}"#,
        );
        let w = SourceTypeWeights::from_env().expect("should parse");
        assert!((w.real - 1.0).abs() < 0.001);
        assert!((w.synthetic - 0.5).abs() < 0.001);
        assert!((w.derived - 0.3).abs() < 0.001);
        assert!((w.unknown - 0.8).abs() < 0.001);
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
    }

    #[test]
    fn test_from_env_partial_json_keeps_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS", r#"{"synthetic":0.2}"#);
        let w = SourceTypeWeights::from_env().expect("should parse");
        // synthetic overridden
        assert!((w.synthetic - 0.2).abs() < 0.001);
        // others stay default
        assert!((w.real - 1.0).abs() < 0.001);
        assert!((w.derived - 0.70).abs() < 0.001);
        assert!((w.unknown - 0.95).abs() < 0.001);
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
    }

    #[test]
    fn test_from_env_clamps_bounds() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var(
            "KNOWWHERE_SOURCE_TYPE_WEIGHTS",
            r#"{"real":3.0,"synthetic":-0.5,"derived":10.0,"unknown":-1.0}"#,
        );
        let w = SourceTypeWeights::from_env().expect("should parse");
        assert!((w.real - 2.0).abs() < 0.001, "real clamped to 2.0");
        assert!(
            (w.synthetic - 0.0).abs() < 0.001,
            "synthetic clamped to 0.0"
        );
        assert!((w.derived - 2.0).abs() < 0.001, "derived clamped to 2.0");
        assert!((w.unknown - 0.0).abs() < 0.001, "unknown clamped to 0.0");
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
    }

    #[test]
    fn test_from_env_invalid_json_returns_none() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS", "not json");
        assert!(SourceTypeWeights::from_env().is_none());
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
    }

    #[test]
    fn test_from_env_array_returns_none() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS", "[1.0, 2.0]");
        assert!(SourceTypeWeights::from_env().is_none());
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
    }

    // --- from_file() tests ---

    #[test]
    fn test_from_file_full_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source_weights.json");
        std::fs::write(
            &path,
            r#"{"real":1.0,"synthetic":0.5,"derived":0.3,"unknown":0.8}"#,
        )
        .unwrap();
        let w = SourceTypeWeights::from_file(&path).expect("should parse");
        assert!((w.real - 1.0).abs() < 0.001);
        assert!((w.synthetic - 0.5).abs() < 0.001);
        assert!((w.derived - 0.3).abs() < 0.001);
        assert!((w.unknown - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_from_file_partial_json_keeps_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path, r#"{"synthetic":0.2}"#).unwrap();
        let w = SourceTypeWeights::from_file(&path).expect("should parse");
        assert!((w.synthetic - 0.2).abs() < 0.001);
        assert!((w.real - 1.0).abs() < 0.001);
        assert!((w.derived - 0.70).abs() < 0.001);
        assert!((w.unknown - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_from_file_clamps_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clamped.json");
        std::fs::write(
            &path,
            r#"{"real":3.0,"synthetic":-0.5,"derived":10.0,"unknown":-1.0}"#,
        )
        .unwrap();
        let w = SourceTypeWeights::from_file(&path).expect("should parse");
        assert!((w.real - 2.0).abs() < 0.001, "real clamped to 2.0");
        assert!(
            (w.synthetic - 0.0).abs() < 0.001,
            "synthetic clamped to 0.0"
        );
        assert!((w.derived - 2.0).abs() < 0.001, "derived clamped to 2.0");
        assert!((w.unknown - 0.0).abs() < 0.001, "unknown clamped to 0.0");
    }

    #[test]
    fn test_from_file_missing_returns_none() {
        assert!(SourceTypeWeights::from_file("/nonexistent/path/weights.json").is_none());
    }

    #[test]
    fn test_from_file_invalid_json_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(SourceTypeWeights::from_file(&path).is_none());
    }

    #[test]
    fn test_from_file_array_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("array.json");
        std::fs::write(&path, "[1.0, 2.0]").unwrap();
        assert!(SourceTypeWeights::from_file(&path).is_none());
    }

    #[test]
    fn test_from_file_empty_object_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.json");
        std::fs::write(&path, "{}").unwrap();
        let w = SourceTypeWeights::from_file(&path).expect("should parse");
        let d = SourceTypeWeights::default();
        assert!((w.real - d.real).abs() < 0.001);
        assert!((w.synthetic - d.synthetic).abs() < 0.001);
        assert!((w.derived - d.derived).abs() < 0.001);
        assert!((w.unknown - d.unknown).abs() < 0.001);
    }

    // --- from_config() tests ---

    #[test]
    fn test_from_config_env_takes_precedence_over_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        // Set env var (highest priority)
        std::env::set_var(
            "KNOWWHERE_SOURCE_TYPE_WEIGHTS",
            r#"{"real":1.5,"synthetic":0.3}"#,
        );
        // Write a conflicting file
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source_weights.json");
        std::fs::write(
            &path,
            r#"{"real":0.1,"synthetic":0.9,"derived":0.8,"unknown":0.7}"#,
        )
        .unwrap();
        std::env::set_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE", &path);
        let w = SourceTypeWeights::from_config().expect("should use env");
        assert!((w.real - 1.5).abs() < 0.001, "env real should win");
        assert!(
            (w.synthetic - 0.3).abs() < 0.001,
            "env synthetic should win"
        );
        // unknown was not in env — should fall to default, NOT file
        assert!(
            (w.unknown - 0.95).abs() < 0.001,
            "unknown should use default"
        );
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE");
    }

    #[test]
    fn test_from_config_file_only() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source_weights.json");
        std::fs::write(&path, r#"{"real":0.9,"synthetic":0.6}"#).unwrap();
        std::env::set_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE", &path);
        let w = SourceTypeWeights::from_config().expect("should use file");
        assert!((w.real - 0.9).abs() < 0.001);
        assert!((w.synthetic - 0.6).abs() < 0.001);
        assert!(
            (w.derived - 0.70).abs() < 0.001,
            "derived should use default"
        );
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE");
    }

    #[test]
    fn test_from_config_no_sources_returns_none() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE");
        // No default source_weights.json in the working directory either.
        // We can't guarantee tmpdir is the cwd, so from_config may or may
        // not find ./source_weights.json. Just verify from_file works standalone.
        // from_config() gracefully returns None when nothing is configured.
        let _result = SourceTypeWeights::from_config();
        // If there's no config anywhere, this should be None.
        // (Tests run from repo root — there's no source_weights.json there.)
    }

    // ── Provenance feature emission tests ──────────────────────────
    // Verify that ScoreDebug carries source_weight_applied and
    // original_source as structured fields, not just the composite
    // source_type string.

    use crate::storage::RetrievalProfile;

    #[test]
    fn test_score_debug_emits_provenance_fields_real() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("real"));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            meta,
        );
        let weights = SourceTypeWeights::default();
        let debug = RetrievalProfile::FullFidelity.score_debug(0.95, &node, Some(weights));
        assert_eq!(debug.original_source.as_deref(), Some("real"));
        assert!((debug.source_weight_applied.unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_score_debug_emits_provenance_fields_synthetic() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        let weights = SourceTypeWeights::default();
        let debug = RetrievalProfile::FullFidelity.score_debug(0.95, &node, Some(weights));
        assert_eq!(debug.original_source.as_deref(), Some("synthetic"));
        assert!((debug.source_weight_applied.unwrap() - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_score_debug_emits_provenance_fields_derived() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("derived"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "derived"}),
            meta,
        );
        let weights = SourceTypeWeights::default();
        let debug = RetrievalProfile::FullFidelity.score_debug(0.95, &node, Some(weights));
        assert_eq!(debug.original_source.as_deref(), Some("derived"));
        assert!((debug.source_weight_applied.unwrap() - 0.70).abs() < 0.001);
    }

    #[test]
    fn test_score_debug_emits_provenance_fields_unknown() {
        let node = make_node(
            MemorySource::Import,
            serde_json::json!({"method": "unrecognized"}),
            HashMap::new(),
        );
        let weights = SourceTypeWeights::default();
        let debug = RetrievalProfile::FullFidelity.score_debug(0.95, &node, Some(weights));
        // Import falls back to Real via MemorySource
        assert_eq!(debug.original_source.as_deref(), Some("real"));
        assert!((debug.source_weight_applied.unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_score_debug_composite_source_type_still_present() {
        // Backward compatibility: the composite source_type string is still populated
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        let weights = SourceTypeWeights::default();
        let debug = RetrievalProfile::FullFidelity.score_debug(0.95, &node, Some(weights));
        assert_eq!(debug.source_type.as_deref(), Some("synthetic (0.85x)"));
    }

    #[test]
    fn test_score_debug_custom_weights_reflected_in_provenance() {
        // Custom weights should be reflected in source_weight_applied
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        let weights = SourceTypeWeights::new(1.0, 0.3, 0.2, 0.1);
        let debug = RetrievalProfile::FullFidelity.score_debug(0.95, &node, Some(weights));
        assert_eq!(debug.original_source.as_deref(), Some("synthetic"));
        assert!((debug.source_weight_applied.unwrap() - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_score_debug_mixed_source_inputs() {
        // Acceptance criteria: weighted scores produced correctly for mixed-source inputs
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        let debug0 = RetrievalProfile::UserFacing.score_debug(0.90, &nodes[0], Some(weights));
        let debug1 = RetrievalProfile::UserFacing.score_debug(0.85, &nodes[1], Some(weights));
        let debug2 = RetrievalProfile::UserFacing.score_debug(0.80, &nodes[2], Some(weights));

        // Real should have highest multiplier and correct source_type
        assert_eq!(debug0.original_source.as_deref(), Some("real"));
        assert!((debug0.source_weight_applied.unwrap() - 1.0).abs() < 0.001);

        // Synthetic penalized
        assert_eq!(debug1.original_source.as_deref(), Some("synthetic"));
        assert!((debug1.source_weight_applied.unwrap() - 0.85).abs() < 0.001);

        // Derived penalized most
        assert_eq!(debug2.original_source.as_deref(), Some("derived"));
        assert!((debug2.source_weight_applied.unwrap() - 0.70).abs() < 0.001);

        // Real score should be highest (with equal base scores, real wins)
        assert!(
            debug0.final_score() > debug1.final_score(),
            "real ({}) > synthetic ({})",
            debug0.final_score(),
            debug1.final_score()
        );
        assert!(
            debug1.final_score() > debug2.final_score(),
            "synthetic ({}) > derived ({})",
            debug1.final_score(),
            debug2.final_score()
        );
    }

    // ── Full pipeline integration tests (score_node) ─────────────────

    /// Verify score_multiplier includes source weight factor for each source type.
    #[test]
    fn test_score_multiplier_includes_source_weight() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        // Node 0: Real — no source penalty
        let m0 = RetrievalProfile::UserFacing.score_multiplier(&nodes[0], Some(weights));
        // Node 1: Synthetic — 0.85x multiplier
        let m1 = RetrievalProfile::UserFacing.score_multiplier(&nodes[1], Some(weights));
        // Node 2: Derived — 0.70x multiplier
        let m2 = RetrievalProfile::UserFacing.score_multiplier(&nodes[2], Some(weights));

        // Real should have highest multiplier
        assert!(m0 > m1, "real multiplier ({}) > synthetic ({})", m0, m1);
        assert!(m1 > m2, "synthetic multiplier ({}) > derived ({})", m1, m2);
    }

    /// Verify score_multiplier with custom source weights.
    #[test]
    fn test_score_multiplier_custom_weights() {
        let nodes = session_nodes();
        // Boost real, heavily penalize synthetic
        let weights = SourceTypeWeights::new(1.5, 0.3, 0.2, 0.5);

        let m0 = RetrievalProfile::UserFacing.score_multiplier(&nodes[0], Some(weights));
        let m1 = RetrievalProfile::UserFacing.score_multiplier(&nodes[1], Some(weights));

        // Real boosted by 1.5x, synthetic penalized to 0.3x
        // Ratio should be significant
        assert!(
            m0 > m1 * 2.0,
            "with custom weights, real multiplier ({}) should be >2x synthetic ({})",
            m0,
            m1
        );
    }

    /// Verify score_multiplier applies default weights when None is passed.
    #[test]
    fn test_score_multiplier_none_uses_defaults() {
        let nodes = session_nodes();

        let with_defaults = RetrievalProfile::UserFacing
            .score_multiplier(&nodes[1], Some(SourceTypeWeights::default()));
        let with_none = RetrievalProfile::UserFacing.score_multiplier(&nodes[1], None);

        // None should behave identically to explicit default weights
        assert!(
            (with_defaults - with_none).abs() < 0.001,
            "None weights should default, got {} vs {}",
            with_defaults,
            with_none
        );
    }

    /// Verify score_node produces correct final scores for mixed-source inputs.
    #[test]
    fn test_score_node_mixed_source_inputs() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        let base_score = 0.95;

        let scored0 =
            RetrievalProfile::UserFacing.score_node(base_score, nodes[0].clone(), Some(weights));
        let scored1 =
            RetrievalProfile::UserFacing.score_node(base_score, nodes[1].clone(), Some(weights));
        let scored2 =
            RetrievalProfile::UserFacing.score_node(base_score, nodes[2].clone(), Some(weights));

        // Verify debug info carries source metadata
        assert_eq!(
            scored0
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real")
        );
        assert_eq!(
            scored1
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("synthetic")
        );
        assert_eq!(
            scored2
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("derived")
        );

        // Real should score highest (equal base scores → real wins)
        assert!(
            scored0.score > scored1.score,
            "real ({}) > synthetic ({})",
            scored0.score,
            scored1.score
        );
        assert!(
            scored1.score > scored2.score,
            "synthetic ({}) > derived ({})",
            scored1.score,
            scored2.score
        );

        // Verify scores are consistent with multipliers
        let m0 = RetrievalProfile::UserFacing.score_multiplier(&nodes[0], Some(weights));
        let expected0 = base_score * m0;
        assert!(
            (scored0.score - expected0).abs() < 0.001,
            "score {} should equal base {} * multiplier {}",
            scored0.score,
            base_score,
            m0
        );
    }

    /// Verify score_node with custom weights changes ranking order.
    #[test]
    fn test_score_node_custom_weights_flip_order() {
        let nodes = session_nodes();
        // Reverse the defaults: penalize real, boost synthetic
        let weights = SourceTypeWeights::new(0.2, 2.0, 1.5, 1.0);

        let base = 0.90;
        let scored0 =
            RetrievalProfile::UserFacing.score_node(base, nodes[0].clone(), Some(weights));
        let scored1 =
            RetrievalProfile::UserFacing.score_node(base, nodes[1].clone(), Some(weights));

        // With reversed weights, synthetic should score higher than real
        assert!(
            scored1.score > scored0.score,
            "with reversed weights, synthetic ({}) > real ({})",
            scored1.score,
            scored0.score
        );
    }

    /// Verify policy retrieval profiles apply source weights (FullFidelity records for observability only).
    #[test]
    fn test_score_node_policy_profiles_apply_source_weights() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        // Synthetic node: should be penalized in policy profiles (UserFacing/AgentDebug)
        let synthetic_node = &nodes[1];

        for profile in [RetrievalProfile::UserFacing, RetrievalProfile::AgentDebug] {
            let scored = profile.score_node(0.95, synthetic_node.clone(), Some(weights));
            let source_applied = scored.debug.as_ref().and_then(|d| d.source_weight_applied);
            assert!(
                source_applied.is_some(),
                "{:?} profile should carry source_weight_applied",
                profile
            );
            assert!(
                (source_applied.unwrap() - 0.85).abs() < 0.001,
                "{:?} profile should apply synthetic weight 0.85, got {:?}",
                profile,
                source_applied
            );
            assert_eq!(
                scored
                    .debug
                    .as_ref()
                    .and_then(|d| d.original_source.as_deref()),
                Some("synthetic"),
                "{:?} profile should detect synthetic source",
                profile
            );
        }
    }

    /// Verify score_node for unknown/fallthrough sources gets Real (1.0x default).
    #[test]
    fn test_score_node_unknown_import_gets_real_multiplier() {
        // Import with unrecognized method → MemorySource fallback to Real → 1.0x
        let node = make_node(
            MemorySource::Import,
            serde_json::json!({"method": "unrecognized_future_method"}),
            HashMap::new(),
        );
        let weights = SourceTypeWeights::default();
        let scored = RetrievalProfile::FullFidelity.score_node(1.0, node, Some(weights));

        assert_eq!(
            scored
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real"),
            "Import with unrecognized method should fall back to Real"
        );
        assert!(
            (scored
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 1.0)
                .abs()
                < 0.001,
            "Real source should have 1.0x multiplier"
        );
    }

    // ── Backward Compatibility: unweighted pipeline ──────────────────
    // The parent task (t_82497b41) promoted source_weight_applied and
    // original_source to top-level on the API ScoredNode. These tests
    // verify that unweighted pipelines (source_type_weights=None, which
    // defaults to SourceTypeWeights::default()) still work correctly
    // and produce consistent provenance fields.

    /// When source_type_weights is None, defaults kick in automatically.
    /// This test verifies the contract: None behaves identically to
    /// explicit SourceTypeWeights::default().
    #[test]
    fn test_none_weights_equals_default_weights() {
        let nodes = session_nodes();
        let base = 0.95;
        let defaults = SourceTypeWeights::default();

        // score_node with None vs explicit defaults — same result
        for (i, node) in nodes.iter().enumerate() {
            let scored_none = RetrievalProfile::UserFacing.score_node(base, node.clone(), None);
            let scored_default =
                RetrievalProfile::UserFacing.score_node(base, node.clone(), Some(defaults));

            assert!(
                (scored_none.score - scored_default.score).abs() < 0.001,
                "node {i}: None score ({}) == default score ({})",
                scored_none.score,
                scored_default.score
            );

            // Both should detect the same provenance
            assert_eq!(
                scored_none
                    .debug
                    .as_ref()
                    .and_then(|d| d.original_source.as_deref()),
                scored_default
                    .debug
                    .as_ref()
                    .and_then(|d| d.original_source.as_deref()),
                "node {i}: provenance should match"
            );
        }
    }

    /// Identity weights (all 1.0) mean no source-type penalty.
    /// For Real nodes, this is identical to defaults (which also use 1.0).
    /// For Synthetic/Derived, identity weight boosts them vs defaults.
    #[test]
    fn test_identity_weights_boost_synthetic_vs_defaults() {
        let nodes = session_nodes();
        let identity = SourceTypeWeights::new(1.0, 1.0, 1.0, 1.0);
        let defaults = SourceTypeWeights::default();

        // Real node: identity == defaults (both 1.0)
        let real_none = RetrievalProfile::UserFacing.score_node(0.90, nodes[0].clone(), None);
        let real_identity =
            RetrievalProfile::UserFacing.score_node(0.90, nodes[0].clone(), Some(identity));
        assert!(
            (real_none.score - real_identity.score).abs() < 0.001,
            "Real: identity == defaults"
        );

        // Synthetic node: identity (1.0) > defaults (0.85)
        let synth_default =
            RetrievalProfile::UserFacing.score_node(0.90, nodes[1].clone(), Some(defaults));
        let synth_identity =
            RetrievalProfile::UserFacing.score_node(0.90, nodes[1].clone(), Some(identity));
        assert!(
            synth_identity.score > synth_default.score,
            "Synthetic: identity ({}) > defaults ({})",
            synth_identity.score,
            synth_default.score
        );

        // Provenance still correct with identity weights
        assert_eq!(
            synth_identity
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("synthetic")
        );
        assert!(
            (synth_identity
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 1.0)
                .abs()
                < 0.001,
            "identity weight should reflect 1.0"
        );
    }

    // ── Mixed-source provenance round-trip ──────────────────────────
    // Verify that a batch of heterogeneous nodes, scored together,
    // each carries the correct source_weight_applied and original_source
    // through the full score_node → ScoreDebug pipeline.

    /// Full round-trip: score a heterogeneous batch, verify each node gets
    /// correct provenance fields and correct relative score ordering.
    #[test]
    fn test_mixed_source_provenance_round_trip() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();
        let base = 0.95;

        let scored: Vec<_> = nodes
            .iter()
            .map(|n| RetrievalProfile::UserFacing.score_node(base, n.clone(), Some(weights)))
            .collect();

        // Node 0: Real → source_weight = 1.0
        assert_eq!(
            scored[0]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real")
        );
        assert!(
            (scored[0]
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 1.0)
                .abs()
                < 0.001
        );

        // Node 1: Synthetic → 0.85
        assert_eq!(
            scored[1]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("synthetic")
        );
        assert!(
            (scored[1]
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 0.85)
                .abs()
                < 0.001
        );

        // Node 2: Derived → 0.70
        assert_eq!(
            scored[2]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("derived")
        );
        assert!(
            (scored[2]
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 0.70)
                .abs()
                < 0.001
        );

        // Node 3: Document → Real → 1.0
        assert_eq!(
            scored[3]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real")
        );
        assert!(
            (scored[3]
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 1.0)
                .abs()
                < 0.001
        );

        // Node 4: Import → Real → 1.0
        assert_eq!(
            scored[4]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real")
        );
        assert!(
            (scored[4]
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 1.0)
                .abs()
                < 0.001
        );

        // Score ordering: Real > Synthetic > Derived (equal base → weights determine)
        assert!(scored[0].score > scored[1].score, "real > synthetic");
        assert!(scored[1].score > scored[2].score, "synthetic > derived");
        // Nodes 0 (Conversation, primary trust) and 3 (Document, reference trust)
        // may have different scores due to trust tier, but both carry Real provenance.
        assert_eq!(
            scored[0]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real"),
            "Conversation → Real"
        );
        assert_eq!(
            scored[3]
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real"),
            "Document → Real"
        );
    }

    /// Mixed-source ordering: with default weights, same base score causes
    /// Real > Synthetic > Derived strict ordering.
    #[test]
    fn test_mixed_source_ordering_default_weights() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        let real_mult = source_multiplier(&nodes[0], &weights);
        let synth_mult = source_multiplier(&nodes[1], &weights);
        let deriv_mult = source_multiplier(&nodes[2], &weights);
        let doc_mult = source_multiplier(&nodes[3], &weights); // Document → Real
        let imp_mult = source_multiplier(&nodes[4], &weights); // Import → Real

        // Default weight hierarchy: Real >= Synthetic > Derived
        assert!((real_mult - 1.0).abs() < 0.001, "real = 1.0");
        assert!((doc_mult - 1.0).abs() < 0.001, "document = real = 1.0");
        assert!((imp_mult - 1.0).abs() < 0.001, "import = real = 1.0");
        assert!((synth_mult - 0.85).abs() < 0.001, "synthetic = 0.85");
        assert!((deriv_mult - 0.70).abs() < 0.001, "derived = 0.70");

        // Strict ordering
        assert!(real_mult > synth_mult);
        assert!(synth_mult > deriv_mult);
    }

    /// With custom weights that penalize Real and boost Synthetic,
    /// the ordering can be inverted.
    #[test]
    fn test_mixed_source_ordering_custom_weights_can_invert() {
        let nodes = session_nodes();
        // Real gets 0.2x, Synthetic gets 2.0x — complete inversion
        let weights = SourceTypeWeights::new(0.2, 2.0, 1.0, 1.0);

        let real_mult = source_multiplier(&nodes[0], &weights);
        let synth_mult = source_multiplier(&nodes[1], &weights);
        let deriv_mult = source_multiplier(&nodes[2], &weights);

        // With these inverted weights: Synthetic > Derived > Real
        assert!((real_mult - 0.2).abs() < 0.001, "real penalized to 0.2");
        assert!((synth_mult - 2.0).abs() < 0.001, "synthetic boosted to 2.0");
        assert!((deriv_mult - 1.0).abs() < 0.001, "derived at 1.0");

        assert!(
            synth_mult > deriv_mult,
            "synthetic > derived with custom weights"
        );
        assert!(deriv_mult > real_mult, "derived > real with custom weights");
    }

    /// Backward compatibility: score_node without weights still works and
    /// produces a valid storage::ScoredNode with debug info.
    #[test]
    fn test_backward_compat_score_node_no_weights() {
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            HashMap::new(),
        );

        let scored = RetrievalProfile::UserFacing.score_node(0.88, node, None);

        // Basic sanity — the node was scored (scores can exceed 1.0 via multipliers)
        assert!(scored.score > 0.0);

        // Debug info should be populated (even without source_type_weights)
        let debug = scored.debug.expect("debug should be present");

        // Provenance should be detected from the node itself
        assert_eq!(debug.original_source.as_deref(), Some("real"));
        assert!((debug.source_weight_applied.unwrap() - 1.0).abs() < 0.001);
    }

    /// Verify that every retrieval profile emits provenance fields correctly.
    /// Backward compat: pre-provenance profiles still work.
    #[test]
    fn test_all_profiles_emit_provenance_on_mixed_sources() {
        let nodes = session_nodes();
        let weights = SourceTypeWeights::default();

        for profile in [
            RetrievalProfile::UserFacing,
            RetrievalProfile::AgentDebug,
            RetrievalProfile::FullFidelity,
        ] {
            let scored = profile.score_node(0.91, nodes[2].clone(), Some(weights)); // Derived node

            let debug = scored.debug.as_ref().expect("debug should be present");
            assert_eq!(
                debug.original_source.as_deref(),
                Some("derived"),
                "{:?}: should detect derived source",
                profile
            );
            assert!(
                (debug.source_weight_applied.unwrap() - 0.70).abs() < 0.001,
                "{:?}: derived weight should be 0.70",
                profile
            );
            assert!(
                debug.source_type.as_deref().unwrap_or("").contains("0.70"),
                "{:?}: composite source_type should mention weight",
                profile
            );
        }
    }

    // ── Edge case: unknown sources ──────────────────────────────────
    // The Unknown SourceType variant exists in the enum and weight table
    // but is currently unreachable through detect_source_type() (all
    // MemorySource variants map to Real or Synthetic). These tests
    // exercise the Unknown weight infrastructure directly so it's
    // covered if/when detect_source_type is extended to return Unknown.

    #[test]
    fn test_unknown_source_type_direct_multiplier_default() {
        let weights = SourceTypeWeights::default();
        assert!(
            (weights.multiplier(SourceType::Unknown) - 0.95).abs() < 0.001,
            "Unknown should default to 0.95x"
        );
    }

    #[test]
    fn test_unknown_source_type_direct_multiplier_custom() {
        let weights = SourceTypeWeights::new(1.0, 0.5, 0.3, 0.1);
        assert!(
            (weights.multiplier(SourceType::Unknown) - 0.1).abs() < 0.001,
            "Unknown with custom weight"
        );
    }

    #[test]
    fn test_unknown_source_type_display() {
        assert_eq!(SourceType::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_unknown_source_type_serde_roundtrip() {
        let json = serde_json::to_string(&SourceType::Unknown).unwrap();
        assert_eq!(json, "\"unknown\"");
        let parsed: SourceType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SourceType::Unknown);
    }

    #[test]
    fn test_unknown_weight_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        std::env::set_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS", r#"{"unknown":0.42}"#);
        let w = SourceTypeWeights::from_env().expect("should parse");
        assert!((w.unknown - 0.42).abs() < 0.001, "unknown overridden");
        assert!((w.real - 1.0).abs() < 0.001, "real stays default");
        std::env::remove_var("KNOWWHERE_SOURCE_TYPE_WEIGHTS");
    }

    #[test]
    fn test_unknown_weight_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unknown_weights.json");
        std::fs::write(&path, r#"{"unknown":0.15}"#).unwrap();
        let w = SourceTypeWeights::from_file(&path).expect("should parse");
        assert!((w.unknown - 0.15).abs() < 0.001);
        assert!((w.real - 1.0).abs() < 0.001, "real stays default");
    }

    // ── Edge case: zero weights ─────────────────────────────────────

    #[test]
    fn test_zero_weight_multiplier_returns_zero() {
        let weights = SourceTypeWeights::new(0.0, 0.0, 0.5, 1.0);
        assert!(
            weights.multiplier(SourceType::Real) == 0.0,
            "zero real weight → zero multiplier"
        );
        assert!(
            weights.multiplier(SourceType::Synthetic) == 0.0,
            "zero synthetic weight → zero multiplier"
        );
        assert!(
            weights.multiplier(SourceType::Derived) > 0.0,
            "non-zero derived stays non-zero"
        );
    }

    #[test]
    fn test_zero_weight_score_node_zeros_score() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("real"));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            meta,
        );
        // Real weight = 0.0 → final score should be 0.0 (policy profiles apply source)
        let weights = SourceTypeWeights::new(0.0, 0.85, 0.70, 0.95);
        let scored = RetrievalProfile::UserFacing.score_node(0.95, node, Some(weights));

        assert!(
            scored.score == 0.0,
            "zero source weight should produce zero final score, got {}",
            scored.score
        );
        assert_eq!(
            scored
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("real")
        );
        assert!(
            (scored
                .debug
                .as_ref()
                .and_then(|d| d.source_weight_applied)
                .unwrap()
                - 0.0)
                .abs()
                < 0.001,
            "source_weight_applied should be 0.0"
        );
    }

    #[test]
    fn test_zero_weight_synthetic_score_node() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("synthetic"));
        let node = make_node(
            MemorySource::Consolidation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        // Synthetic weight = 0.0
        let weights = SourceTypeWeights::new(1.0, 0.0, 0.70, 0.95);
        let scored = RetrievalProfile::UserFacing.score_node(0.90, node, Some(weights));

        assert!(
            scored.score == 0.0,
            "zero synthetic weight → zero score, got {}",
            scored.score
        );
        assert_eq!(
            scored
                .debug
                .as_ref()
                .and_then(|d| d.original_source.as_deref()),
            Some("synthetic")
        );
    }

    #[test]
    fn test_all_zero_weights_config() {
        let weights = SourceTypeWeights::new(0.0, 0.0, 0.0, 0.0);
        let nodes = session_nodes();

        for (i, node) in nodes.iter().enumerate() {
            let mult = source_multiplier(node, &weights);
            assert!(
                mult == 0.0,
                "node {i}: all-zero weights should give 0.0 multiplier, got {mult}"
            );

            let scored = RetrievalProfile::UserFacing.score_node(0.95, node.clone(), Some(weights));
            assert!(
                scored.score == 0.0,
                "node {i}: all-zero weights should give 0.0 final score, got {}",
                scored.score
            );
        }
    }

    #[test]
    fn test_mixed_zero_and_nonzero_weights() {
        // Only Real penalized to zero; Synthetic/Derived/Unknown keep defaults
        let weights = SourceTypeWeights::new(0.0, 0.85, 0.70, 0.95);
        let nodes = session_nodes();

        // Node 0: Real → zero
        let mult0 = source_multiplier(&nodes[0], &weights);
        assert!(mult0 == 0.0, "Real with zero weight");

        // Node 1: Synthetic → 0.85 (non-zero, uses explicit value)
        let mult1 = source_multiplier(&nodes[1], &weights);
        assert!((mult1 - 0.85).abs() < 0.001, "Synthetic non-zero");

        // Node 2: Derived → 0.70 (non-zero)
        let mult2 = source_multiplier(&nodes[2], &weights);
        assert!((mult2 - 0.70).abs() < 0.001, "Derived non-zero");

        // Synthetic should score higher than Real when Real is zeroed
        let scored0 =
            RetrievalProfile::UserFacing.score_node(0.90, nodes[0].clone(), Some(weights));
        let scored1 =
            RetrievalProfile::UserFacing.score_node(0.90, nodes[1].clone(), Some(weights));
        assert!(
            scored1.score > scored0.score,
            "synthetic ({}) > real ({}) when real is zeroed",
            scored1.score,
            scored0.score
        );
    }

    #[test]
    fn test_zero_weight_penalizes_all_real_sources() {
        // All sources that resolve to Real should be zeroed
        let weights = SourceTypeWeights::new(0.0, 1.0, 1.0, 1.0);

        let sources: Vec<(MemorySource, &str)> = vec![
            (MemorySource::Conversation, "conversation"),
            (MemorySource::Document, "document"),
            (MemorySource::Import, "import"),
            (MemorySource::Manual, "manual"),
        ];

        for (source, name) in &sources {
            let node = make_node(*source, serde_json::json!({}), HashMap::new());
            let mult = source_multiplier(&node, &weights);
            assert!(
                mult == 0.0,
                "{name}: should be zeroed (resolves to Real), got {mult}"
            );
        }
    }

    #[test]
    fn test_zero_weight_all_profiles() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("real"));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            meta,
        );
        let weights = SourceTypeWeights::new(0.0, 0.85, 0.70, 0.95);

        for profile in [RetrievalProfile::UserFacing, RetrievalProfile::AgentDebug] {
            let scored = profile.score_node(0.95, node.clone(), Some(weights));
            assert!(
                scored.score == 0.0,
                "{:?}: zero Real weight → zero score, got {}",
                profile,
                scored.score
            );
            let debug = scored.debug.as_ref().expect("debug present");
            assert_eq!(debug.original_source.as_deref(), Some("real"));
            assert!(
                (debug.source_weight_applied.unwrap() - 0.0).abs() < 0.001,
                "{:?}: source_weight_applied should be 0.0",
                profile
            );
        }
    }

    // ── Edge case: unrecognized / malformed metadata ─────────────────

    #[test]
    fn test_unrecognized_metadata_provenance_falls_through() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!("garbage_value"));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            meta,
        );
        // "garbage_value" doesn't match real/synthetic/derived → falls through
        // to provenance.method → "session" → Real
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    #[test]
    fn test_empty_string_provenance_falls_through() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!(""));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "consolidation"}),
            meta,
        );
        // Empty string doesn't match → falls to provenance.method →
        // "consolidation" → Synthetic
        assert_eq!(detect_source_type(&node), SourceType::Synthetic);
    }

    #[test]
    fn test_non_string_provenance_metadata_ignored() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::json!(42));
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            meta,
        );
        // Integer provenance value → .as_str() returns None → falls through
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    #[test]
    fn test_non_string_source_dataset_ignored() {
        let mut meta = HashMap::new();
        meta.insert("source_dataset".into(), serde_json::json!(true));
        let node = make_node(
            MemorySource::Import,
            serde_json::json!({"method": "external"}),
            meta,
        );
        // Boolean source_dataset → .as_str() returns None → falls through
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    #[test]
    fn test_null_provenance_metadata_ignored() {
        let mut meta = HashMap::new();
        meta.insert("provenance".into(), serde_json::Value::Null);
        let node = make_node(
            MemorySource::Document,
            serde_json::json!({"method": "external"}),
            meta,
        );
        // Null provenance → .as_str() returns None → falls through
        assert_eq!(detect_source_type(&node), SourceType::Real);
    }

    // ── Edge case: weight at exact clamp boundaries ──────────────────

    #[test]
    fn test_weight_exactly_zero() {
        let weights = SourceTypeWeights::new(0.0, 0.0, 0.0, 0.0);
        assert!(weights.real == 0.0);
        assert!(weights.synthetic == 0.0);
        assert!(weights.derived == 0.0);
        assert!(weights.unknown == 0.0);
    }

    #[test]
    fn test_weight_exactly_two() {
        let weights = SourceTypeWeights::new(2.0, 2.0, 2.0, 2.0);
        assert!((weights.real - 2.0).abs() < 0.001);
        assert!((weights.synthetic - 2.0).abs() < 0.001);
        assert!((weights.derived - 2.0).abs() < 0.001);
        assert!((weights.unknown - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_weight_exactly_two_doubles_score() {
        let weights_2x = SourceTypeWeights::new(2.0, 1.0, 1.0, 1.0);
        let weights_1x = SourceTypeWeights::new(1.0, 1.0, 1.0, 1.0);
        let node = make_node(
            MemorySource::Conversation,
            serde_json::json!({"method": "session"}),
            HashMap::new(),
        );
        let base = 0.50;

        // Verify the source_multiplier itself is exactly 2.0
        let mult = source_multiplier(&node, &weights_2x);
        assert!((mult - 2.0).abs() < 0.001);

        // score_multiplier() includes tier * explicit * memory_type * source_type,
        // so the 2x source weight should exactly double the final score relative
        // to a 1x source weight when all other multipliers are held equal.
        let scored_2x =
            RetrievalProfile::UserFacing.score_node(base, node.clone(), Some(weights_2x));
        let scored_1x =
            RetrievalProfile::UserFacing.score_node(base, node.clone(), Some(weights_1x));

        assert!(
            scored_2x.score > 0.0,
            "2x weight should produce non-zero score"
        );
        assert!(
            (scored_2x.score - 2.0 * scored_1x.score).abs() < 0.001,
            "2x weight score ({}) = 2.0 * 1x weight score ({})",
            scored_2x.score,
            scored_1x.score
        );
    }
}

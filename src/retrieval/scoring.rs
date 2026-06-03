// src/retrieval/scoring.rs
use crate::memory::fractal_node::FractalNode;
use crate::retrieval::source_weighting::{self, SourceTypeWeights};
use crate::storage::backend::{RetrievalProfile, ScoreDebug, ScoredNode};

#[derive(Debug, Clone, Default)]
pub struct ScoringContext {
    pub source_type_weights: Option<SourceTypeWeights>,
    pub temporal_weight: Option<f32>,
}

pub struct ScoringEngine;

impl ScoringEngine {
    /// Core-Faktor (immer): nur Ebbinghaus.
    pub fn core_multiplier(node: &FractalNode) -> f32 {
        node.ebbinghaus_decay(chrono::Utc::now()) as f32
    }

    /// Reiner Core-Score (für Contract-Tests).
    /// Garantie: FullFidelity-Pfad produziert exakt diesen Wert (bei purem Cosine-Signal).
    pub fn core_score(signal: f32, node: &FractalNode) -> f32 {
        signal * Self::core_multiplier(node)
    }

    /// Effektiver Multiplier für ein Node unter gegebenem Profile.
    /// FullFidelity → exakt core_multiplier (Ebbinghaus).
    /// UserFacing/AgentDebug → tier * explicit * mtype * source * ebbinghaus.
    pub fn multiplier(
        profile: RetrievalProfile,
        node: &FractalNode,
        weights: Option<SourceTypeWeights>,
    ) -> f32 {
        let ebbi = Self::core_multiplier(node);
        if matches!(profile, RetrievalProfile::FullFidelity) {
            return ebbi;
        }
        let w = weights.unwrap_or_default();
        let src = source_weighting::source_multiplier(node, &w);
        let tier = Self::tier_multiplier(node.trust_tier());
        let mtype = Self::memory_type_multiplier(node);
        let expl = Self::explicit_weight(node);
        tier * expl * mtype * src * ebbi
    }

    pub fn score_node(
        profile: RetrievalProfile,
        base_score: f32,
        node: FractalNode,
        weights: Option<SourceTypeWeights>,
    ) -> ScoredNode {
        let debug = Self::score_debug(profile, base_score, &node, weights);
        ScoredNode {
            id: node.id,
            score: debug.final_score(),
            distribution_scores: None,
            debug: Some(debug),
            node,
        }
    }

    pub fn score_debug(
        profile: RetrievalProfile,
        base_score: f32,
        node: &FractalNode,
        weights: Option<SourceTypeWeights>,
    ) -> ScoreDebug {
        let src_type = source_weighting::detect_source_type(node).to_string();
        let w = weights.unwrap_or_default();
        let src_mult = source_weighting::source_multiplier(node, &w);
        let eff_mult = Self::multiplier(profile, node, Some(w));
        ScoreDebug {
            profile,
            trust_tier: node.trust_tier().to_string(),
            base_score,
            multiplier: eff_mult,
            source_type: Some(format!("{src_type} ({src_mult:.2}x)")),
            source_weight_applied: Some(src_mult),
            original_source: Some(src_type),
            ebbinghaus_factor: Some(Self::core_multiplier(node)),
            recency_factor: None,
            session_boost: None,
            temporal_weight: None,
            explanation: None,
        }
    }

    // Policy-Funktionen (extrahiert aus altem backend.rs, identisches Verhalten)
    fn tier_multiplier(trust_tier: &str) -> f32 {
        match trust_tier {
            "primary" => 1.3,
            "reference" => 1.1,
            "derived" => 0.9,
            "volatile" => 0.7,
            _ => 1.0,
        }
    }

    fn memory_type_multiplier(node: &FractalNode) -> f32 {
        use crate::memory::types::MemoryType;
        match node.memory_type {
            MemoryType::Decision => 1.5,
            MemoryType::Preference => 1.2,
            MemoryType::Procedural => 1.15,
            MemoryType::Semantic => 1.05,
            _ => 1.0,
        }
    }

    fn explicit_weight(node: &FractalNode) -> f32 {
        node.explicit_trust_weight()
            .or_else(|| {
                if (node.weight - 1.0).abs() > f64::EPSILON {
                    Some(node.weight as f32)
                } else {
                    None
                }
            })
            .unwrap_or(1.0)
            .clamp(0.1, 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::fractal_node::FractalNode;
    use crate::memory::types::MemoryType;
    use crate::storage::backend::RetrievalProfile;
    use std::collections::HashMap;

    fn make_node(memory_type: MemoryType, trust_tier: Option<&str>) -> FractalNode {
        let mut meta = HashMap::new();
        if let Some(t) = trust_tier {
            meta.insert("trust_tier".to_string(), serde_json::json!(t));
        }
        FractalNode::new_typed(
            Some("test".into()), None, vec![1.0; 4],
            meta, memory_type, crate::memory::types::MemorySource::Conversation,
        )
    }

    #[test]
    fn full_fidelity_multiplier_is_ebbinghaus_only() {
        let node = make_node(MemoryType::Decision, None);
        let mult = ScoringEngine::multiplier(RetrievalProfile::FullFidelity, &node, None);
        let ebbi = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        assert!((mult - ebbi).abs() < 1e-6,
            "FullFidelity multiplier ({mult}) must equal ebbinghaus ({ebbi})");
    }

    #[test]
    fn full_fidelity_ignores_tier_mtype_source() {
        // Decision=primary(1.3) + Decision mtype(1.5) + Conversation=Real(1.0) → 1.95 policy
        // FullFidelity should IGNORE all of that
        let node = make_node(MemoryType::Decision, None);
        let mult = ScoringEngine::multiplier(RetrievalProfile::FullFidelity, &node, None);
        let ebbi = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        assert!((mult - ebbi).abs() < 1e-6);
        assert!(mult < 1.1); // ebbinghaus is ~1.0 for fresh node, policy would be 1.95
    }

    #[test]
    fn user_facing_applies_policy_multipliers() {
        let node = make_node(MemoryType::Decision, None);
        let mult = ScoringEngine::multiplier(RetrievalProfile::UserFacing, &node, None);
        // Decision=primary(1.3) × explicit(1.0) × mtype(1.5) × source(1.0) × ebbi(~1.0) ≈ 1.95
        let ebbi = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        let expected = 1.3 * 1.0 * 1.5 * 1.0 * ebbi;
        assert!((mult - expected).abs() < 1e-6,
            "UserFacing multiplier ({mult}) must include policy, expected ~{expected}");
        assert!(mult > 1.5); // must be significantly >1.0
    }

    #[test]
    fn full_fidelity_core_score_equals_cosine_times_ebbinghaus() {
        let node = make_node(MemoryType::Semantic, None);
        let cos = 0.85_f32;
        let score = ScoringEngine::core_score(cos, &node);
        let ebbi = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        assert!((score - cos * ebbi).abs() < 1e-6);
    }

    #[test]
    fn full_fidelity_score_node_produces_core_score() {
        let node = make_node(MemoryType::Episodic, None);
        let ebbi = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        let base = 0.75_f32;
        let scored = ScoringEngine::score_node(
            RetrievalProfile::FullFidelity, base, node.clone(), None,
        );
        assert!((scored.score - base * ebbi).abs() < 1e-6);
        assert_eq!(scored.id, node.id);
    }

    #[test]
    fn score_debug_includes_source_info_even_under_full_fidelity() {
        let node = make_node(MemoryType::Semantic, None);
        let debug = ScoringEngine::score_debug(
            RetrievalProfile::FullFidelity, 1.0, &node, None,
        );
        // Source fields are populated for observability even if not used in multiplier
        assert!(debug.source_type.is_some());
        assert!(debug.source_weight_applied.is_some());
        assert!(debug.ebbinghaus_factor.is_some());
        // But multiplier must equal ebbinghaus
        let ebbi = debug.ebbinghaus_factor.unwrap();
        assert!((debug.multiplier - ebbi).abs() < 1e-6);
    }

    #[test]
    fn explicit_trust_tier_is_respected() {
        let node = make_node(MemoryType::Decision, Some("volatile"));
        // Explicit "volatile" overrides Decision→primary hard-rule
        assert_eq!(node.trust_tier(), "volatile");
        let mult = ScoringEngine::multiplier(RetrievalProfile::UserFacing, &node, None);
        // tier=0.7(volatile) × 1.0 × 1.5 × 1.0 × ebbi ≈ 1.05
        let ebbi = node.ebbinghaus_decay(chrono::Utc::now()) as f32;
        let expected = 0.7 * 1.0 * 1.5 * 1.0 * ebbi;
        assert!((mult - expected).abs() < 1e-6);
    }
}

//! Shared storage utilities — used by both MemoryStore and PostgresStore.
//!
//! Functions here are backend-agnostic and must produce identical results
//! regardless of which storage backend is active. Keep them DRY.

use std::cmp::Ordering;
use std::collections::HashMap;
use uuid::Uuid;

use crate::memory::fractal_node::FractalNode;
use crate::memory::types::MemoryType;

// ── Internal Meta Filtering ────────────────────────────────────────────

/// Whether the given memory-type filter permits internal meta artifacts.
pub(crate) fn allow_internal_meta(filter: Option<MemoryType>) -> bool {
    filter == Some(MemoryType::Meta)
}

/// Returns `true` when a FractalNode is an internal meta artifact that should be
/// hidden from normal retrieval.
pub(crate) fn is_internal_meta_artifact(node: &FractalNode) -> bool {
    if node.memory_type != MemoryType::Meta {
        return false;
    }
    let derivation = node
        .metadata
        .get(FractalNode::DERIVATION_KEY)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(derivation.as_str(), "instruction" | "reflected")
        || node
            .metadata
            .get(FractalNode::RETRIEVAL_VISIBILITY_KEY)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| v.eq_ignore_ascii_case(FractalNode::INTERNAL_VISIBILITY))
}

// ── Reciprocal Rank Fusion ─────────────────────────────────────────────

/// Reciprocal Rank Fusion (RRF) — combines vector and BM25 result sets into a
/// single ranked list.
///
/// BM25 scores (typically 0–20) are normalized to 0–1 and used as a
/// confidence weight on the rank contribution. Pure vector results get a
/// flat rank-based contribution.
pub(crate) fn rrf_fuse(
    vector_ids: &[Uuid],
    bm25_results: &[(Uuid, f32)],
    k: f32,
) -> Vec<(Uuid, f32)> {
    let mut scores: HashMap<Uuid, f32> = HashMap::new();

    // Vector results — pure rank contribution
    for (rank, id) in vector_ids.iter().enumerate() {
        let score = 1.0 / (k + (rank as f32 + 1.0));
        *scores.entry(*id).or_insert(0.0) += score;
    }

    // BM25 results — rank contribution weighted by normalized BM25 score
    for (rank, (id, bm25_score)) in bm25_results.iter().enumerate() {
        let score = 1.0 / (k + (rank as f32 + 1.0));
        let normalized_bm25 = (bm25_score / 20.0).min(1.0);
        *scores.entry(*id).or_insert(0.0) += score * (normalized_bm25 + 0.1);
    }

    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    results
}

// ── Recency ─────────────────────────────────────────────────────────────

/// Exponential recency factor with configurable half-life.
///
/// Returns 1.0 for age=0, 0.5 at age=half_life_days, and decays toward a
/// floor of 0.05.
///
/// Used by both backends' temporal scoring and in tests.
pub(crate) fn recency_factor(age_days: f32, half_life_days: f32) -> f32 {
    0.5f32.powf(age_days / half_life_days).max(0.05)
}

// ── Temporal Scoring ───────────────────────────────────────────────────

/// Hybrid temporal + semantic scoring on ScoredNode slices (WP1).
///
/// Blends semantic similarity with global recency (exponential decay, 7-day
/// half-life) using a configurable weight. Produces identical results across
/// both storage backends.
///
/// `temporal_weight` is clamped to [0.0, 0.8] — purely semantic at 0.0,
/// dominated by recency at 0.8.
pub(crate) fn apply_hybrid_temporal_scoring(
    results: &mut [crate::storage::backend::ScoredNode],
    temporal_weight: f32,
) {
    if results.is_empty() || temporal_weight <= 0.0 {
        tracing::info!(
            results_empty = results.is_empty(),
            temporal_weight,
            "hybrid_temporal_scoring SKIPPED (empty or weight <= 0)"
        );
        return;
    }
    let w = temporal_weight.clamp(0.0, 0.8);
    let now = chrono::Utc::now();

    // Snapshot pre-scoring state
    let original_scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let original_top3_ids: Vec<String> = results.iter().take(3).map(|r| r.id.to_string()).collect();

    let mut recency_factors: Vec<f32> = Vec::with_capacity(results.len());

    for item in results.iter_mut() {
        let age_days = (now - item.node.created_at).num_days() as f32;
        // Half-life of 7 days — see postgres_store.rs history for tuning rationale.
        let rf = recency_factor(age_days, 7.0);
        recency_factors.push(rf);

        if let Some(debug) = &mut item.debug {
            debug.recency_factor = Some(rf);
            debug.temporal_weight = Some(w);
            debug.explanation = Some(format!(
                "Hybrid score: semantic×{:.2} + recency({:.1}d)×{:.2}",
                1.0 - w,
                age_days,
                w
            ));
        }

        // Hybrid score: semantic * (1-w) + recency * w
        item.score = item.score * (1.0 - w) + rf * w;
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Diagnostics
    let new_scores: Vec<f32> = results.iter().map(|r| r.score).collect();
    let new_top3_ids: Vec<String> = results.iter().take(3).map(|r| r.id.to_string()).collect();

    let max_delta = original_scores
        .iter()
        .zip(new_scores.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let mean_delta = original_scores
        .iter()
        .zip(new_scores.iter())
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / results.len() as f32;
    let recency_min = recency_factors.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let recency_max = recency_factors
        .iter()
        .fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let recency_mean = recency_factors.iter().sum::<f32>() / recency_factors.len() as f32;
    let recency_range = recency_max - recency_min;
    let reorder_happened = original_top3_ids != new_top3_ids;

    tracing::info!(
        weight = w,
        nodes = results.len(),
        score_range_before = format!(
            "{:.4}-{:.4}",
            original_scores.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
            original_scores
                .iter()
                .fold(f32::NEG_INFINITY, |a, &b| a.max(b))
        ),
        score_range_after = format!(
            "{:.4}-{:.4}",
            new_scores.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
            new_scores.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))
        ),
        max_delta = format!("{:.4}", max_delta),
        mean_delta = format!("{:.4}", mean_delta),
        recency_factor_range = format!("{:.4}-{:.4}", recency_min, recency_max),
        recency_factor_mean = format!("{:.4}", recency_mean),
        recency_range = format!("{:.4}", recency_range),
        top3_reordered = reorder_happened,
        "hybrid_temporal_scoring applied (WP1) — diagnostics"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_factor_at_half_life_is_0_5() {
        let rf = recency_factor(7.0, 7.0);
        assert!((rf - 0.5).abs() < 0.01);
    }

    #[test]
    fn recency_factor_at_double_half_life_is_0_25() {
        let rf = recency_factor(14.0, 7.0);
        assert!((rf - 0.25).abs() < 0.01);
    }

    #[test]
    fn recency_factor_floor_is_0_05() {
        let rf = recency_factor(1000.0, 7.0);
        assert!((rf - 0.05).abs() < 0.001);
    }

    #[test]
    fn recency_factor_zero_age_is_1_0() {
        let rf = recency_factor(0.0, 7.0);
        assert!((rf - 1.0).abs() < 0.001);
    }
}

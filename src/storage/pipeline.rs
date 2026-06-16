//! Shared retrieval pipeline — backend-agnostic post-fusion processing.
//!
//! After a backend has materialized `(score, FractalNode)` tuples from its
//! search path, it feeds them into [`finalize_retrieval`], which runs the
//! common filter → score → sort → temporal → distributional chain.
//! Both backends produce identical results for identical inputs.
//!
//! This module exists because the same ~80 lines of pipeline logic were
//! duplicated in `in_memory.rs` and (4×!) in `postgres_store.rs`.

use std::cmp::Ordering;

use chrono::{DateTime, Utc};

use crate::memory::fractal_node::FractalNode;
use crate::storage::backend::{HybridQuery, RetrievalProfile, ScoredNode};
use crate::storage::shared;

// ── Temporal Boost (generic over input type) ─────────────────────────

/// Trait abstracting over the two types that receive temporal recency boosts:
/// raw `(f32, FractalNode)` tuples (in-memory) and `ScoredNode` (postgres).
pub(crate) trait TemporalScore {
    fn score(&self) -> f32;
    fn set_score(&mut self, s: f32);
    fn created_at(&self) -> DateTime<Utc>;
}

impl TemporalScore for (f32, FractalNode) {
    fn score(&self) -> f32 {
        self.0
    }
    fn set_score(&mut self, s: f32) {
        self.0 = s;
    }
    fn created_at(&self) -> DateTime<Utc> {
        self.1.created_at
    }
}

impl TemporalScore for ScoredNode {
    fn score(&self) -> f32 {
        self.score
    }
    fn set_score(&mut self, s: f32) {
        self.score = s;
    }
    fn created_at(&self) -> DateTime<Utc> {
        self.node.created_at
    }
}

/// Apply temporal recency boost to close-scoring items.
///
/// Items whose score is within `recency_boost * 0.5` of the maximum score
/// receive a bonus proportional to how recent they are relative to the
/// newest item in the set. Results are re-sorted by score descending.
///
/// Returns the count of items that received a boost.
pub(crate) fn apply_temporal_boost<T: TemporalScore>(items: &mut [T], recency_boost: f32) -> usize {
    let mut boosted = 0usize;
    if items.is_empty() {
        return boosted;
    }
    let newest = items.iter().map(|n| n.created_at()).max();
    let Some(newest) = newest else { return boosted };

    let oldest = items.iter().map(|n| n.created_at()).min().unwrap_or(newest);
    let time_range = (newest - oldest).num_seconds() as f32;
    if time_range < 1.0 {
        return boosted; // All roughly same age — no meaningful recency gradient
    }

    let max_score = items
        .iter()
        .map(|n| n.score())
        .fold(f32::NEG_INFINITY, f32::max);
    let closeness_threshold = recency_boost * 0.5;

    for item in items.iter_mut() {
        if (max_score - item.score()).abs() <= closeness_threshold {
            let age_seconds = (newest - item.created_at()).num_seconds() as f32;
            let recency_factor = 1.0 - (age_seconds / time_range).clamp(0.0, 1.0);
            item.set_score(item.score() + recency_boost * recency_factor);
            boosted += 1;
        }
    }

    items.sort_by(|a, b| b.score().partial_cmp(&a.score()).unwrap_or(Ordering::Equal));
    tracing::info!(
        boosted,
        total = items.len(),
        boost_factor = recency_boost,
        time_range_s = time_range,
        "temporal_boost applied"
    );
    boosted
}

// ── Pipeline ─────────────────────────────────────────────────────────

/// Shared post-fusion retrieval pipeline.
///
/// Accepts materialized `(score, node)` tuples — the backend is responsible
/// for vector search, BM25, RRF fusion, and node materialization. The pipeline
/// runs the fully deterministic post-materialization chain:
///
/// 1. Profile filter ([`RetrievalProfile::allows`])
/// 2. Internal-meta filter
/// 3. Memory-type filter
/// 4. User-id filter
/// 5. Score conversion ([`RetrievalProfile::score_node`])
/// 6. Stable sort (score desc, UUID tiebreaker)
/// 7. Hybrid temporal scoring (WP1, policy-gated)
/// 8. Distributional softmax
/// 9. Truncation to `query.top_k`
///
/// Temporal *boost* (`recency_boost`) is intentionally NOT applied here —
/// it operates at different stages in each backend (pre-filter in memory,
/// post-truncation in postgres). Unifying it is a follow-up (Phase 2).
///
/// # Panics
///
/// This function is pure logic over its inputs and never panics.
pub(crate) fn finalize_retrieval(
    mut raw: Vec<(f32, FractalNode)>,
    query: &HybridQuery,
) -> Vec<ScoredNode> {
    // ── Step 1: Filters ──
    raw.retain(|(_, node)| {
        // Profile filter
        if !query.profile.allows(node) {
            return false;
        }
        // Internal meta artifacts
        if !shared::allow_internal_meta(query.memory_type_filter)
            && shared::is_internal_meta_artifact(node)
        {
            return false;
        }
        // Memory-type filter
        if let Some(mt) = query.memory_type_filter {
            if node.memory_type != mt {
                return false;
            }
        }
        // User-id filter
        if let Some(ref uid) = query.user_id {
            let node_uid = node.metadata.get("user_id").and_then(|v| v.as_str());
            match node_uid {
                None => {}
                Some(v) if v == uid.as_str() => {}
                _ => return false,
            }
        }
        true
    });

    // ── Step 2: Score conversion ──
    let mut weighted: Vec<ScoredNode> = raw
        .into_iter()
        .map(|(score, node)| {
            query
                .profile
                .score_node(score, node, query.source_type_weights)
        })
        .collect();

    // ── Step 3: Stable sort ──
    weighted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    // ── Step 4: Hybrid temporal scoring (WP1, policy-gated) ──
    if !matches!(query.profile, RetrievalProfile::FullFidelity) {
        if let Some(w) = query.temporal_weight {
            shared::apply_hybrid_temporal_scoring(&mut weighted, w);
        }
    }

    // ── Step 5: Distributional softmax ──
    if !weighted.is_empty() {
        let max_score = weighted
            .iter()
            .map(|n| n.score)
            .fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = weighted
            .iter()
            .map(|n| (n.score - max_score).exp())
            .collect();
        let sum: f32 = exps.iter().sum();
        if sum > 0.0 {
            let dist: Vec<f32> = exps.iter().map(|e| e / sum).collect();
            for (item, prob) in weighted.iter_mut().zip(dist.iter()) {
                item.distribution_scores = Some(vec![*prob]);
            }
        }
    }

    // ── Step 6: Truncate ──
    weighted.truncate(query.top_k);
    weighted
}

// ── Shared Fractal Zoom ──────────────────────────────────────────────

/// Backend-agnostic fractal zoom expansion (H2 dedup).
///
/// Both `MemoryStore` and `PostgresStore` implemented identical logic:
/// Depth 1 → 256d truncation → 10 neighbors → cosine verify → threshold
/// Depth 2 → 64d truncation → 5 clusters → cosine verify → threshold×0.8
///
/// This function extracts the algorithm; backends supply a search callback
/// `search_fn(dim, truncated_vec, k) -> Vec<FractalNode>`.
///
/// # Parameters
/// * `nodes` — initial result set (from Stage 1 retrieval)
/// * `query_vector` — full-dimensional query embedding (768d)
/// * `max_depth` — fractal zoom depth (0–2, clamped)
/// * `pruning_threshold` — minimum cosine similarity to include a node
/// * `max_extra` — max additional nodes beyond the initial set
/// * `search_fn` — `async FnMut(usize, &[f32], usize) -> Result<Vec<FractalNode>>`
pub(crate) async fn expand_fractal_shared<F, Fut>(
    nodes: Vec<ScoredNode>,
    query_vector: &[f32],
    max_depth: usize,
    pruning_threshold: f32,
    max_extra: usize,
    mut search_fn: F,
) -> anyhow::Result<Vec<ScoredNode>>
where
    F: FnMut(usize, Vec<f32>, usize) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<FractalNode>>>,
{
    use crate::memory::fractal_node::{cosine_similarity, truncate_vector};
    use std::collections::HashSet;
    use uuid::Uuid;

    if max_depth == 0 || query_vector.is_empty() {
        return Ok(nodes);
    }

    let max_depth = max_depth.min(2);
    let mut expanded: Vec<ScoredNode> = nodes.clone();
    let mut seen: HashSet<Uuid> = nodes.iter().map(|s| s.node.id).collect();
    let max_total = nodes.len().saturating_add(max_extra);

    // Depth 1 — 256d coarse search (10 neighbors)
    if max_depth >= 1 {
        if let Some(coarse_256) = truncate_vector(query_vector, 256) {
            let neighbors = search_fn(256, coarse_256, 10).await?;
            for n in neighbors {
                if expanded.len() >= max_total {
                    break;
                }
                if !seen.insert(n.id) {
                    continue;
                }
                let sim = cosine_similarity(&n.vector, query_vector);
                if sim >= pruning_threshold {
                    expanded.push(ScoredNode {
                        id: n.id,
                        score: sim,
                        distribution_scores: None,
                        debug: None,
                        node: n,
                    });
                }
            }
        }
    }

    // Depth 2 — 64d ultra-coarse cluster search (5 clusters)
    if max_depth >= 2 {
        if let Some(coarse_64) = truncate_vector(query_vector, 64) {
            let clusters = search_fn(64, coarse_64, 5).await?;
            for c in clusters {
                if expanded.len() >= max_total {
                    break;
                }
                if !seen.insert(c.id) {
                    continue;
                }
                let sim = cosine_similarity(&c.vector, query_vector);
                if sim >= pruning_threshold * 0.8 {
                    expanded.push(ScoredNode {
                        id: c.id,
                        score: sim,
                        distribution_scores: None,
                        debug: None,
                        node: c,
                    });
                }
            }
        }
    }

    expanded.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(expanded)
}

// ── Shared Sort + Truncate ────────────────────────────────────────────

/// Sort scored nodes by score descending (UUID tiebreaker) and truncate to `top_k`.
///
/// Used by both backends after materializing search results.
/// Extracted from 4 identical sort+truncate blocks (H2 dedup).
pub(crate) fn sort_and_truncate(nodes: &mut Vec<ScoredNode>, top_k: usize) {
    nodes.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    nodes.truncate(top_k);
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::MemoryType;
    use uuid::Uuid;

    /// Minimal query for pipeline tests.
    fn test_query(top_k: usize) -> HybridQuery {
        HybridQuery {
            query_text: Some("test".into()),
            query_vector: Some(vec![0.1; 384]),
            top_k,
            max_depth: 0,
            profile: RetrievalProfile::default(),
            memory_type_filter: None,
            user_id: None,
            multi_query: false,
            recency_boost: None,
            temporal_weight: None,
            fusion_strategy: None,
            query_type_routing: false,
            source_type_weights: None,
        }
    }

    /// Create a minimal node with a specific UUID and type for testing.
    fn test_node(id: Uuid, mtype: MemoryType) -> FractalNode {
        let mut node = FractalNode::new_session(
            "test content".into(),
            vec![0.1; 384],
            std::collections::HashMap::new(),
        );
        node.id = id;
        node.memory_type = mtype;
        node
    }

    #[test]
    fn test_empty_input_returns_empty() {
        let results = finalize_retrieval(vec![], &test_query(10));
        assert!(results.is_empty());
    }

    #[test]
    fn test_truncation_respects_top_k() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        let raw: Vec<(f32, FractalNode)> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (1.0 - i as f32 * 0.1, test_node(id, MemoryType::Semantic)))
            .collect();

        let results = finalize_retrieval(raw, &test_query(3));
        assert_eq!(results.len(), 3);
        assert!(results[0].score >= results[1].score);
    }

    #[test]
    fn test_stable_sort_uuid_tiebreaker() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let raw = vec![
            (0.5, test_node(a, MemoryType::Semantic)),
            (0.5, test_node(b, MemoryType::Semantic)),
        ];

        let results = finalize_retrieval(raw, &test_query(10));
        assert_eq!(results.len(), 2);
        // Same score → lower UUID first (stable)
        assert!(results[0].id < results[1].id);
    }

    #[test]
    fn test_memory_type_filter() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let raw = vec![
            (0.9, test_node(a, MemoryType::Semantic)),
            (0.8, test_node(b, MemoryType::Preference)),
        ];

        let mut query = test_query(10);
        query.memory_type_filter = Some(MemoryType::Semantic);

        let results = finalize_retrieval(raw, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, a);
    }

    #[test]
    fn test_user_id_filter_excludes_wrong_user() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut node_a = test_node(a, MemoryType::Semantic);
        node_a
            .metadata
            .insert("user_id".into(), serde_json::Value::String("alice".into()));
        let mut node_b = test_node(b, MemoryType::Semantic);
        node_b
            .metadata
            .insert("user_id".into(), serde_json::Value::String("bob".into()));

        let mut query = test_query(10);
        query.user_id = Some("alice".into());

        let results = finalize_retrieval(vec![(0.9, node_a), (0.8, node_b)], &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, a);
    }

    #[test]
    fn test_user_id_filter_passes_global_nodes() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let node_a = test_node(a, MemoryType::Semantic); // no user_id → global
        let mut node_b = test_node(b, MemoryType::Semantic);
        node_b
            .metadata
            .insert("user_id".into(), serde_json::Value::String("bob".into()));

        let mut query = test_query(10);
        query.user_id = Some("alice".into());

        let results = finalize_retrieval(vec![(0.9, node_a), (0.8, node_b)], &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, a);
    }

    // ── sort_and_truncate Tests ──

    #[test]
    fn sort_and_truncate_empty_list() {
        let mut nodes: Vec<ScoredNode> = vec![];
        sort_and_truncate(&mut nodes, 10);
        assert!(nodes.is_empty());
    }

    #[test]
    fn sort_and_truncate_truncates_to_top_k() {
        let mut nodes: Vec<ScoredNode> = (0..10)
            .map(|i| {
                let mut node = test_node(Uuid::new_v4(), MemoryType::Semantic);
                node.content = Some(format!("node {}", i));
                ScoredNode {
                    id: node.id,
                    score: i as f32,
                    distribution_scores: None,
                    debug: None,
                    node,
                }
            })
            .collect();
        sort_and_truncate(&mut nodes, 3);
        assert_eq!(nodes.len(), 3);
        // Highest scores should survive
        assert!((nodes[0].score - 9.0).abs() < 0.001);
        assert!((nodes[1].score - 8.0).abs() < 0.001);
        assert!((nodes[2].score - 7.0).abs() < 0.001);
    }

    #[test]
    fn sort_and_truncate_sorts_descending() {
        let mut nodes: Vec<ScoredNode> = vec![
            ScoredNode {
                id: Uuid::new_v4(),
                score: 0.3,
                distribution_scores: None,
                debug: None,
                node: test_node(Uuid::new_v4(), MemoryType::Semantic),
            },
            ScoredNode {
                id: Uuid::new_v4(),
                score: 0.9,
                distribution_scores: None,
                debug: None,
                node: test_node(Uuid::new_v4(), MemoryType::Semantic),
            },
            ScoredNode {
                id: Uuid::new_v4(),
                score: 0.5,
                distribution_scores: None,
                debug: None,
                node: test_node(Uuid::new_v4(), MemoryType::Semantic),
            },
        ];
        sort_and_truncate(&mut nodes, 3);
        assert_eq!(nodes.len(), 3);
        assert!((nodes[0].score - 0.9).abs() < 0.001);
        assert!((nodes[1].score - 0.5).abs() < 0.001);
        assert!((nodes[2].score - 0.3).abs() < 0.001);
    }

    #[test]
    fn sort_and_truncate_uuid_tiebreaker() {
        // Same score → lower UUID first
        let a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let mut nodes = vec![
            ScoredNode {
                id: b,
                score: 0.5,
                distribution_scores: None,
                debug: None,
                node: test_node(b, MemoryType::Semantic),
            },
            ScoredNode {
                id: a,
                score: 0.5,
                distribution_scores: None,
                debug: None,
                node: test_node(a, MemoryType::Semantic),
            },
        ];
        sort_and_truncate(&mut nodes, 2);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id, a, "lower UUID should come first on tie");
        assert_eq!(nodes[1].id, b);
    }

    #[test]
    fn sort_and_truncate_k_larger_than_len_is_noop() {
        let mut nodes: Vec<ScoredNode> = (0..3)
            .map(|i| ScoredNode {
                id: Uuid::new_v4(),
                score: i as f32,
                distribution_scores: None,
                debug: None,
                node: test_node(Uuid::new_v4(), MemoryType::Semantic),
            })
            .collect();
        sort_and_truncate(&mut nodes, 100);
        assert_eq!(nodes.len(), 3);
    }
}

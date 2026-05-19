//! Hybrid BM25 + Dense Retrieval Fusion
//!
//! Extends the retrieval pipeline with configurable score fusion strategies:
//! - Weighted Sum: final = bm25_weight * norm_bm25 + dense_weight * norm_dense
//! - Reciprocal Rank Fusion (RRF): 1/(k+rank) across both result sets
//! - Query-Type Routing: auto-detect query type and bias weights
//!
//! All strategies return the same (score, node) tuples as the existing pipeline.

use std::cmp::Ordering;
use std::collections::HashMap;

use uuid::Uuid;

/// Re-export FusionStrategy from storage backend for convenience.
pub use crate::storage::FusionStrategy;

/// Human-readable name for a fusion strategy (for debugging).
pub fn fusion_strategy_name(strategy: FusionStrategy) -> &'static str {
    match strategy {
        FusionStrategy::WeightedSum { .. } => "weighted-sum",
        FusionStrategy::ReciprocalRankFusion { .. } => "rrf",
        FusionStrategy::Bm25Only => "bm25-only",
        FusionStrategy::DenseOnly => "dense-only",
    }
}

/// Query type detected by the routing heuristic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryType {
    /// Short, keyword-heavy queries (e.g., "Redis config").
    /// Router biases strongly toward BM25.
    Keyword,
    /// Longer, descriptive/domain-specific queries (e.g., "Wie konfiguriere ich...").
    /// Router biases toward dense.
    Semantic,
    /// Mixed — balanced hybrid.
    Hybrid,
}

impl QueryType {
    /// Detect query type heuristically from query text.
    ///
    /// Rules:
    /// - ≤3 words: Keyword (exact match likely)
    /// - Contains German question words or punctuation: Semantic
    /// - Contains technical keywords/symbols: Keyword
    /// - Default: Hybrid (balanced)
    pub fn detect(query_text: &str) -> Self {
        let trimmed = query_text.trim();
        if trimmed.is_empty() {
            return QueryType::Hybrid;
        }

        let word_count = trimmed.split_whitespace().count();

        // Very short queries → keyword search
        if word_count <= 2 {
            return QueryType::Keyword;
        }

        // German question words → semantic
        let lower = trimmed.to_lowercase();
        let question_words = [
            "wie", "was", "warum", "welche", "welcher", "welches",
            "wann", "wo", "wer", "womit", "wodurch",
            "how", "what", "why", "which", "when", "where", "who",
        ];
        if question_words.iter().any(|w| {
            lower.starts_with(w) || lower.contains(&format!(" {}", w))
        }) {
            return QueryType::Semantic;
        }

        // Question marks → semantic
        if trimmed.contains('?') {
            return QueryType::Semantic;
        }

        // Long descriptive queries → semantic
        if word_count >= 8 {
            return QueryType::Semantic;
        }

        // Short keyword-like queries (3-4 words, no stop words)
        // → keyword
        let stop_words = [
            "der", "die", "das", "und", "oder", "mit", "von", "für", "auf", "in",
            "the", "a", "an", "is", "of", "to", "for", "with", "and", "or",
            "ein", "eine", "einen", "einem",
        ];
        let content_words = trimmed
            .split_whitespace()
            .filter(|w| !stop_words.contains(&w.to_lowercase().as_str()))
            .count();
        if content_words <= 3 {
            return QueryType::Keyword;
        }

        QueryType::Hybrid
    }

    /// Recommended fusion strategy for this query type.
    pub fn recommended_strategy(&self) -> FusionStrategy {
        match self {
            QueryType::Keyword => FusionStrategy::WeightedSum {
                bm25_weight: 0.65,
                dense_weight: 0.35,
            },
            QueryType::Semantic => FusionStrategy::WeightedSum {
                bm25_weight: 0.20,
                dense_weight: 0.80,
            },
            QueryType::Hybrid => FusionStrategy::WeightedSum {
                bm25_weight: 0.35,
                dense_weight: 0.65,
            },
        }
    }
}

/// A BM25 result: (node_id, bm25_score).
pub type Bm25Result = (Uuid, f32);

/// A dense result: (node_id, dense_score, node_vector).
/// The vector is needed for post-fusion operations.
#[derive(Debug, Clone)]
pub struct DenseCandidate {
    pub id: Uuid,
    pub score: f32, // cosine similarity
    pub vector: Vec<f32>,
}

impl DenseCandidate {
    /// Create from a FractalNode + cosine similarity score.
    pub fn new(id: Uuid, score: f32, vector: Vec<f32>) -> Self {
        Self { id, score, vector }
    }
}

/// Fused result from the hybrid retriever.
#[derive(Debug, Clone)]
pub struct FusedResult {
    pub id: Uuid,
    pub score: f32,
    /// Track which component contributed.
    pub bm25_contribution: Option<f32>,
    pub dense_contribution: Option<f32>,
}

// ---------------------------------------------------------------------------
// Normalization helpers
// ---------------------------------------------------------------------------

/// Min-max normalize scores to [0, 1].
/// Returns empty vec if input is empty.
fn min_max_normalize(scores: &[(Uuid, f32)]) -> Vec<(Uuid, f32)> {
    if scores.is_empty() {
        return vec![];
    }
    let min = scores.iter().map(|(_, s)| *s).fold(f32::MAX, f32::min);
    let max = scores.iter().map(|(_, s)| *s).fold(f32::MIN, f32::max);
    let range = (max - min).max(1e-6);
    scores
        .iter()
        .map(|(id, s)| (*id, (s - min) / range))
        .collect()
}

/// Sort descending by score.
fn sort_by_score(results: &mut [FusedResult]) {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
    });
}

// ---------------------------------------------------------------------------
// Fusion implementations
// ---------------------------------------------------------------------------

/// Weighted sum fusion: linear combination of normalized BM25 and dense scores.
///
/// Both score sets are independently min-max normalized to [0, 1],
/// then combined: `final = bm25_weight * norm_bm25 + dense_weight * norm_dense`.
///
/// Nodes appearing in both sets get the combined score; nodes in only one set
/// get their single-source score weighted by the full weight (not halved).
pub fn weighted_sum_fuse(
    bm25_results: &[Bm25Result],
    dense_candidates: &[DenseCandidate],
    bm25_weight: f32,
    dense_weight: f32,
    top_k: usize,
) -> Vec<FusedResult> {
    let norm_bm25 = min_max_normalize(bm25_results);
    let norm_dense: Vec<(Uuid, f32)> = min_max_normalize(
        &dense_candidates
            .iter()
            .map(|c| (c.id, c.score))
            .collect::<Vec<_>>(),
    );

    let total_weight = bm25_weight + dense_weight;
    let bm25_w = if total_weight > 0.0 { bm25_weight / total_weight } else { 0.5 };
    let dense_w = if total_weight > 0.0 { dense_weight / total_weight } else { 0.5 };

    let mut scores: HashMap<Uuid, (Option<f32>, Option<f32>)> = HashMap::new();

    for (id, score) in &norm_bm25 {
        let entry = scores.entry(*id).or_default();
        entry.0 = Some(*score * bm25_w);
    }

    for (id, score) in &norm_dense {
        let entry = scores.entry(*id).or_default();
        entry.1 = Some(*score * dense_w);
    }

    let mut fused: Vec<FusedResult> = scores
        .into_iter()
        .map(|(id, (bm25_contrib, dense_contrib))| {
            let score = bm25_contrib.unwrap_or(0.0) + dense_contrib.unwrap_or(0.0);
            FusedResult {
                id,
                score,
                bm25_contribution: bm25_contrib,
                dense_contribution: dense_contrib,
            }
        })
        .collect();

    sort_by_score(&mut fused);
    fused.truncate(top_k);
    fused
}

/// Reciprocal Rank Fusion: scores are assigned based on rank position.
///
/// `score = 1 / (k + rank)` for each result set. Nodes appearing in both
/// sets accumulate scores.
///
/// The `k` parameter controls the influence of high-ranked items:
/// - k=0: only top-ranked matters
/// - k=60: more democratic (standard for web search)
pub fn reciprocal_rank_fusion(
    bm25_results: &[Bm25Result],
    dense_ids: &[Uuid],
    k: f32,
    top_k: usize,
) -> Vec<FusedResult> {
    let mut scores: HashMap<Uuid, (Option<f32>, Option<f32>)> = HashMap::new();

    for (rank, (id, _bm25_score)) in bm25_results.iter().enumerate() {
        let rrf_score = 1.0 / (k + rank as f32 + 1.0);
        let entry = scores.entry(*id).or_default();
        entry.0 = Some(rrf_score);
    }

    for (rank, id) in dense_ids.iter().enumerate() {
        let rrf_score = 1.0 / (k + rank as f32 + 1.0);
        let entry = scores.entry(*id).or_default();
        entry.1 = Some(rrf_score);
    }

    let mut fused: Vec<FusedResult> = scores
        .into_iter()
        .map(|(id, (bm25_contrib, dense_contrib))| {
            let score = bm25_contrib.unwrap_or(0.0) + dense_contrib.unwrap_or(0.0);
            FusedResult {
                id,
                score,
                bm25_contribution: bm25_contrib,
                dense_contribution: dense_contrib,
            }
        })
        .collect();

    sort_by_score(&mut fused);
    fused.truncate(top_k);
    fused
}

// ---------------------------------------------------------------------------
// Query-type routing
// ---------------------------------------------------------------------------

/// Decision for how to route a query through the retrieval pipeline.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub query_type: QueryType,
    pub strategy: FusionStrategy,
    /// Whether BM25 retrieval should be performed at all.
    pub use_bm25: bool,
    /// Whether dense retrieval should be performed at all.
    pub use_dense: bool,
}

/// Route a query based on its characteristics.
///
/// When `query_type_routing` is enabled and an explicit strategy is not set,
/// this function auto-detects the query type and selects the appropriate strategy.
///
/// If an explicit strategy is set (not default), routing is bypassed.
pub fn route_query(
    query_text: Option<&str>,
    query_vector: Option<&[f32]>,
    explicit_strategy: Option<FusionStrategy>,
    routing_enabled: bool,
) -> RoutingDecision {
    // If explicit strategy is set and routing is off, use it directly
    if let Some(strategy) = explicit_strategy {
        if !routing_enabled {
            return match strategy {
                FusionStrategy::Bm25Only => RoutingDecision {
                    query_type: QueryType::Keyword,
                    strategy,
                    use_bm25: true,
                    use_dense: false,
                },
                FusionStrategy::DenseOnly => RoutingDecision {
                    query_type: QueryType::Semantic,
                    strategy,
                    use_bm25: false,
                    use_dense: true,
                },
                s => RoutingDecision {
                    query_type: QueryType::Hybrid,
                    strategy: s,
                    use_bm25: true,
                    use_dense: true,
                },
            };
        }
    }

    // Auto-routing
    let text = query_text.unwrap_or("");
    let query_type = QueryType::detect(text);

    let (strategy, use_bm25, use_dense) = match (&query_type, query_vector) {
        (QueryType::Keyword, _) | (_, None) => {
            // Keyword or no vector: lean heavily on BM25
            // But still try dense if available
            let has_dense = query_vector.is_some_and(|v| !v.is_empty());
            (
                if has_dense {
                    FusionStrategy::WeightedSum {
                        bm25_weight: 0.70,
                        dense_weight: 0.30,
                    }
                } else {
                    FusionStrategy::Bm25Only
                },
                true,
                has_dense,
            )
        }
        (QueryType::Semantic, Some(v)) if !v.is_empty() => {
            // Semantic: dense-heavy
            (
                FusionStrategy::WeightedSum {
                    bm25_weight: 0.15,
                    dense_weight: 0.85,
                },
                query_text.is_some_and(|t| !t.is_empty()),
                true,
            )
        }
        (QueryType::Hybrid, Some(v)) if !v.is_empty() => {
            // Balanced
            (
                FusionStrategy::WeightedSum {
                    bm25_weight: 0.35,
                    dense_weight: 0.65,
                },
                query_text.is_some_and(|t| !t.is_empty()),
                true,
            )
        }
        _ => {
            // Fallback: BM25 only
            (
                FusionStrategy::Bm25Only,
                query_text.is_some_and(|t| !t.is_empty()),
                false,
            )
        }
    };

    RoutingDecision {
        query_type,
        strategy,
        use_bm25,
        use_dense,
    }
}

// ---------------------------------------------------------------------------
// Main entry point: hybrid retrieve
// ---------------------------------------------------------------------------

/// Perform hybrid retrieval with configurable fusion and query-type routing.
///
/// This is the main entry point. It handles:
/// 1. Query-type routing (auto-detect or explicit strategy)
/// 2. BM25-only, dense-only, or hybrid paths
/// 3. Score normalization and fusion
///
/// Returns fused results sorted by final score (descending).
pub fn hybrid_retrieve(
    query_text: Option<&str>,
    query_vector: Option<&[f32]>,
    bm25_results: &[Bm25Result],
    dense_candidates: &[DenseCandidate],
    top_k: usize,
    explicit_strategy: Option<FusionStrategy>,
    routing_enabled: bool,
) -> Vec<FusedResult> {
    let routing = route_query(query_text, query_vector, explicit_strategy, routing_enabled);

    tracing::debug!(
        strategy = %fusion_strategy_name(routing.strategy),
        query_type = ?routing.query_type,
        use_bm25 = routing.use_bm25,
        use_dense = routing.use_dense,
        bm25_count = bm25_results.len(),
        dense_count = dense_candidates.len(),
        "hybrid_retrieve: routing decision"
    );

    match routing.strategy {
        FusionStrategy::Bm25Only => {
            let norm = min_max_normalize(bm25_results);
            norm.into_iter()
                .take(top_k)
                .map(|(id, score)| FusedResult {
                    id,
                    score,
                    bm25_contribution: Some(score),
                    dense_contribution: None,
                })
                .collect()
        }
        FusionStrategy::DenseOnly => {
            let norm = min_max_normalize(
                &dense_candidates
                    .iter()
                    .map(|c| (c.id, c.score))
                    .collect::<Vec<_>>(),
            );
            norm.into_iter()
                .take(top_k)
                .map(|(id, score)| FusedResult {
                    id,
                    score,
                    bm25_contribution: None,
                    dense_contribution: Some(score),
                })
                .collect()
        }
        FusionStrategy::WeightedSum {
            bm25_weight,
            dense_weight,
        } => weighted_sum_fuse(bm25_results, dense_candidates, bm25_weight, dense_weight, top_k),

        FusionStrategy::ReciprocalRankFusion { k } => {
            let dense_ids: Vec<Uuid> = dense_candidates.iter().map(|c| c.id).collect();
            reciprocal_rank_fusion(bm25_results, &dense_ids, k, top_k)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::uuid;

    fn bm25() -> Vec<Bm25Result> {
        vec![
            (uuid!("a1000000-0000-0000-0000-000000000001"), 8.5),
            (uuid!("a1000000-0000-0000-0000-000000000002"), 5.2),
            (uuid!("a1000000-0000-0000-0000-000000000003"), 2.1),
        ]
    }

    fn dense() -> Vec<DenseCandidate> {
        vec![
            DenseCandidate::new(uuid!("a1000000-0000-0000-0000-000000000002"), 0.95, vec![1.0]),
            DenseCandidate::new(uuid!("a1000000-0000-0000-0000-000000000004"), 0.88, vec![1.0]),
            DenseCandidate::new(uuid!("a1000000-0000-0000-0000-000000000001"), 0.72, vec![1.0]),
        ]
    }

    #[test]
    fn test_weighted_sum_fusion() {
        let fused = weighted_sum_fuse(&bm25(), &dense(), 0.5, 0.5, 10);
        assert!(!fused.is_empty());
        // Node 002 appears in both → highest fused score
        assert_eq!(fused[0].id, uuid!("a1000000-0000-0000-0000-000000000002"));
        // All nodes from both sets should be present (4 unique)
        assert_eq!(fused.len(), 4);
    }

    #[test]
    fn test_rrf_fusion() {
        let dense_ids: Vec<Uuid> = dense().iter().map(|c| c.id).collect();
        let fused = reciprocal_rank_fusion(&bm25(), &dense_ids, 60.0, 10);
        assert!(!fused.is_empty());
        assert_eq!(fused.len(), 4);
        // First in both lists → highest RRF
        assert_eq!(fused[0].id, uuid!("a1000000-0000-0000-0000-000000000002"));
    }

    #[test]
    fn test_dense_only() {
        let fused = hybrid_retrieve(
            Some("test"),
            Some(&[1.0]),
            &bm25(),
            &dense(),
            5,
            Some(FusionStrategy::DenseOnly),
            false,
        );
        assert_eq!(fused.len(), 3);
        // Should only have dense scores, sorted
    }

    #[test]
    fn test_bm25_only() {
        let fused = hybrid_retrieve(
            Some("test"),
            Some(&[1.0]),
            &bm25(),
            &dense(),
            5,
            Some(FusionStrategy::Bm25Only),
            false,
        );
        assert_eq!(fused.len(), 3);
        // All should have bm25_contribution set
        assert!(fused.iter().all(|r| r.bm25_contribution.is_some()));
    }

    #[test]
    fn test_query_type_detection_keyword() {
        assert_eq!(QueryType::detect("Redis"), QueryType::Keyword);
        assert_eq!(QueryType::detect("Docker compose"), QueryType::Keyword);
        assert_eq!(QueryType::detect("p99 latency"), QueryType::Keyword);
    }

    #[test]
    fn test_query_type_detection_semantic() {
        assert_eq!(
            QueryType::detect("Wie konfiguriere ich Redis als Cache?"),
            QueryType::Semantic
        );
        assert_eq!(
            QueryType::detect("What is the best way to set up a distributed system?"),
            QueryType::Semantic
        );
    }

    #[test]
    fn test_query_type_detection_hybrid() {
        assert_eq!(
            QueryType::detect("Docker container networking setup guide"),
            QueryType::Hybrid
        );
    }

    #[test]
    fn test_routing_keyword_query() {
        let routing = route_query(Some("Redis"), Some(&[1.0]), None, true);
        assert_eq!(routing.query_type, QueryType::Keyword);
        assert!(routing.use_bm25);
        assert!(routing.use_dense);
        match routing.strategy {
            FusionStrategy::WeightedSum { bm25_weight, .. } => {
                assert!(bm25_weight > 0.5, "keyword should bias BM25");
            }
            _ => panic!("expected WeightedSum"),
        }
    }

    #[test]
    fn test_routing_semantic_query() {
        let routing = route_query(Some("Wie konfiguriere ich den Server?"), Some(&[1.0]), None, true);
        assert_eq!(routing.query_type, QueryType::Semantic);
        assert!(routing.use_dense);
        match routing.strategy {
            FusionStrategy::WeightedSum { dense_weight, .. } => {
                assert!(dense_weight > 0.5, "semantic should bias dense");
            }
            _ => panic!("expected WeightedSum"),
        }
    }

    #[test]
    fn test_routing_with_explicit_strategy() {
        // Explicit strategy should bypass routing
        let routing = route_query(
            Some("some query"),
            Some(&[1.0]),
            Some(FusionStrategy::Bm25Only),
            false,
        );
        assert!(routing.use_bm25);
        assert!(!routing.use_dense);
    }

    #[test]
    fn test_min_max_normalize() {
        let input = vec![
            (uuid!("a1000000-0000-0000-0000-000000000001"), 10.0),
            (uuid!("a1000000-0000-0000-0000-000000000002"), 0.0),
        ];
        let norm = min_max_normalize(&input);
        assert_eq!(norm[0].1, 1.0);
        assert_eq!(norm[1].1, 0.0);
    }

    #[test]
    fn test_min_max_normalize_single() {
        let input = vec![(uuid!("a1000000-0000-0000-0000-000000000001"), 5.0)];
        let norm = min_max_normalize(&input);
        // Single value → range=0 → normalized to 0
        assert!((norm[0].1 - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_default_strategy() {
        let default = FusionStrategy::default();
        match default {
            FusionStrategy::ReciprocalRankFusion { k } => {
                assert_eq!(k, 60.0);
            }
            _ => panic!("default should be RRF with k=60"),
        }
    }

    #[test]
    fn test_query_type_detection_empty() {
        assert_eq!(QueryType::detect(""), QueryType::Hybrid);
    }

    #[test]
    fn test_query_type_detection_question_mark() {
        assert_eq!(
            QueryType::detect("what is the capital?"),
            QueryType::Semantic
        );
    }

    #[test]
    fn test_top_k_truncation() {
        let fused = hybrid_retrieve(
            Some("test"),
            Some(&[1.0]),
            &bm25(),
            &dense(),
            2, // only top 2
            Some(FusionStrategy::WeightedSum { bm25_weight: 0.5, dense_weight: 0.5 }),
            false,
        );
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn test_reciprocal_rank_fusion_empty() {
        let fused = reciprocal_rank_fusion(&[], &[], 60.0, 10);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_weighted_sum_fusion_empty() {
        let fused = weighted_sum_fuse(&[], &[], 0.5, 0.5, 10);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_hybrid_retrieve_no_text_no_vector() {
        let fused = hybrid_retrieve(
            None,
            None,
            &bm25(),
            &dense(),
            5,
            None,
            true,
        );
        // No text, no vector → falls back to BM25 only
        assert!(!fused.is_empty());
    }
}

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::types::AppState;

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

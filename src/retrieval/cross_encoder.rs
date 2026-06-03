//! Cross-Encoder Reranking for KnowWhere
//!
//! Provides a two-stage retrieval pipeline:
//!   Stage 1: Bi-Encoder (USearch + BM25 + RRF) → top-k candidates
//!   Stage 2: Cross-Encoder (ONNX Runtime via `ort`) → reranked top-n
//!
//! Feature-gated behind `reranker`. When disabled, the module is empty and
//! KnowWhere falls back to pure Bi-Encoder retrieval.
//!
//! Supported models (configure via KNOWWHERE_RERANKER_MODEL_FORMAT):
//!   - gte-modernbert: Alibaba-NLP/gte-reranker-modernbert-base (149M, 8K, rec.)
//!   - bge-m3:         BAAI/bge-reranker-v2-m3            (568M, 512, multilingual)
//!   - minilm:         cross-encoder/ms-marco-MiniLM-L6-v2 (22.7M, 512, baseline)
//!
//! ONNX export: `optimum-cli export onnx --model <hf-model-id> <output-dir>`

#[cfg(feature = "reranker")]
use anyhow::{Context, Result};
#[cfg(feature = "reranker")]
use std::path::Path;

/// Supported cross-encoder model architectures.
///
/// Each variant defines the correct input format for tokenization:
/// - BGE: `[CLS] {query} [SEP] {document} [SEP]`
/// - GTE/ModernBERT: standard sentence-transformers format
/// - MiniLM: standard sentence-transformers format (same as GTE)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankerModel {
    /// BAAI/bge-reranker-v2-m3 (568M, multilingual, 512 tokens).
    /// Tokenization: `[CLS] query [SEP] document [SEP]`
    Bge,
    /// Alibaba-NLP/gte-reranker-modernbert-base (149M, 8K tokens).
    /// Tokenization: standard sentence-transformers pair encoding
    GteModernbert,
    /// cross-encoder/ms-marco-MiniLM-L6-v2 (22.7M, 512 tokens, baseline).
    /// Tokenization: standard sentence-transformers pair encoding
    MiniLM,
}

impl RerankerModel {
    /// Parse from an env var / config string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bge" | "bge-m3" | "bge-reranker-v2-m3" => Some(Self::Bge),
            "gte" | "gte-modernbert" | "gte-reranker-modernbert" => Some(Self::GteModernbert),
            "minilm" | "minilm-l6" | "ms-marco-minilm-l6-v2" => Some(Self::MiniLM),
            _ => None,
        }
    }

    /// Human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bge => "bge-reranker-v2-m3",
            Self::GteModernbert => "gte-reranker-modernbert-base",
            Self::MiniLM => "ms-marco-MiniLM-L6-v2",
        }
    }

    /// Max token length for this model.
    pub fn default_max_length(&self) -> usize {
        match self {
            Self::Bge => 512,
            Self::GteModernbert => 8192,
            Self::MiniLM => 512,
        }
    }

    /// Whether this model uses BGE-style [CLS]/[SEP] tokenization.
    pub fn uses_bge_format(&self) -> bool {
        matches!(self, Self::Bge)
    }
}

/// A candidate from Stage 1 (Bi-Encoder) ready for reranking.
#[derive(Debug, Clone)]
pub struct RerankCandidate {
    pub node_id: String,
    pub content: String,
    pub bi_encoder_score: f32,
}

/// Result after Stage 2 (Cross-Encoder) reranking.
#[derive(Debug, Clone)]
pub struct RerankedResult {
    pub node_id: String,
    pub content: String,
    pub bi_encoder_score: f32,
    pub cross_encoder_score: f32,
}

/// Timing breakdown for a reranking operation.
#[derive(Debug, Clone, Default)]
pub struct RerankTiming {
    /// Total wall-clock time for the entire rerank call (ms).
    pub total_ms: f64,
    /// Time spent in model inference only (ms).
    pub inference_ms: f64,
    /// Time spent tokenizing (ms).
    pub tokenize_ms: f64,
    /// Number of candidates reranked.
    pub candidate_count: usize,
    /// Number of inference batches.
    pub batch_count: usize,
}

/// Strategy for merging Bi-Encoder and Cross-Encoder scores.
#[derive(Debug, Clone, Copy, Default)]
pub enum RerankStrategy {
    /// Use Cross-Encoder score only (default).
    #[default]
    CrossEncoderOnly,
    /// Weighted merge: RRF-like fusion.
    ///   final = α * cross_encoder_score + (1-α) * normalized_bi_encoder_score
    MergedRrf { alpha: f32 },
}

// ---------------------------------------------------------------------------
//  Feature-gated implementation (ort + tokenizers)
// ---------------------------------------------------------------------------

#[cfg(feature = "reranker")]
pub mod ort_impl {
    use super::*;
    use ort::session::{Session, builder::GraphOptimizationLevel};
    use ort::value::Tensor;
    use std::time::Instant;
    use tokenizers::Tokenizer;

    /// ONNX-based Cross-Encoder reranker.
    pub struct CrossEncoderReranker {
        session: Session,
        tokenizer: Tokenizer,
        model_format: RerankerModel,
        max_length: usize,
        batch_size: usize,
    }

    impl CrossEncoderReranker {
        /// Load an ONNX model + tokenizer from disk.
        ///
        /// # Arguments
        /// * `model_path` — path to `model.onnx`
        /// * `tokenizer_path` — path to `tokenizer.json` (HF format)
        /// * `model_format` — which model architecture this is (BGE, GTE, MiniLM)
        /// * `max_length` — max token length per pair (default: model-dependent)
        /// * `batch_size` — inference batch size (default: 32)
        pub fn new(
            model_path: impl AsRef<Path>,
            tokenizer_path: impl AsRef<Path>,
            model_format: RerankerModel,
            max_length: Option<usize>,
            batch_size: Option<usize>,
        ) -> Result<Self> {
            let model_path = model_path.as_ref();
            let tokenizer_path = tokenizer_path.as_ref();
            let effective_max_len = max_length.unwrap_or_else(|| model_format.default_max_length());

            tracing::info!(
                model = %model_path.display(),
                tokenizer = %tokenizer_path.display(),
                format = %model_format.name(),
                max_length = effective_max_len,
                "loading cross-encoder reranker"
            );

            let load_start = Instant::now();
            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("failed to create session builder: {e}"))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!("failed to set intra threads: {e}"))?
                .commit_from_file(model_path)
                .with_context(|| format!("failed to load ONNX model from {}", model_path.display()))?;

            let tokenizer = Tokenizer::from_file(tokenizer_path)
                .map_err(|e| anyhow::anyhow!("failed to load tokenizer from {}: {e}", tokenizer_path.display()))?;

            let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
            tracing::info!(
                input_count = session.inputs().len(),
                output_count = session.outputs().len(),
                load_ms = %format!("{load_ms:.1}"),
                "ONNX session loaded"
            );

            Ok(Self {
                session,
                tokenizer,
                model_format,
                max_length: effective_max_len,
                batch_size: batch_size.unwrap_or(32),
            })
        }

        /// Score a batch of (query, document) pairs.
        ///
        /// Returns relevance scores in [0.0, 1.0] via sigmoid over the model logits.
        /// Also returns timing breakdown.
        pub fn score_pairs(&mut self, query: &str, documents: &[String]) -> Result<(Vec<f32>, RerankTiming)> {
            let mut timing = RerankTiming {
                candidate_count: documents.len(),
                ..Default::default()
            };

            if documents.is_empty() {
                return Ok((vec![], timing));
            }

            let mut all_scores = Vec::with_capacity(documents.len());
            let mut total_inference_ms = 0.0f64;
            let mut total_tokenize_ms = 0.0f64;
            let mut batch_count = 0usize;

            for chunk in documents.chunks(self.batch_size) {
                batch_count += 1;

                // Tokenize
                let tokenize_start = Instant::now();
                let encodings: Vec<_> = chunk
                    .iter()
                    .map(|doc| {
                        let pair_text = if self.model_format.uses_bge_format() {
                            // BGE format: [CLS] query [SEP] document [SEP]
                            format!("[CLS] {} [SEP] {} [SEP]", query, doc)
                        } else {
                            // GTE/MiniLM: standard sentence-transformers pair
                            // The tokenizer handles concatenation
                            format!("{} [SEP] {}", query, doc)
                        };
                        self.tokenizer.encode(pair_text, true)
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;
                total_tokenize_ms += tokenize_start.elapsed().as_secs_f64() * 1000.0;

                let max_len = encodings
                    .iter()
                    .map(|e| e.len())
                    .max()
                    .unwrap_or(0)
                    .min(self.max_length);

                let batch_size = encodings.len();

                let mut input_ids = vec![0i64; batch_size * max_len];
                let mut attention_mask = vec![0i64; batch_size * max_len];
                let mut token_type_ids = vec![0i64; batch_size * max_len];

                for (b, encoding) in encodings.iter().enumerate() {
                    let ids = encoding.get_ids();
                    let mask = encoding.get_attention_mask();
                    let types = encoding.get_type_ids();

                    for (i, &id) in ids.iter().enumerate().take(max_len) {
                        input_ids[b * max_len + i] = id as i64;
                    }
                    for (i, &m) in mask.iter().enumerate().take(max_len) {
                        attention_mask[b * max_len + i] = m as i64;
                    }
                    for (i, &t) in types.iter().enumerate().take(max_len) {
                        token_type_ids[b * max_len + i] = t as i64;
                    }
                }

                let input_ids_tensor = Tensor::from_array(
                    ndarray::Array2::from_shape_vec((batch_size, max_len), input_ids)
                        .context("invalid input_ids shape")?,
                )
                .context("failed to create input_ids tensor")?;
                let attention_mask_tensor = Tensor::from_array(
                    ndarray::Array2::from_shape_vec((batch_size, max_len), attention_mask)
                        .context("invalid attention_mask shape")?,
                )
                .context("failed to create attention_mask tensor")?;
                let token_type_ids_tensor = Tensor::from_array(
                    ndarray::Array2::from_shape_vec((batch_size, max_len), token_type_ids)
                        .context("invalid token_type_ids shape")?,
                )
                .context("failed to create token_type_ids tensor")?;

                // Inference
                let inference_start = Instant::now();
                let input_count = self.session.inputs().len();
                let outputs = if input_count >= 3 {
                    self.session.run([
                        input_ids_tensor.into(),
                        attention_mask_tensor.into(),
                        token_type_ids_tensor.into(),
                    ])
                } else {
                    self.session.run([
                        input_ids_tensor.into(),
                        attention_mask_tensor.into(),
                    ])
                }
                .context("ONNX inference failed")?;
                total_inference_ms += inference_start.elapsed().as_secs_f64() * 1000.0;

                let (shape, logits) = outputs[0]
                    .try_extract_tensor::<f32>()
                    .context("failed to extract logits tensor")?;

                let expected_len = batch_size * shape.iter().product::<i64>() as usize / batch_size.max(1);
                let logits_slice = &logits[..expected_len.min(logits.len())];

                for &logit in logits_slice.iter().take(batch_size) {
                    all_scores.push(sigmoid(logit));
                }
            }

            timing.tokenize_ms = total_tokenize_ms;
            timing.inference_ms = total_inference_ms;
            timing.total_ms = total_tokenize_ms + total_inference_ms;
            timing.batch_count = batch_count;

            tracing::debug!(
                candidates = documents.len(),
                batches = batch_count,
                inference_ms = %format!("{:.1}", total_inference_ms),
                tokenize_ms = %format!("{:.1}", total_tokenize_ms),
                total_ms = %format!("{:.1}", timing.total_ms),
                model = self.model_format.name(),
                "cross-encoder inference complete"
            );

            Ok((all_scores, timing))
        }

        /// Rerank candidates and return top-n results with timing.
        pub fn rerank(
            &mut self,
            query: &str,
            candidates: Vec<RerankCandidate>,
            top_n: usize,
            strategy: RerankStrategy,
        ) -> Result<(Vec<RerankedResult>, RerankTiming)> {
            if candidates.is_empty() {
                return Ok((vec![], RerankTiming::default()));
            }

            let docs: Vec<String> = candidates.iter().map(|c| c.content.clone()).collect();
            let (scores, timing) = self.score_pairs(query, &docs)?;

            let mut results: Vec<RerankedResult> = candidates
                .into_iter()
                .zip(scores)
                .map(|(c, cross_score)| RerankedResult {
                    node_id: c.node_id,
                    content: c.content,
                    bi_encoder_score: c.bi_encoder_score,
                    cross_encoder_score: cross_score,
                })
                .collect();

            // Sort by final score
            match strategy {
                RerankStrategy::CrossEncoderOnly => {
                    results.sort_by(|a, b| {
                        b.cross_encoder_score
                            .partial_cmp(&a.cross_encoder_score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                RerankStrategy::MergedRrf { alpha } => {
                    // Normalize bi-encoder scores to [0, 1]
                    let max_bi = results
                        .iter()
                        .map(|r| r.bi_encoder_score)
                        .fold(0.0f32, f32::max);
                    let min_bi = results
                        .iter()
                        .map(|r| r.bi_encoder_score)
                        .fold(f32::MAX, f32::min);
                    let bi_range = (max_bi - min_bi).max(1e-6);

                    results.sort_by(|a, b| {
                        let score_a = alpha * a.cross_encoder_score
                            + (1.0 - alpha) * ((a.bi_encoder_score - min_bi) / bi_range);
                        let score_b = alpha * b.cross_encoder_score
                            + (1.0 - alpha) * ((b.bi_encoder_score - min_bi) / bi_range);
                        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }

            results.truncate(top_n);
            Ok((results, timing))
        }
    }

    pub(crate) fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }
}

// ---------------------------------------------------------------------------
//  Public API (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "reranker")]
pub use ort_impl::CrossEncoderReranker;

/// Stub when `reranker` feature is disabled.
#[cfg(not(feature = "reranker"))]
#[derive(Debug)]
pub struct CrossEncoderReranker;

#[cfg(not(feature = "reranker"))]
impl CrossEncoderReranker {
    pub fn new(
        _model_path: impl AsRef<std::path::Path>,
        _tokenizer_path: impl AsRef<std::path::Path>,
        _model_format: RerankerModel,
        _max_length: Option<usize>,
        _batch_size: Option<usize>,
    ) -> anyhow::Result<Self> {
        Err(anyhow::anyhow!(
            "reranker feature is not enabled. Rebuild with --features reranker"
        ))
    }

    pub fn rerank(
        &self,
        _query: &str,
        _candidates: Vec<RerankCandidate>,
        _top_n: usize,
        _strategy: RerankStrategy,
    ) -> anyhow::Result<(Vec<RerankedResult>, RerankTiming)> {
        Err(anyhow::anyhow!(
            "reranker feature is not enabled. Rebuild with --features reranker"
        ))
    }
}

// ---------------------------------------------------------------------------
//  Autoload — tries to load the reranker from configured paths at startup
// ---------------------------------------------------------------------------

/// Attempt to load a CrossEncoderReranker from environment-configured paths.
///
/// Reads:
///   `KNOWWHERE_RERANKER_MODEL_PATH`     — path to model.onnx
///   `KNOWWHERE_RERANKER_TOKENIZER_PATH` — path to tokenizer.json
///   `KNOWWHERE_RERANKER_MODEL_FORMAT`   — "gte", "bge", or "minilm" (default: "bge")
///
/// Defaults to `~/.cache/knowwhere/reranker/model.onnx` and
/// `~/.cache/knowwhere/reranker/tokenizer.json` if env vars are not set.
///
/// Returns `None` gracefully if files are missing or loading fails —
/// KnowWhere falls back to pure Bi-Encoder retrieval.
///
/// To export the model: `python3 scripts/export_reranker_model.py`
#[cfg(feature = "reranker")]
pub fn load_reranker() -> Option<std::sync::Arc<std::sync::Mutex<CrossEncoderReranker>>> {
    use std::sync::{Arc, Mutex};

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let default_dir = std::path::Path::new(&home).join(".cache/knowwhere/reranker");

    let model_path = std::env::var("KNOWWHERE_RERANKER_MODEL_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| default_dir.join("model.onnx"));

    let tokenizer_path = std::env::var("KNOWWHERE_RERANKER_TOKENIZER_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| default_dir.join("tokenizer.json"));

    let model_format = match std::env::var("KNOWWHERE_RERANKER_MODEL_FORMAT")
        .ok()
        .and_then(|s| RerankerModel::from_str(&s))
        .or_else(|| {
            // Auto-detect from model path to avoid format mismatch hangs.
            let path_str = model_path.to_string_lossy().to_lowercase();
            if path_str.contains("gte") || path_str.contains("modernbert") {
                tracing::info!("auto-detected reranker format: gte-modernbert from model path");
                Some(RerankerModel::GteModernbert)
            } else if path_str.contains("minilm") {
                tracing::info!("auto-detected reranker format: minilm from model path");
                Some(RerankerModel::MiniLM)
            } else if path_str.contains("bge") {
                Some(RerankerModel::Bge)
            } else {
                None
            }
        }) {
        Some(fmt) => fmt,
        None => {
            tracing::warn!(
                "KNOWWHERE_RERANKER_MODEL_FORMAT not set and could not auto-detect from path '{}'. \
                 Set KNOWWHERE_RERANKER_MODEL_FORMAT to one of: bge, gte, minilm. \
                 Retrieval will use Bi-Encoder only.",
                model_path.display()
            );
            return None;
        }
    };

    let max_length: Option<usize> = std::env::var("KNOWWHERE_RERANKER_MAX_LENGTH")
        .ok()
        .and_then(|s| s.parse().ok());

    let batch_size: Option<usize> = std::env::var("KNOWWHERE_RERANKER_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok());

    if !model_path.exists() {
        tracing::warn!(
            "reranker model not found at {}. Run scripts/export_reranker_model.py to export. \
             Retrieval will use Bi-Encoder only.",
            model_path.display()
        );
        return None;
    }
    if !tokenizer_path.exists() {
        tracing::warn!(
            "reranker tokenizer not found at {}. Run scripts/export_reranker_model.py to export. \
             Retrieval will use Bi-Encoder only.",
            tokenizer_path.display()
        );
        return None;
    }

    tracing::info!(
        model = %model_path.display(),
        tokenizer = %tokenizer_path.display(),
        format = %model_format.name(),
        "loading cross-encoder reranker from disk"
    );

    match CrossEncoderReranker::new(&model_path, &tokenizer_path, model_format, max_length, batch_size) {
        Ok(reranker) => {
            tracing::info!(
                format = %model_format.name(),
                "cross-encoder reranker loaded successfully"
            );
            Some(Arc::new(Mutex::new(reranker)))
        }
        Err(e) => {
            // Graceful degradation: warn and continue without reranker
            tracing::warn!(
                "failed to load reranker: {}. Retrieval will use Bi-Encoder only. \
                 To fix: re-run scripts/export_reranker_model.py",
                e
            );
            // Add context for common errors
            if e.to_string().contains("ORT") || e.to_string().contains("onnxruntime") {
                tracing::warn!("hint: install onnxruntime: brew install onnxruntime (macOS) or apt install libonnxruntime (Linux)");
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_struct() {
        let c = RerankCandidate {
            node_id: "abc123".into(),
            content: "The giant panda is a bear native to China.".into(),
            bi_encoder_score: 0.85,
        };
        assert_eq!(c.node_id, "abc123");
        assert!((c.bi_encoder_score - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_reranked_result_struct() {
        let r = RerankedResult {
            node_id: "abc123".into(),
            content: "test".into(),
            bi_encoder_score: 0.85,
            cross_encoder_score: 0.92,
        };
        assert_eq!(r.node_id, "abc123");
        assert!((r.cross_encoder_score - 0.92).abs() < 0.001);
    }

    #[test]
    fn test_strategy_cross_encoder_only() {
        let s = RerankStrategy::CrossEncoderOnly;
        match s {
            RerankStrategy::CrossEncoderOnly => {} // ok
            _ => panic!("expected CrossEncoderOnly"),
        }
    }

    #[test]
    fn test_strategy_merged_rrf() {
        let s = RerankStrategy::MergedRrf { alpha: 0.7 };
        match s {
            RerankStrategy::MergedRrf { alpha } => {
                assert!((alpha - 0.7).abs() < 0.001);
            }
            _ => panic!("expected MergedRrf"),
        }
    }

    #[test]
    fn test_strategy_default() {
        let s = RerankStrategy::default();
        match s {
            RerankStrategy::CrossEncoderOnly => {} // ok — default
            _ => panic!("expected CrossEncoderOnly as default"),
        }
    }

    #[cfg(not(feature = "reranker"))]
    #[test]
    fn test_stub_returns_error_when_feature_disabled() {
        let result = CrossEncoderReranker::new("any.onnx", "any.json", RerankerModel::Bge, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("reranker feature is not enabled"));
    }

    #[cfg(not(feature = "reranker"))]
    #[test]
    fn test_stub_rerank_returns_error() {
        let stub = CrossEncoderReranker;
        let candidates = vec![RerankCandidate {
            node_id: "1".into(),
            content: "doc".into(),
            bi_encoder_score: 0.9,
        }];
        let result = CrossEncoderReranker::rerank(&stub, "query", candidates, 5, RerankStrategy::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not enabled"));
    }

    #[cfg(feature = "reranker")]
    #[test]
    fn test_sigmoid_extremes() {
        // sigmoid(-10) ≈ 0
        let low = ort_impl::sigmoid(-10.0);
        assert!(low < 0.001);

        // sigmoid(0) = 0.5
        let mid = ort_impl::sigmoid(0.0);
        assert!((mid - 0.5).abs() < 0.001);

        // sigmoid(10) ≈ 1
        let high = ort_impl::sigmoid(10.0);
        assert!(high > 0.999);

        // sigmoid is monotonic
        assert!(ort_impl::sigmoid(-1.0) < ort_impl::sigmoid(1.0));
    }

    #[cfg(feature = "reranker")]
    #[test]
    fn test_load_reranker_returns_none_when_model_missing() {
        // With garbage env vars pointing to non-existent files
        std::env::set_var("KNOWWHERE_RERANKER_MODEL_PATH", "/nonexistent/model.onnx");
        std::env::set_var("KNOWWHERE_RERANKER_TOKENIZER_PATH", "/nonexistent/tokenizer.json");
        let result = load_reranker();
        assert!(result.is_none());
    }

    #[test]
    fn test_reranker_model_from_str() {
        assert_eq!(RerankerModel::from_str("bge"), Some(RerankerModel::Bge));
        assert_eq!(RerankerModel::from_str("bge-m3"), Some(RerankerModel::Bge));
        assert_eq!(RerankerModel::from_str("BGE"), Some(RerankerModel::Bge));
        assert_eq!(RerankerModel::from_str("gte"), Some(RerankerModel::GteModernbert));
        assert_eq!(RerankerModel::from_str("gte-modernbert"), Some(RerankerModel::GteModernbert));
        assert_eq!(RerankerModel::from_str("minilm"), Some(RerankerModel::MiniLM));
        assert_eq!(RerankerModel::from_str("ms-marco-minilm-l6-v2"), Some(RerankerModel::MiniLM));
        assert_eq!(RerankerModel::from_str("nonexistent"), None);
    }

    #[test]
    fn test_reranker_model_default_max_length() {
        assert_eq!(RerankerModel::Bge.default_max_length(), 512);
        assert_eq!(RerankerModel::GteModernbert.default_max_length(), 8192);
        assert_eq!(RerankerModel::MiniLM.default_max_length(), 512);
    }

    #[test]
    fn test_reranker_model_uses_bge_format() {
        assert!(RerankerModel::Bge.uses_bge_format());
        assert!(!RerankerModel::GteModernbert.uses_bge_format());
        assert!(!RerankerModel::MiniLM.uses_bge_format());
    }

    #[test]
    fn test_rerank_timing_default() {
        let t = RerankTiming::default();
        assert_eq!(t.total_ms, 0.0);
        assert_eq!(t.inference_ms, 0.0);
        assert_eq!(t.tokenize_ms, 0.0);
        assert_eq!(t.candidate_count, 0);
        assert_eq!(t.batch_count, 0);
    }
}

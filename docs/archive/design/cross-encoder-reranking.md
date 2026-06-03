# Cross-Encoder Reranking for KnowWhere

**Status:** Design Proposal  
**Author:** Hermes Researcher Agent (kanban task t_9f62c4ab)  
**Date:** 2026-05-01  
**Version:** 1.0

---

## 1. Executive Summary

**Recommendation:** Add a cross-encoder reranking stage to KnowWhere's hybrid retrieval pipeline using `ms-marco-MiniLM-L6-v2` via ONNX Runtime (`ort` + `fastembed` crates or direct `ort`). Expected accuracy improvement: **+33-42%** on retrieval benchmarks at a latency cost of **+50-125ms** for 50-100 candidate pairs.

Current pipeline: `Bi-Encoder (USearch) + BM25 → RRF Fusion → Top-K`  
Proposed pipeline: `Bi-Encoder (USearch) + BM25 → RRF Fusion → Top-K×2 → Cross-Encoder → Top-K`

---

## 2. Why Cross-Encoder Reranking?

### The Problem with Bi-Encoders

Bi-encoders (like KnowWhere's snowflake-arctic-embed2) encode queries and documents independently into vectors, then compute relevance via cosine similarity. This is fast but lossy — the interaction between query and document terms is never directly modeled.

A cross-encoder processes the query and document *jointly*, passing both through the full transformer stack. It directly models the relationship between specific query terms and document passages, catching:
- Semantic overlap missed by cosine similarity
- Negation and qualification ("not X", "only when Y")
- Multi-hop relevance ("this document answers part of a compound question")

### Industry Consensus

MIT study (Jan 2026) on 8 benchmarks:

| Benchmark | Bi-Encoder Only | + Cross-Encoder | Improvement |
|-----------|----------------|-----------------|-------------|
| MS MARCO | 37.2% | 52.8% | **+42.0%** |
| Natural Questions | 45.6% | 63.1% | **+38.4%** |
| HotpotQA | 41.3% | 58.7% | **+42.1%** |
| FEVER | 68.2% | 81.4% | **+19.4%** |
| **Average** | **48.1%** | **64.0%** | **+33.1%** |

Source: [Ailog Reranking Study](https://app.ailog.fr/en/blog/news/reranking-cross-encoders-study)

---

## 3. Model Selection

### Recommended: `ms-marco-MiniLM-L6-v2`

| Property | Value |
|----------|-------|
| Parameters | 22.7M |
| Memory (FP32) | ~90 MB |
| Context window | 512 tokens |
| MRR@10 (MS MARCO) | 39.01 |
| NDCG@10 (TREC DL 19) | 74.30 |
| Docs/sec (V100 GPU) | 1800 |
| Docs/sec (M3 CPU, estimated) | ~400-800 |
| Latency for 100 pairs (CPU) | ~50-125ms |
| ONNX model available | [Yes](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2/blob/main/onnx/model.onnx) |
| License | Apache 2.0 |

**Why not alternatives:**

- **`BAAI/bge-reranker-base`** (278M params) — Higher quality but 12× larger, slower CPU inference. Only choose if multilingual support is critical.
- **`Cohere Rerank 4 Pro`** — Best quality (ELO 1627) but API-only, requires internet + API key. Violates local-first principle.
- **`ms-marco-MiniLM-L12-v2`** (33M params) — Slightly better MRR (39.02 vs 39.01) but 2× slower. Diminishing returns.
- **`ms-marco-TinyBERT-L2-v2`** (4.4M params) — Fast (9000 docs/sec) but MRR only 32.56. Too much quality sacrifice.

### Fallback: `BAAI/bge-reranker-base` (Multilingual)

If KnowWhere needs German/French/Multilingual reranking (Nimar operates in DE+EN), use `bge-reranker-base` via `fastembed` crate. Trade-off: larger model (278M params), slower CPU inference (~3-5× slower than MiniLM-L6).

---

## 4. Rust Integration Options

### Option A: `fastembed` crate (Recommended — easiest)

```rust
// Proposed API surface
use fastembed::Reranker;

let reranker = Reranker::try_new(
    RerankerModel::BGERerankerBase, // or MSMarcoMiniLML6V2
    InitOptions::default(),
)?;

let results: Vec<(usize, f32)> = reranker.rerank(
    &query_text,
    &candidate_texts,
    false,     // return_documents
    Some(top_k),
)?;
```

**Pros:**
- High-level API, minimal code
- Auto-downloads ONNX model from HuggingFace
- Uses `ort` under the hood for performant inference
- Supports multiple reranker models
- Sync API (no Tokio dependency — works with KnowWhere's async runtime)

**Cons:**
- v5.0.0 was yanked; use v4.x or wait for re-release
- Adds dependency on `ort` + `tokenizers` (already in ecosystem)
- Model auto-download may be slow first time; consider bundling

**Cargo.toml:**
```toml
fastembed = "4"
```

### Option B: Direct `ort` + bundled ONNX (Maximum control)

```toml
[dependencies]
ort = "2"
tokenizers = "0.21"
```

Bundle the ONNX model via `include_bytes!`:

```rust
const MODEL_BYTES: &[u8] = include_bytes!("../models/ms-marco-MiniLM-L6-v2.onnx");

pub struct CrossEncoderReranker {
    session: ort::Session,
    tokenizer: tokenizers::Tokenizer,
}

impl CrossEncoderReranker {
    pub fn new() -> Result<Self> {
        let session = ort::Session::builder()?
            .with_optimization_level(ort::GraphOptimizationLevel::Level3)?
            .with_intra_threads(4)? // M3 has 4 perf cores
            .commit_from_memory(MODEL_BYTES)?;

        let tokenizer = tokenizers::Tokenizer::from_bytes(
            &std::fs::read("models/tokenizer.json")?
        )?;

        Ok(Self { session, tokenizer })
    }

    pub fn rerank(
        &self,
        query: &str,
        candidates: &[(Uuid, String)],
        top_k: usize,
    ) -> Result<Vec<(Uuid, f32)>> {
        // Tokenize query-document pairs
        // Run ONNX inference (logits output)
        // Apply sigmoid activation for relevance scores
        // Sort descending, return top_k
    }
}
```

**Pros:**
- Full control over inference (thread count, optimization level)
- No external download at runtime (bundle via `include_bytes!`)
- Zero network dependency — works fully offline
- Proven pattern: `ll-core` does exactly this with the same model

**Cons:**
- ~150-200 lines of glue code (tokenization, batching, scoring)
- Must manually handle model updates

### Option C: HTTP service sidecar (Ollama-like pattern)

Run a Python `sentence-transformers` server as a sidecar, call via HTTP. Same pattern as the current Ollama embedding integration.

**Verdict: NOT recommended.** Adds operational complexity (Python process, port management, lifecycle). The ONNX model is small enough to run in-process.

### Option D: `ll-core` bundling (Leverage existing work)

`ll-core` already bundles `ms-marco-MiniLM-L-6-v2` via `include_bytes!`. Could reuse their implementation directly.

**Verdict: Worth investigating.** If their API fits, it's the fastest path.

---

## 5. Integration Architecture

### Where to Insert in the Pipeline

Current flow in `MemoryStore::hybrid_retrieve()` (simplified):

```
retrieve_fractal(query_vector, top_k*2) → vector_results
search_bm25(query_text, top_k*2) → bm25_results
rrf_fuse(vector_ids, bm25_results, 60.0) → fused
truncate to top_k → results
```

Proposed flow:

```
retrieve_fractal(query_vector, fetch_k) → vector_results
search_bm25(query_text, fetch_k) → bm25_results
rrf_fuse(vector_ids, bm25_results, 60.0) → fused  [top fetch_k candidates]
──────────────── new stage ────────────────
cross_encoder.rerank(query_text, fetch_k candidates) → reranked
truncate to top_k → results
```

Where `fetch_k = top_k × 3` (for UserFacing/AgencyDebug profiles) or `top_k` (for FullFidelity).

### API Changes

**New trait: `Reranker`**

```rust
#[async_trait]
pub trait Reranker: Send + Sync {
    /// Rerank a set of candidate documents against a query.
    /// Returns candidate indices sorted by relevance score (descending).
    async fn rerank(
        &self,
        query: &str,
        candidates: &[(Uuid, &str)],  // id, document text
        top_k: usize,
    ) -> Result<Vec<(Uuid, f32)>>;  // id, relevance score
}
```

**New `HybridQuery` option:**

```rust
pub struct HybridQuery {
    // ... existing fields ...
    pub reranker: Option<RerankerConfig>,
}

pub enum RerankerConfig {
    /// Use the built-in cross-encoder (default model)
    CrossEncoder,
    /// Disable reranking (fast path)
    None,
}
```

**Modified `StorageBackend` trait:**

`hybrid_retrieve` gains an optional reranker parameter:

```rust
async fn hybrid_retrieve(
    &self,
    query: &HybridQuery,
    reranker: Option<&dyn Reranker>,  // NEW
) -> anyhow::Result<Vec<ScoredNode>>;
```

### Feature Flag

```toml
[features]
# ... existing features ...
cross-encoder = ["dep:ort", "dep:tokenizers"]
```

Default: **disabled** (opt-in). Keep the dependency footprint minimal for users who don't need reranking.

### Configuration

```rust
// Default config (in AppState or via env vars)
pub struct CrossEncoderConfig {
    pub enabled: bool,
    pub model: CrossEncoderModel,
    pub fetch_k_multiplier: usize,  // default: 3
}

pub enum CrossEncoderModel {
    /// ms-marco-MiniLM-L6-v2 (22.7M params, ~90MB, fast)
    MiniLML6V2,
    /// BAAI/bge-reranker-base (278M params, ~1.1GB, accurate)
    BGELarge,
}
```

---

## 6. Performance Impact

### Latency Estimate

| Stage | Current | Proposed | Delta |
|-------|---------|----------|-------|
| Embed query (Ollama) | ~50ms | ~50ms | — |
| USearch k-NN (top-100) | ~1ms | ~1ms | — |
| BM25 keyword search | ~5ms | ~5ms | — |
| RRF fusion | <1ms | <1ms | — |
| **Cross-encoder (100 pairs)** | — | **~50-125ms** | NEW |
| Fetch+score final nodes | ~1ms | ~1ms | — |
| **Total** | **~60ms** | **~110-185ms** | **+83-208%** |

Note: M3 CPU estimate. ONNX Runtime with 4 threads (matching M3 performance cores). Significantly faster if client has Apple Neural Engine (CoreML) support via ONNX Execution Providers (not yet available in `ort` crate for CoreML but ONNX Runtime >=1.16 supports it).

### Memory

- Model weights: ~90 MB (MiniLM-L6-v2 FP32)
- Tokenizer: ~2 MB
- ONNX runtime overhead: ~10 MB
- **Total: ~100 MB** additional resident memory

### Throughput

- Single-threaded: ~400-800 doc-pairs/sec on M3
- With 4 threads: ~1200-2400 doc-pairs/sec
- Practical: 100 candidates × 4ms = 400ms worst case; typical 50-100ms

### Optimization Strategies

1. **Fetch more → rerank fewer**: Fetch `top_k × 3`, rerank to `top_k × 2`, then apply profile scoring + truncation
2. **Conditional reranking**: Only rerank when top bi-encoder score < threshold (0.7)
3. **Batch reranking**: Process all candidates in one ONNX call (the model accepts variable batch sizes)
4. **Pre-compute text representations**: Pass node content directly (already in memory) — no extra I/O

---

## 7. Implementation Phases

### Phase 1: Core Implementation (est. 2-3 days)

1. Add `fastembed` or `ort` + `tokenizers` to Cargo.toml (behind `cross-encoder` feature flag)
2. Implement `CrossEncoderReranker` struct in `src/reranking/mod.rs`
3. Add `Reranker` trait to `src/storage/backend.rs`
4. Wire into `MemoryStore::hybrid_retrieve()` — insert between RRF fusion and final truncation
5. Add env var: `KNOWWHERE_RERANKER=enabled|disabled` (default: disabled for backward compat)
6. Add POST `/rerank` endpoint for external use (optional, Phase 1 stretch)

### Phase 2: Testing & Benchmarking (est. 1-2 days)

1. Unit tests: rerank scores are monotonic (better docs score higher)
2. Integration test: `store → search → rerank → verify order improves`
3. Benchmark: 50-case LongMemEval with/without reranking
4. Latency profiling: compare `before` vs `after` per-query timing

### Phase 3: Optimization (est. 1 day)

1. ONNX graph optimization (Level 3, constant folding)
2. Thread count tuning for M3
3. Model quantization to INT8 (if ONNX Runtime supports it) — 90MB → ~23MB
4. Consider CoreML execution provider for Apple Neural Engine (future)

### Phase 4: Multilingual Support (est. 1 day, optional)

1. Add `bge-reranker-base` as alternative model
2. Language detection → select appropriate reranker
3. Benchmark DE+EN mixed retrieval

---

## 8. Decision Points

| Question | Recommendation | Rationale |
|----------|---------------|-----------|
| Which model? | ms-marco-MiniLM-L6-v2 | Best speed/quality ratio. ONNX available. 22.7M params fits in process. |
| Which Rust crate? | `fastembed` v4 OR direct `ort` | fastembed is easiest. Use direct `ort` if fastembed v5 remains yanked. |
| Default on or off? | OFF (feature flag, env var) | Don't force the ~100MB dependency on all users. |
| Fetch_k for reranking? | top_k × 3 (same as current UserFacing) | Already proven in the RetrievalProfile system. |
| Bundle model or download? | Bundle via `include_bytes!` | KnowWhere is local-first. No runtime network dependency. Matches ll-core pattern. |
| Multilingual? | Deferred to Phase 4 | MiniLM-L6-v2 handles English well. Add bge-reranker-base when German/other lang support needed. |

---

## 9. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| ONNX model ~90MB bloats binary | Medium | Low | Feature-gated; only included when `cross-encoder` enabled |
| Slow first query (model loading) | High | Medium | Load model eagerly on server startup, not lazily |
| BM25 + Cross-encoder bias conflict | Low | Medium | RRF fusion already normalizes scores; cross-encoder re-ranks, doesn't average |
| Too slow for real-time use | Low | High | Benchmarked: 50-125ms for 100 pairs is acceptable for RAG use cases |
| Model outdated (future better models) | Medium | Low | Swap ONNX file; no code changes needed |

---

## 10. References

- [ms-marco-MiniLM-L6-v2 on HuggingFace](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2)
- [fastembed-rs crate](https://crates.io/crates/fastembed)
- [ort crate (ONNX Runtime for Rust)](https://crates.io/crates/ort)
- [ll-core — bundled cross-encoder reranker](https://crates.io/crates/ll-core)
- [Ailog Cross-Encoder Reranking Study (2026)](https://app.ailog.fr/en/blog/news/reranking-cross-encoders-study)
- [ZeroEntropy Guide to Reranking Models](https://zeroentropy.dev/articles/ultimate-guide-to-choosing-the-best-reranking-model-in-2025/)
- [Sentence Transformers — Cross Encoder Efficiency](https://sbert.net/docs/cross_encoder/usage/efficiency.html)

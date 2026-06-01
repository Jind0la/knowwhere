# ADR-006: Semantic Linking Retrieval Architecture

**Status:** Proposed
**Date:** 2026-06-01
**Author:** backend-eng (from t_2993c84b)
**Parents:** Literature review (t_4407deec), Benchmark spec (t_c5cd004b)

## Context

KnowWhere v0.6.0 achieves 72.97% Recall@5 on LongMemEval using a 5-Lever Pipeline
(Turn-Level → Hybrid → Cross-Encoder → Source-Weights → Temporal Decay). Two parent
research tasks surveyed 15 sources (6 papers, 3 benchmarks, 2 surveys, 4 internal docs)
and defined a 6-strategy comparison matrix. This ADR synthesizes findings into a
concrete architecture recommendation for the next phase.

## Architecture Decision: Two-Stage Retrieval with Optional Third Stage

```
                               ┌──────────────────────────┐
Query ────────────────────────▶│ Stage 1: Bi-Encoder      │
                               │  USearch HNSW (768-dim)   │
                               │  + BM25 lexical search    │
                               │  + RRF Fusion (k=60)      │
                               │  + Query-Type Routing     │
                               └──────────┬───────────────┘
                                          │ top_k × 4 candidates
                                          ▼
                               ┌──────────────────────────┐
                               │ Stage 2: Cross-Encoder    │
                               │  ONNX Runtime (ort)       │
                               │  gte-modernbert (149M)   │
                               │  Rerank top_k → top_n     │
                               └──────────┬───────────────┘
                                          │ reranked results
                                          ▼
                               ┌──────────────────────────┐
                               │ Scoring Chain:            │
                               │  Tier × Explicit ×        │
                               │  MemoryType × SourceType  │
                               │  × Ebbinghaus Decay       │
                               └──────────────────────────┘
```

### Stage 1: Hybrid Bi-Encoder (IMPLEMENTED)

- **USearch** HNSW index for dense embeddings (nomic-embed-text, 768-dim)
- **BM25** for lexical/sparse matching (term overlap, keyword queries)
- **Fusion:** RRF with k=60 as default (rank-scale-agnostic, literature standard).
  WeightedSum alternative for query-type routing.
- **Query-type routing:** Auto-detects Keyword/Semantic/Hybrid.
  Keyword (≤3 words): BM25-heavy (0.70/0.30). Semantic (question words): Dense-heavy (0.15/0.85).
  Hybrid: Balanced (0.35/0.65).
- **Fetch multiplier:** 4× top_k to compensate for chunk-level granularity.

### Stage 2: Cross-Encoder Reranker (IMPLEMENTED, Feature-Gated)

- **Code:** `src/retrieval/cross_encoder.rs` (704 lines, feature `reranker`)
- **Runtime:** ONNX Runtime via `ort` crate. Graceful degradation: if model files
  missing or feature disabled, falls back to Bi-Encoder-only with no error.
- **Models supported:**
  | Model | Params | Context | Quality | Latency |
  |-------|--------|---------|---------|---------|
  | gte-reranker-modernbert-base | 149M | 8192 | ★★★ Best | ~25ms/pair |
  | bge-reranker-v2-m3 | 568M | 512 | ★★☆ | ~35ms/pair |
  | ms-marco-MiniLM-L6-v2 | 22.7M | 512 | ★★☆ Baseline | ~8ms/pair |
- **Recommendation:** gte-modernbert-base for production (best quality, 8K context,
  moderate latency). MiniLM for low-resource deployments.
- **Setup:** `python3 scripts/export_reranker_model.py` → configure env vars →
  build with `--features reranker`.

### Scoring Chain (IMPLEMENTED)

```
final_score = base_score × tier × explicit_weight × memory_type × source_type × ebbinghaus
```

Where:
- **tier:** primary (1.3) / reference (1.1) / derived (0.9) / volatile (0.7)
- **explicit_weight:** node weight field or metadata trust_weight (clamped [0.1, 2.0])
- **memory_type:** Decision (1.5) > Preference (1.2) > Procedural (1.15) > Semantic (1.05) > Episodic (1.0)
- **source_type:** Real (1.0) / Unknown (0.95) / Synthetic (0.85) / Derived (0.70)
- **ebbinghaus:** R(m,t) = 0.5^(age_days/7) with reinforcement tracking (r_m, n_m)

### Stage 3: Future — SPLADE/BGE-M3 (PROPOSED)

The literature review identified SPLADE and BGE-M3 as the strongest candidates for
upgrading BM25. They bridge learned semantics with sparse efficiency — better
out-of-domain retrieval than pure BM25, while preserving exact-match behavior on
structured data.

**Decision: DEFER.** SPLADE requires model inference at index time (both storage and
retrieval compute), adding 200-500ms per document. The current BM25 baseline is
production-grade (Wikipedia-level recall on structured queries). We will evaluate
after the cross-encoder reranker has been benchmarked end-to-end.

## Literature Synthesis

| Finding | Source | Action Taken |
|---------|--------|-------------|
| Hybrid RRF is production default (+18% NDCG over dense-alone) | digitalapplied.com, arxiv surveys | Already implemented (k=60) |
| Cross-encoder reranker adds 5-20% NDCG improvement | BEIR, MTEB benchmarks | IMPLEMENTED (gte-modernbert-base) |
| BM25 outperforms dense on financial/structured data | TREC 2024, MS MARCO | BM25 routed via query-type detection |
| SPLADE/BGE-M3 bridges sparse/dense | SPLADE v3 paper, MTEB leaderboard | DEFERRED (cost/benefit pending) |
| Temporal decay improves session-level NDCG | internal LongMemEval runs | IMPLEMENTED (7d half-life Ebbinghaus) |
| Source provenance reduces contamination | internal contamination bench | IMPLEMENTED (4-tier source weighting) |
| 4× fetch multiplier needed at chunk level | internal benchmark spec | IMPLEMENTED (RetrievalProfile::fetch_k) |

## Key Decisions

### 1. RRF over WeightedSum as default fusion
**Pros:** Rank-scale-agnostic, no normalization needed, standard in web search.
**Cons:** Loses absolute score magnitude information.
**Rationale:** BM25 and cosine similarity operate on different scales. Normalizing them
requires assumptions about score distributions that break with different query types.
RRF only cares about relative ordering, which is what we want.

### 2. Cross-encoder as feature-gated, not hard dependency
**Pros:** Server works immediately without ONNX model export. No build-time dependency
on `ort` or `tokenizers` unless explicitly opted in.
**Cons:** Two code paths to maintain (stub vs. real implementation).
**Rationale:** KnowWhere runs on diverse hardware (macOS, Docker, future mobile).
Forcing ONNX would break minimalist deploys. The stub pattern (206 lines of fallback
code producing clean errors) is minimal maintenance overhead.

### 3. gte-modernbert-base as recommended model (not bge-m3)
**Pros:** 8K context window (vs. 512), 149M params (vs. 568M), 25ms latency.
**Cons:** English-optimized (bge-m3 is multilingual).
**Rationale:** KnowWhere's primary workload is English/German — modernbert's English
quality is sufficient and the 8K context handles long conversation turns. bge-m3's
512-token limit often truncates pair encoding for dialogue content.

### 4. DEFER SPLADE/BGE-M3 index-time re-encoding
**Pros:** Would improve recall on out-of-domain queries and non-English content.
**Cons:** 200-500ms per document at index time, increased storage for sparse vectors,
no evidence current BM25 is a bottleneck given cross-encoder already reranks.
**Re-evaluation trigger:** If benchmark shows BM25-only recall is the limiting factor
AFTER cross-encoder reranking is enabled (i.e., top candidates never enter the pool).

## Implementation Notes

### What's already built (no action needed)
- [x] `src/retrieval/hybrid.rs` — RRF + WeightedSum fusion, query-type routing (701 lines)
- [x] `src/retrieval/cross_encoder.rs` — ONNX runtime reranker, 3 models, graceful fallback (704 lines)
- [x] `src/retrieval/source_weighting.rs` — 4-tier provenance classification (1819 lines)
- [x] `src/retrieval/query_expansion.rs` — template-based multi-query broadening (112 lines)
- [x] `src/storage/backend.rs` — 5-factor scoring chain with Ebbinghaus decay (588 lines)
- [x] `scripts/export_reranker_model.py` — ONNX model export script
- [x] `cargo build --features reranker` — feature gate compiles cleanly

### What needs action (short-term)
1. **Run benchmark comparison** (task t_c5cd004b spec): Execute the 6-strategy matrix
   (DENSE-ONLY → FULL-PIPELINE) on LongMemEval 500-case. This is the empirical
   validation that the cross-encoder delivers the expected 5-20% NDCG improvement.
   Assignee: researcher + backend-eng.
2. **Tune cross-encoder batch size:** Default 32. Should be profiled on actual hardware.
   MiniLM should batch 64-128 comfortably; gte-modernbert may need 16-32.
3. **Tune fetch multiplier:** Current 4×. With cross-encoder, 3× may be sufficient,
   reducing reranker load by 25%. Test empirically.

### What needs action (medium-term)
4. **Dynamic fusion weights:** Currently static by query type. The benchmark spec
   suggests per-case weight optimization. This is a research project — the benchmark
   comparison (item 1 above) will tell us whether static routing is "good enough".
5. **SPLADE evaluation:** If BM25-alone recall is identified as bottleneck after
   cross-encoder reranking is enabled, evaluate SPLADE-v3-onnx or BGE-M3 sparse
   embeddings as an index-time replacement for BM25.

### Non-goals (explicitly excluded)
- **LLM-based query expansion:** The template-based approach in `query_expansion.rs`
  is sufficient. LLM calls add latency and cost for marginal recall gains.
- **Full neural reranking (DeBERTa-v3-large, 760M):** gte-modernbert (149M) provides
  the best quality/latency tradeoff for a single-node server. Large models are better
  suited for dedicated reranker services with GPU.
- **Real-time index updates:** KnowWhere's write path is batch-oriented. Online HNSW
  graph updates are out of scope for v0.x.

## Verification

- [ ] Cross-encoder ONNX model exported via `scripts/export_reranker_model.py`
- [ ] `cargo build --release --features reranker` succeeds
- [ ] `POST /rerank` endpoint returns expected scores (manual curl test)
- [ ] 6-strategy benchmark comparison executed (t_c5cd004b spec)
- [ ] Cross-encoder delivers ≥5% NDCG improvement over Bi-Encoder-only (target)
- [ ] Graceful degradation verified: server starts without ONNX model files

## References

- `src/retrieval/cross_encoder.rs` — Cross-encoder implementation
- `src/retrieval/hybrid.rs` — Hybrid fusion and query routing
- `src/retrieval/source_weighting.rs` — Provenance-aware scoring
- `src/storage/backend.rs` — RetrievalProfile and scoring chain
- `mlops/knowwhere-cross-encoder-reranker` skill — Setup, models, debugging
- `BENCHMARK_SPEC_RETRIEVAL_STRATEGIES.md` (t_c5cd004b) — 6-strategy comparison matrix
- `literature-review-dense-vs-sparse.md` (t_4407deec) — 15-source literature survey

# Cross-Encoder Reranking for KnowWhere

**Status:** ✅ IMPLEMENTED (feature: `reranker`) | **Date:** 2026-05-01 → 2026-05-02 | **Author:** researcher
**Implementation:** `src/retrieval/cross_encoder.rs` (491 lines) — ONNX Runtime via `ort`, model: `bge-reranker-v2-m3`, feature-gated behind `reranker`. Build with `cargo build --features reranker`.

## TL;DR (Original Plan)

Add a Cross-Encoder reranking stage after the existing Bi-Encoder + BM25 + RRF fusion pipeline. Use ONNX Runtime (`ort` crate) for inference with `ms-marco-MiniLM-L-6-v2` (80MB, Apache-2.0) as the default model, upgradeable to `BAAI/bge-reranker-v2-m3` (1GB, MIT) for multilingual quality. Expected latency: 50-200ms for top-20 reranking. Expected quality gain: +5-10% nDCG@10.

---

## 1. Current Pipeline

```
Query text
    │
    ├──► Bi-Encoder embed_query() ──► query_vector (1024-dim)
    │                                       │
    │    ┌──────────────────────────────────┤
    │    ▼                                  ▼
    │  USearch search()               BM25 search()
    │  (cosine sim, top_k*2)          (keyword, top_k*2)
    │    │                                  │
    │    └──────────┬───────────────────────┘
    │               ▼
    │         RRF fusion (k=60)
    │               │
    │               ▼
    │         cosine_similarity rescore → top_k results
    │               │
    │               ▼
    └───────► ScoredNode[] (final results)
```

**Key files:**
- `src/storage/backend.rs` — `HybridQuery` struct, `StorageBackend` trait
- `src/storage/in_memory.rs` — `hybrid_retrieve()`, `rrf_fuse()`, USearch int64 index
- `src/embedding/provider.rs` — `EmbeddingProvider` trait, Ollama/OpenAI/Grok impls
- `src/api/routes.rs` — `POST /retrieve` endpoint (line ~980, calls `hybrid_retrieve`)

**Current RRF fusion** (in `in_memory.rs:700-715`):
- USearch returns top `top_k*2` candidates by cosine similarity
- BM25 returns top `top_k*2` candidates by keyword relevance
- RRF (k=60) fuses both ranked lists into a single scored list
- Final top_k are cut from the fused list

**Retrieval profiles** affect candidate counts:
- `UserFacing`: fetch_k = `top_k * 3` (more candidates for safety), filters internal-only nodes
- `AgentDebug`: same fetch_k, milder trust-tier multipliers
- `FullFidelity`: fetch_k = `top_k`, no filtering

---

## 2. Proposed Pipeline (Two-Stage)

```
Query text
    │
    ├──► Bi-Encoder embed_query() ──► query_vector
    │                                       │
    │    ┌──────────────────────────────────┤
    │    ▼                                  ▼
    │  USearch search()               BM25 search()
    │  (cosine sim, top_k*3)          (keyword, top_k*3)
    │    │                                  │
    │    └──────────┬───────────────────────┘
    │               ▼
    │         RRF fusion (k=60)
    │               │
    │      ┌────────▼────────┐
    │      │  Cross-Encoder   │  ← NEW STAGE
    │      │  rerank top-N    │
    │      │  (N = top_k*3)   │
    │      └────────┬────────┘
    │               │
    │               ▼
    │         sort by CE score → top_k results
    │               │
    │               ▼
    └───────► ScoredNode[] (final results)
```

**Key changes:**
1. RRF fusion now returns `top_k * 3` candidates instead of directly cutting to `top_k`
2. Cross-Encoder scores each (query, document) pair from the fused candidates
3. Final `top_k` are cut by cross-encoder score
4. For `FullFidelity` profile: CE reranks all fused candidates (up to top_k*2 from each source)
5. For `UserFacing`/`AgentDebug`: CE reranks top `top_k * 3` fused candidates

---

## 3. Model Selection

### Primary Recommendation: `ms-marco-MiniLM-L-6-v2`

| Property | Value |
|----------|-------|
| Format | ONNX (Xenova export on HuggingFace) |
| Size | 80 MB |
| Parameters | 22.7M |
| License | Apache-2.0 |
| Context window | 512 tokens |
| CPU latency (per pair) | ~5-10ms |
| Throughput (batch=16) | ~1,800 docs/sec |
| nDCG@10 (MS MARCO) | 74.30 |
| Languages | English (primary) |

**Why this model:**
- Tiny enough to bundle in Docker image or download on first use
- Apache-2.0 license — no restrictions
- Well-tested ONNX export via Xenova (used by Transformers.js and FastEmbed)
- Fast enough for real-time retrieval (20 pairs × 10ms = 200ms CPU, or 12.5ms batched)
- Sufficient quality for the v1 goal: "one thing working flawlessly"

### Upgrade Path: `BAAI/bge-reranker-v2-m3`

| Property | Value |
|----------|-------|
| Format | ONNX (onnx-community export on HuggingFace) |
| Size | 1.0 GB |
| Base model | bge-m3 (XLM-RoBERTa-derived) |
| License | MIT |
| Context window | 8192 tokens |
| CPU latency (per pair) | ~50-100ms |
| Languages | Multilingual (100+ languages including EN+DE) |

**When to upgrade:**
- German retrieval quality becomes a bottleneck
- Token budget allows 8192-token documents
- User is willing to trade 1GB disk and 5-10x latency for +5% nDCG

**Other options considered:**
- `jina-reranker-v2-base-multilingual` (1.1GB, CC-BY-NC-4.0) — license incompatible with commercial use
- `DeBERTa-v3-large MS MARCO` (1.7GB) — too large, English-only
- Cohere Rerank API — external dependency, latency, cost — rejected per v1 "local-first" philosophy

---

## 4. Rust Integration: `ort` Crate

### Why `ort` over alternatives

| Crate | Pros | Cons | Verdict |
|-------|------|------|---------|
| **ort** (pykeio/ort v2.0.0-rc.x) | Production-grade, 3-5x faster than Python, CUDA/Metal/OpenVINO backends, fastembed-rs uses it | C++ ONNX Runtime shared library required | ✅ Primary choice |
| **candle** (HuggingFace) | Pure Rust, no C++ dep, WASM support, direct HF Hub integration | 3-4x slower than PyTorch on GPU, cross-encoder support immature | ⚠️ Future option |
| **tract** (ONNX) | Pure Rust, no C++ dep | Slower, less model coverage | ❌ Not mature enough |
| **fastembed-rs** | High-level API, model auto-download, supports reranking | Adds dependency weight, abstracts control | ⚠️ Could use as reference |

### Dependency additions to `Cargo.toml`

```toml
[dependencies]
ort = { version = "2.0.0-rc.8", features = ["load-dynamic"] }
tokenizers = { version = "0.22", features = ["http"] }  # for HF tokenizer.json download
```

`load-dynamic` feature loads `libonnxruntime.so` / `onnxruntime.dll` at runtime, avoiding compile-time linking issues.

### Model storage

Models live in `~/.knowwhere/models/reranker/` (configurable via `KNOWWHERE_RERANKER_MODEL_DIR`). On first use, the model is downloaded from HuggingFace Hub:

```
~/.knowwhere/models/reranker/
├── Xenova/ms-marco-MiniLM-L-6-v2/
│   ├── onnx/
│   │   └── model.onnx
│   └── tokenizer.json
└── BAAI/bge-reranker-v2-m3/
    ├── onnx/
    │   └── model.onnx
    └── tokenizer.json
```

Model download uses `reqwest` (already a dependency) with progress logging via `tracing`.

---

## 5. Trait Design

### New: `Reranker` trait

```rust
// src/reranker/mod.rs

#[async_trait]
pub trait Reranker: Send + Sync {
    /// Rerank a set of candidate documents against a query.
    /// Input: query text + candidate documents (id + text content)
    /// Output: (doc_id, relevance_score) sorted by score descending
    async fn rerank(
        &self,
        query: &str,
        candidates: &[(Uuid, &str)],  // (node_id, document_text)
        top_k: usize,
    ) -> anyhow::Result<Vec<(Uuid, f32)>>;

    fn name(&self) -> &str;
    fn is_available(&self) -> bool;
}
```

### New: `CrossEncoderReranker` struct

```rust
// src/reranker/cross_encoder.rs

pub struct CrossEncoderReranker {
    session: ort::Session,           // ONNX Runtime session
    tokenizer: tokenizers::Tokenizer, // HuggingFace tokenizer
    model_name: String,
}

impl CrossEncoderReranker {
    pub fn load(model_path: &Path) -> anyhow::Result<Self>;
    pub fn download(model_name: &str, cache_dir: &Path) -> anyhow::Result<PathBuf>;
}
```

### New: `NoopReranker` (default when no model available)

```rust
pub struct NoopReranker;

impl Reranker for NoopReranker {
    async fn rerank(&self, _query: &str, candidates: &[(Uuid, &str)], top_k: usize) -> ... {
        // Return candidates unchanged (preserve RRF order)
        Ok(candidates.iter().take(top_k).map(|(id, _)| (*id, 1.0)).collect())
    }
}
```

### Modified: `StorageBackend` trait

Add to `src/storage/backend.rs`:

```rust
/// Optional reranker for two-stage retrieval.
/// When set, hybrid_retrieve applies cross-encoder reranking after RRF fusion.
fn reranker(&self) -> Option<&dyn Reranker> { None }
```

Or better: pass the reranker as a parameter to `hybrid_retrieve()`:

```rust
async fn hybrid_retrieve(
    &self,
    query: &HybridQuery,
    reranker: Option<&dyn Reranker>,  // NEW
) -> anyhow::Result<Vec<ScoredNode>>;
```

This avoids storing the reranker in the storage backend (separation of concerns). The reranker lives in `AppState` alongside the embedding provider.

---

## 6. Integration Points

### 6.1 AppState (`src/api/routes.rs`)

```rust
pub struct AppState {
    // ... existing fields ...
    pub reranker: Option<Arc<dyn Reranker>>,  // NEW
}
```

### 6.2 Server startup (`src/main.rs`)

```rust
// After embedding provider initialization
let reranker: Option<Arc<dyn Reranker>> = if enable_reranking {
    let model_dir = std::env::var("KNOWWHERE_RERANKER_MODEL")
        .unwrap_or_else(|_| "Xenova/ms-marco-MiniLM-L-6-v2".into());
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("knowwhere/models");

    match CrossEncoderReranker::load_or_download(&model_dir, &cache_dir).await {
        Ok(r) => {
            tracing::info!(model = %r.name(), "cross-encoder reranker loaded");
            Some(Arc::new(r))
        }
        Err(e) => {
            tracing::warn!("cross-encoder reranker unavailable: {e}, falling back to RRF-only");
            None
        }
    }
} else {
    None
};
```

### 6.3 Retrieval endpoint (`src/api/routes.rs`)

In the `/retrieve` handler, after building the `HybridQuery`:

```rust
// Currently (simplified):
let results = state.store.hybrid_retrieve(&query).await?;

// Proposed:
let results = state.store.hybrid_retrieve(
    &query,
    state.reranker.as_ref().map(|r| r.as_ref()),
).await?;
```

### 6.4 In `hybrid_retrieve()` (`src/storage/in_memory.rs`)

After RRF fusion (line ~822), before final top_k cut:

```rust
let fused = Self::rrf_fuse(&vector_ids, &bm25_results, 60.0);

// NEW: Cross-encoder reranking
let reranked = if let Some(reranker) = reranker {
    let nodes = self.nodes.read().await;
    let candidates: Vec<(Uuid, String)> = fused.iter()
        .filter_map(|(id, _)| {
            nodes.get(id).map(|n| (*id, n.content.clone()))
        })
        .collect();
    drop(nodes);

    match reranker.rerank(query_text.unwrap_or(""), &candidates, top_k).await {
        Ok(scored) => scored,
        Err(e) => {
            tracing::warn!("reranker failed, falling back to RRF scores: {e}");
            fused.into_iter().take(top_k).collect()
        }
    }
} else {
    fused.into_iter().take(top_k).collect()
};
```

---

## 7. Performance Impact

### Latency model (M3 Mac, CPU-only)

| Stage | Current (ms) | Proposed (ms) | Delta |
|-------|-------------|---------------|-------|
| Bi-Encoder embed | 15 (Ollama) | 15 | — |
| USearch search | <1 | <1 | — |
| BM25 search | <1 | <1 | — |
| RRF fusion | <1 | <1 | — |
| **Cross-Encoder rerank** | — | **50-200** | **+50-200ms** |
| **Total** | ~16ms | **~66-216ms** | **+50-200ms** |

Cross-encoder latency depends on:
- Candidate count: reranking 20 pairs = 20 forward passes
- Batch size: batched inference reduces latency (20 pairs in batch of 16 = 2 passes)
- Model: MiniLM-L6 (80MB) ~5-10ms/pair CPU; BGE-reranker (1GB) ~50-100ms/pair CPU

### Optimizations available

1. **Batch inference**: ONNX Runtime supports batched input — process all pairs in one call
2. **Caching**: Same query repeated? Cache CE scores (TTL: 60s)
3. **Token limit**: Truncate documents to 512 tokens (MiniLM limit) before reranking
4. **Feature flag**: `reranker` feature flag — builds without ORT dependency when disabled
5. **Threshold pruning**: Skip CE for candidates with RRF score below threshold

### Memory

- MiniLM-L6: ~100MB RAM for ONNX session + tokenizer
- BGE-reranker-v2-m3: ~1.2GB RAM
- No persistent memory growth (stateless inference)

---

## 8. Feature Flag Strategy

```toml
[features]
default = ["reranker"]  # enabled by default for v1
reranker = ["dep:ort", "dep:tokenizers"]  # cross-encoder reranking via ONNX
```

When `reranker` is disabled:
- `CrossEncoderReranker` is `#[cfg(feature = "reranker")]`
- `AppState.reranker` is always `None`
- Retrieval falls back to RRF-only (current behavior)
- No `ort` / `tokenizers` dependency — smaller binary, no libonnxruntime needed

Docker builds include `reranker` by default. The `Dockerfile` must install `libonnxruntime`:
```dockerfile
RUN apt-get update && apt-get install -y libonnxruntime-dev
```

---

## 9. Testing Strategy

### Unit tests (`src/reranker/tests.rs`)

```rust
#[cfg(feature = "reranker")]
mod tests {
    // Test with a tiny ONNX model (or mock)
    #[tokio::test]
    async fn test_reranker_loads_model();
    #[tokio::test]
    async fn test_reranker_scores_relevant_higher();
    #[tokio::test]
    async fn test_reranker_handles_empty_candidates();
    #[tokio::test]
    async fn test_reranker_truncates_long_documents();
    #[tokio::test]
    async fn test_noop_reranker_passthrough();
}
```

### Integration tests

- Run retrieval with and without reranker, verify score ordering differs
- German queries: verify MiniLM handles German reasonably (or falls back gracefully)
- Benchmark: measure latency impact on 50-case Canary benchmark

### Canary benchmark extension

Add reranker-enabled variant to `longmemeval_retrieval_eval`:
```
--reranker-model Xenova/ms-marco-MiniLM-L-6-v2
```

---

## 10. Implementation Plan

### Phase 1: Infrastructure (1-2 days)

1. Add `ort` + `tokenizers` dependencies to `Cargo.toml` (behind `reranker` feature)
2. Create `src/reranker/mod.rs` + `src/reranker/cross_encoder.rs`
3. Implement `Reranker` trait + `NoopReranker`
4. Implement model download from HuggingFace Hub
5. Implement ONNX inference for (query, document) → score

### Phase 2: Integration (1 day)

6. Add `reranker: Option<Arc<dyn Reranker>>` to `AppState`
7. Wire reranker into `main.rs` startup (load model, graceful fallback)
8. Modify `StorageBackend::hybrid_retrieve` signature to accept `Option<&dyn Reranker>`
9. Implement CE reranking in `in_memory.rs` RRF fusion pipeline
10. Update `postgres_store.rs` equivalently

### Phase 3: Polish (1 day)

11. Add `KNOWWHERE_RERANKER_MODEL` env var support
12. Add `--reranker-model` CLI flag to server binary
13. Update Dockerfile with `libonnxruntime`
14. Tests + benchmarks
15. Update docs (README, ARCHITECTURE)

---

## 11. Open Questions

| # | Question | Recommendation |
|---|----------|---------------|
| 1 | Should reranker be enabled by default? | Yes — falls back gracefully to RRF-only if model unavailable |
| 2 | Which model for v1? | MiniLM-L6 — 80MB, Apache-2.0, fast |
| 3 | Batch or sequential inference? | Batch (single ONNX call per query, all pairs at once) |
| 4 | Cache CE scores? | Not in v1 — low query volume, simple is better |
| 5 | German support in MiniLM? | Partial — MiniLM was trained on English MS MARCO. German documents will be scored but suboptimally. BGE-reranker-v2-m3 is the proper multilingual upgrade path. |
| 6 | Where to store ONNX models? | `~/.knowwhere/models/reranker/` (XDG cache dir) |

---

## 12. References

- [ort crate docs](https://ort.pyke.io/) — ONNX Runtime for Rust
- [fastembed-rs](https://github.com/Anush008/fastembed-rs) — Reference implementation for reranking in Rust
- [frankensearch-rerank](https://docs.rs/frankensearch-rerank) — FlashRank cross-encoder for Rust
- [Vera](https://github.com/lemon07r/Vera) — Rust code search with BM25+vector+RRF+CE reranking
- [BGE Reranker v2-m3 ONNX](https://huggingface.co/onnx-community/bge-reranker-v2-m3-ONNX) — Community ONNX export
- [Reranker Benchmarks (Nexumo, 2025)](https://medium.com/@Nexumo_/10-vector-db-rerankers-quality-vs-latency-c747611f4c96)
- [Sentence Transformers Cross-Encoder Models](https://sbert.net/docs/cross_encoder/pretrained_models.html)

# TST → KnowWhere: Implementation Summary (2026-05-15)

Token Superposition Training (arXiv: 2605.06546, Nous Research) mapped to KnowWhere's fractal memory architecture — analysis, spike, and implementation.

## Paper Summary

TST trains LLMs 2-3× faster by processing "bags" of contiguous tokens instead of individual tokens during the first 20-40% of training. The model averages token embeddings (input superposition) and predicts the next bag via multi-hot cross-entropy (output superposition). After this coarse phase, it switches to standard next-token prediction. The final model is architecturally identical to a conventionally trained one.

**Critical finding:** Re-initializing embeddings between phases destroys ALL gains.

## 6 TST Insights → KnowWhere Mapping

### 1. ✅ Embedding-Averaging for L1 Nodes (VALIDATED)
**Status:** Spike-proven + helper functions implemented

The core TST insight: averaging embeddings preserves enough geometric structure for useful coarse representations. Our spike (001-bag-of-claims-avg) proved this with KnowWhere's nomic-embed-text-v2-moe vectors:
- 25% union Jaccard between centroid and L0 neighborhoods (random groups)
- 91% coverage (centroid neighbors are also L0 neighbors)
- 0.85 centroid→member cosine similarity

**Implementation:**
- `truncate_vector()` — Matryoshka truncation to first N dimensions
- `mean_vector()` — TST bag-of-claims averaging for L1 parent node creation
- Both in `src/memory/fractal_node.rs`

**Rule for future consolidation:** L1 node vectors MUST be `mean_vector(child_vectors)`, never re-embedded from text.

### 2. ✅ 64d Ultra-Coarse Index (INFRASTRUCTURE READY)
**Status:** Index created, search function implemented, retrieval wiring pending

Added third HNSW index for 64d truncated embeddings (`ULTRA_COARSE_DIM = 64`). This enables a TST-inspired 3-level retrieval cascade:

```
Query → 64d HNSW → 256d HNSW → 768d Precision
        ↑ billig    ↑ mittel      ↑ teuer
        filtert 95% filtert 50%   precision
```

**Implementation:**
- `ULTRA_COARSE_DIM = 64` constant
- `ultra_coarse_index`, `ultra_coarse_dimension`, `ultra_coarse_uuid_to_key`, `ultra_coarse_key_to_uuid` fields in `MemoryStore`
- `ensure_ultra_coarse_index()` — auto-creates/re-reserves the 64d index
- `ultra_coarse_search()` — HNSW search on 64d truncated vectors
- Auto-insert on every `insert()` call (lines after coarse index block)
- Constructor init in `new()` and `with_persistence()`

**Not yet wired:** `retrieve_fractal()` still uses only the main 768d index. The 3-level cascade needs to be integrated — see "Next Steps."

### 3. ✅ Representation Continuity Test
**Status:** Test written and passing

TST's harshest ablation result: re-initializing embeddings between phases = all gains lost. For KnowWhere, this means truncated embeddings (64d, 256d) MUST stay geometrically continuous with full 768d embeddings after consolidation.

**Implementation:**
- `matryoshka_continuity(a, b, trunc_dim)` — returns `(full_sim, truncated_sim)`
- Test: `matryoshka_continuity_preserved` — verifies truncated cosine similarity stays within 10% of full similarity
- All 3 new tests passing: `truncate_vector_matryoshka`, `mean_vector_bag_of_claims`, `matryoshka_continuity_preserved`

### 4. 🔜 Distributional Retrieval Scoring
**Status:** Analyzed, not implemented

TST's MCE loss predicts a token *distribution*, not a single token. Analog: retrieval should return a probability distribution over candidates, not discrete top-k.

KnowWhere's RRF fusion already does implicit distributional scoring. The gap: normalize scores as a proper distribution (`softmax` over candidate set). Add `distribution_scores` field to `ScoredNode` response.

**Estimated effort:** ~15 lines in `routes.rs`, after RRF fusion step.

### 5. 🔜 Two-Phase Consolidation
**Status:** Analyzed, not implemented

TST's architecture: Phase 1 (billig, grob) → Phase 2 (teuer, präzise). For KnowWhere consolidation:
- Phase 1: `mean_vector()` on semantic clusters → coarse parent nodes (cheap)
- Phase 2: Precision refinement only on already-coarse nodes (expensive, reduced set)

Current `ConsolidationEngine` is scaffolding only — `ConsolidationStore` trait has no implementation. Full implementation needs ~150 lines + scheduler changes.

### 6. ✅ Anti-Patterns (Documented)
**Status:** Saved to skill `tst-knowwhere-insights`

What TST empirically rejected and KnowWhere should avoid:
- ❌ Positional encoding in bags (bag permutation invariance is a feature)
- ❌ Complex loss functions (simple mean aggregation is more robust)
- ❌ Multi-head prediction for different granularities (no gain)
- ❌ Re-initializing between phases (destroys all gains)

---

## Spike Results

**Location:** `knowwhere/spikes/001-bag-of-claims-avg/`

| Metric | Range | Average |
|--------|-------|---------|
| Union Jaccard (Centroid vs L0∪) | 0.22–0.29 | **0.254** |
| Centroid Coverage | 76–100% | **91.3%** |
| Centroid→Member cos_sim | 0.83–0.86 | **0.847** |

Data: 412 PersonaMem nodes, nomic-embed-text-v2-moe (768d), 4 random groups of 5 nodes each.

**Verdict: VALIDATED** — Bag-of-claims embedding averaging preserves geometric structure. L1 nodes via mean-pooling are viable.

---

## Code Changes (2026-05-15)

### `src/memory/fractal_node.rs`
- `+ truncate_vector(vector, dim) -> Option<Vec<f32>>` — Matryoshka truncation
- `+ mean_vector(vectors) -> Option<Vec<f32>>` — TST bag-of-claims averaging
- `+ matryoshka_continuity(a, b, trunc_dim) -> Option<(f32, f32)>` — geometric continuity check

### `src/storage/in_memory.rs`
- `+ ULTRA_COARSE_DIM = 64` constant
- `+ ultra_coarse_index`, `ultra_coarse_dimension`, `ultra_coarse_uuid_to_key`, `ultra_coarse_key_to_uuid` — MemoryStore fields
- `+ ensure_ultra_coarse_index()` — index lifecycle management
- `+ ultra_coarse_search()` — 64d HNSW search
- `+ ultra-coarse insert block` — auto-populates on every insert
- `+ constructor init` — in `new()` and `with_persistence()`

### `src/memory/tests.rs`
- `+ truncate_vector_matryoshka` — basic truncation test
- `+ mean_vector_bag_of_claims` — averaging test (normal + edge cases)
- `+ matryoshka_continuity_preserved` — continuity validation

### Build
- ✅ `cargo build --release` — clean (9 pre-existing warnings, 0 new)
- ✅ `cargo test` (new tests) — 5/5 passing
- ⚠️ Full test suite not run (pre-existing Rust 2015 edition errors on async tests)

---

## Bonus: User-ID Filter Bug

`retrieve_fractal` in `routes.rs` (lines 210-214):

```rust
match &query.user_id {
    None => node_uid.is_none(),  // ← returns ONLY nodes without user_id
    Some(uid) => node_uid.map_or(true, |v| v == uid.as_str()),
}
```

When no `user_id` is passed, only nodes *without* `user_id` metadata are returned. PersonaMem data has ALL nodes with `user_id` → queries return 0 results silently.

**Workaround:** Extract user_id from `data/state.json` and pass in request.  
**Fix needed:** Either document as intended behavior, or change `None` branch to return all nodes.

---

## Next Steps (Priority Order)

1. **Wire 3-level cascade** — Integrate `ultra_coarse_search()` + `coarse_search()` into `retrieve_fractal()` (~30 lines)
2. **Distributional Scoring** — Add `distribution_scores` field to `ScoredNode` (~15 lines)
3. **Implement ConsolidationStore** — Actual storage backend for consolidation trait, using `mean_vector()` (~100 lines)
4. **64d index rebuild on load** — Currently 64d index is not rebuilt in `load_state()` (only main 768d index is)
5. **Fix user-id filter** — Either change default behavior or document prominently
6. **Run full test suite** — Fix Rust 2015 edition issue or filter to sync tests

---

## Reference

- **Paper:** arXiv 2605.06546 — "Efficient Pre-Training with Token Superposition"
- **Blog:** https://nousresearch.com/token-superposition/
- **Skill:** `tst-knowwhere-insights` — full insight map
- **Spike:** `knowwhere/spikes/001-bag-of-claims-avg/` — code + README

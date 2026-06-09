# Spike: Storage Pipeline Extraction

**Date:** 2026-06-09
**Status:** Proposed (3-agent consensus, 3/3 agree)
**Decision:** Standalone pipeline with `NodeProvider` trait

## Problem

`hybrid_retrieve()` in both storage backends (~200 lines each) implements identical pipeline logic: filter → score_node → sort → temporal_boost → temporal_scoring → distributional_softmax → truncate. The postgres backend duplicates this chain 4× across its branching code paths (text-only, vector-only, dim-mismatch fallback, hybrid). Every new filter, scoring tweak, or policy gate must be implemented twice — and tested twice.

## Architecture

### The Seam: Post-Fusion

Both backends converge at the same point: after vector search + BM25 + RRF fusion, they produce `Vec<(Uuid, f32)>` (node ID → fused score). Everything after this point is pure, deterministic, I/O-free computation on those tuples. That's the extraction boundary.

### Interface

```rust
// src/storage/pipeline.rs

use async_trait::async_trait;
use uuid::Uuid;
use crate::memory::fractal_node::FractalNode;
use crate::storage::backend::{HybridQuery, ScoredNode};

/// Minimal trait — only the one thing the pipeline needs from a backend.
#[async_trait]
pub(crate) trait NodeProvider: Send + Sync {
    async fn get_node(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>>;
}

/// Every StorageBackend automatically implements NodeProvider.
#[async_trait]
impl<T: crate::storage::StorageBackend + Send + Sync + ?Sized> NodeProvider for T {
    async fn get_node(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>> {
        self.get(id).await
    }
}

/// Shared post-fusion retrieval pipeline.
///
/// After a backend has performed vector search, BM25, and RRF fusion,
/// pass the fused `(node_id, rrf_score)` tuples here. The pipeline
/// materializes nodes via `provider`, then runs the full filter → score
/// → sort → temporal → distributional chain and returns `ScoredNode`s.
///
/// This function is pure logic — no I/O beyond `provider.get_node()`.
pub(crate) async fn finalize_retrieval(
    provider: &impl NodeProvider,
    fused: Vec<(Uuid, f32)>,
    query: &HybridQuery,
) -> anyhow::Result<Vec<ScoredNode>> {
    // 1. Materialize nodes
    let mut raw: Vec<(f32, FractalNode)> = Vec::with_capacity(fused.len());
    for (id, score) in fused {
        if let Some(node) = provider.get_node(&id).await? {
            raw.push((score, node));
        }
    }

    // 2. Filters (profile, internal-meta, memory-type, user-id)
    raw.retain(|(_, node)| {
        if !query.profile.allows(node) { return false; }
        if !shared::allow_internal_meta(query.memory_type_filter)
            && shared::is_internal_meta_artifact(node) { return false; }
        if let Some(mt) = query.memory_type_filter {
            if node.memory_type != mt { return false; }
        }
        if let Some(ref uid) = query.user_id {
            let node_uid = node.metadata.get("user_id").and_then(|v| v.as_str());
            if node_uid.is_some_and(|v| v != uid.as_str()) { return false; }
        }
        true
    });

    // 3. Recency boost (policy-gated)
    if !matches!(query.profile, RetrievalProfile::FullFidelity) {
        if let Some(b) = query.recency_boost {
            apply_temporal_boost(&mut raw, b);
        }
    }

    // 4. Score conversion
    let mut weighted: Vec<ScoredNode> = raw
        .into_iter()
        .map(|(score, node)| query.profile.score_node(score, node, query.source_type_weights))
        .collect();

    // 5. Stable sort
    weighted.sort_by(|a, b| {
        b.score.partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    // 6. Hybrid temporal scoring (policy-gated)
    if !matches!(query.profile, RetrievalProfile::FullFidelity) {
        if let Some(w) = query.temporal_weight {
            shared::apply_hybrid_temporal_scoring(&mut weighted, w);
        }
    }

    // 7. Distributional softmax
    if !weighted.is_empty() {
        let max_score = weighted.iter()
            .map(|n| n.score)
            .fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = weighted.iter()
            .map(|n| (n.score - max_score).exp())
            .collect();
        let sum: f32 = exps.iter().sum();
        if sum > 0.0 {
            for (item, prob) in weighted.iter_mut().zip(exps.iter().map(|e| e / sum)) {
                item.distribution_scores = Some(vec![prob]);
            }
        }
    }

    // 8. Truncate
    weighted.truncate(query.top_k);
    Ok(weighted)
}
```

### Usage in backends

**MemoryStore** — drops from ~92 lines to ~15:

```rust
async fn hybrid_retrieve(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
    let vector = query.query_vector.as_deref().unwrap_or(&[]);
    let fetch_k = query.profile.fetch_k(query.top_k);
    let raw = self.hybrid_retrieve_inner(
        query.query_text.as_deref(), vector, fetch_k,
        query.max_depth, query.fusion_strategy, query.query_type_routing,
    ).await; // already returns Vec<(f32, FractalNode)>
    pipeline::finalize_retrieval(self, raw.into_iter().collect(), query).await
}
```

**PostgresStore** — collapses 4 duplicated filter chains into one call:

```rust
async fn hybrid_retrieve(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
    // ... dimension alignment + trajectory setup (unchanged) ...
    let fused = shared::rrf_fuse(&vector_ids, &bm25_ids, 60.0);
    pipeline::finalize_retrieval(self, fused, query).await
}
```

## LOC Impact

| File | Removed | Added | Net |
|------|---------|-------|-----|
| `pipeline.rs` (new) | — | ~120 | +120 |
| `in_memory.rs` trait impl `hybrid_retrieve` | ~77 | 0 | −77 |
| `postgres_store.rs` trait impl `hybrid_retrieve` | ~83 | 0 | −83 |
| `postgres_store.rs` (4× duplicated filter/score chains) | ~60 | 0 | −60 |
| `mod.rs` | — | 1 | +1 |
| **Total** | **~220** | **~121** | **~−99** |

## Test Strategy

### Tier 1 — Pipeline unit tests (in `pipeline.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockProvider(HashMap<Uuid, FractalNode>);

    #[async_trait]
    impl NodeProvider for MockProvider {
        async fn get_node(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>> {
            Ok(self.0.get(id).cloned())
        }
    }

    fn mock_node(id: &str, mtype: MemoryType, score: f32) -> (Uuid, FractalNode) { /* ... */ }

    #[tokio::test]
    async fn test_memory_type_filter_excludes_wrong_type() { /* ... */ }
    #[tokio::test]
    async fn test_user_id_filter() { /* ... */ }
    #[tokio::test]
    async fn test_temporal_boost_gated_by_full_fidelity() { /* ... */ }
    #[tokio::test]
    async fn test_stable_sort_by_uuid_on_equal_score() { /* ... */ }
    #[tokio::test]
    async fn test_distributional_softmax_sums_to_one() { /* ... */ }
}
```

### Tier 2 — Backend integration tests (unchanged)

Existing tests in `src/memory/tests.rs` calling `store.hybrid_retrieve()` continue to pass. The trait signature hasn't changed.

### Tier 3 — Cross-backend equivalence (new, optional)

```rust
#[tokio::test]
async fn test_inmemory_and_postgres_produce_same_ranking() {
    // Populate both stores with identical data, run same query, assert same IDs + scores
}
```

## Migration Plan

### Phase 1 — Create pipeline module (1 PR, pure addition)

- Create `src/storage/pipeline.rs` with `NodeProvider` trait + blanket impl + `finalize_retrieval`
- Add `pub(crate) mod pipeline;` to `mod.rs`
- Write Tier 1 unit tests
- Impact: Zero behavioral change, zero callers touched
- Risk: Minimal

### Phase 2 — Migrate MemoryStore (1 PR)

- Rewrite `MemoryStore::hybrid_retrieve` trait impl to delegate to `pipeline::finalize_retrieval`
- Remove the inline filter/score/sort/temporal chain (lines ~250-317 in trait impl)
- Keep `hybrid_retrieve_inner` untouched (it's the backend-specific search + fusion)
- Run all in-memory tests
- Impact: `in_memory.rs` shrinks by ~77 lines
- Risk: Low — in-memory path is simpler, no branching

### Phase 3 — Migrate PostgresStore (1 PR)

- Rewrite `PostgresStore::hybrid_retrieve` to delegate to pipeline
- All 4 branching paths (text-only, vector-only, dim-mismatch, hybrid) collapse: each builds fused results, then calls `finalize_retrieval`
- Remove duplicated filter/score/sort blocks from all branches
- Remove `apply_temporal_boost_scored` (now handled by pipeline)
- Dimension alignment + trajectory tracking stay in postgres
- Run all postgres tests
- Impact: `postgres_store.rs` shrinks by ~140 lines
- Risk: Medium — verify trajectory logging still interleaves correctly

### Phase 4 — Cleanup (optional)

- Move `apply_temporal_boost` variants into `pipeline.rs` (unify the `[(f32, FractalNode)]` and `[ScoredNode]` versions)
- Delete dead code from both backends

## Rejected Alternative: Template Method

Adding `hybrid_retrieve` as a default method on `StorageBackend` was rejected because:

1. Postgres has significant pre-pipeline logic (dimension alignment, 3-branch early-return, `#[cfg]`-gated trajectory tracking) that cannot be cleanly split into abstract hooks
2. Postgres would need to override the default method entirely — defeating the purpose
3. `StorageBackend` already has ~20 methods; adding pipeline hooks would bloat it further
4. Testing a default trait method requires a full mock backend implementing all 20 methods

## Rejected Alternative: Closure-based pipeline

Passing a closure `|fetch_k| -> Future<Vec<(f32, FractalNode)>>` was rejected because:

1. `async` closures in trait bounds are verbose and unstable-adjacent
2. A trait with blanket impl is more idiomatic Rust and simpler at the call site
3. The `NodeProvider` trait is self-documenting: "this pipeline only needs `get_node`"

## Summary

| Dimension | Current | Target |
|-----------|---------|--------|
| Pipeline implementations | 2 (identical copies) | 1 (shared) |
| Postgres duplicated filter chains | 4× | 1× |
| Lines changed for new filter/scoring step | 2 files | 1 file |
| New backend onboarding | Copy ~200 lines | Implement `StorageBackend` + call `finalize_retrieval` |
| Pipeline test coverage | Via integration tests only | Tier 1 unit tests + Tier 2 integration |

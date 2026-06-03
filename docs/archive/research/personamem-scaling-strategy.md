# PersonaMem 128k/1M Scaling Strategy

**Date:** 2026-05-07
**Context:** After fixing 32k split (embedding + chunking), evaluate path to larger splits.

## Scaling Dimensions

| Aspect | 32k (current) | 128k (4×) | 1M (30×) |
|--------|-------------|----------|---------|
| Docs per persona | 2-4 sessions | 8-16 sessions | 40-80 sessions |
| Doc size range | 5K-40K chars | 15K-120K chars | 40K-400K chars |
| Chunks per doc | 1-3 | 1-8 | 3-25 |
| Total nodes (est.) | 500 | 3,000 | 25,000 |
| Ingest time (batch) | ~2 min | ~5 min | ~20 min |
| Retrieval latency | <100ms | <200ms | <500ms (HNSW log N) |

## What Already Scales

1. **Chunking**: Hybrid strategy (session → turn → 16K fixed) works identically for all splits. Just more chunks.
2. **qwen3-embedding**: 32K token context handles ALL chunk sizes (16K max chars = 4K tokens).
3. **pgvector HNSW**: O(log N) vector search. 25K nodes is tiny for pgvector.
4. **Cross-Encoder**: Runs on top-K candidates (constant cost regardless of DB size).

## What Needs Adjustment

### top_k Increase
More chunks per persona → need more candidates to cover the same number of sessions.

| Split | Recommended top_k | Rationale |
|-------|-----------------|-----------|
| 32k | 5 | 2-4 sessions, 5-12 chunks |
| 128k | 10 | 8-16 sessions, 15-40 chunks |
| 1M | 15-20 | 40-80 sessions, 80-200 chunks |

### Consolidation Tuning
More nodes → consolidation runs longer. Adjust scheduler:
- `DREAM_CONSOLIDATION_BATCH_SIZE`: 10 → 20 (128k), 10 → 50 (1M)
- Consolidation interval: keep 60 min, but force after ingest

### Context Window for LLM
With top_k=15-20, context grows significantly. Need to manage:
- Trim chunk content to relevant excerpts (first 500 chars?)
- Use Cross-Encoder to select top-5 from top-20
- Or: two-stage: retrieve 20 → cross-encoder → keep top 5 for LLM

### DB Size Management
25K nodes with 1024-dim vectors = ~100MB for vectors. Manageable on M1/8GB.
pgvector HNSW index: ~20MB additional. Total: ~150MB for 1M split.

## Recommended Approach

1. **Test 128k first** — validates scaling before committing to 1M
2. **Run with --query-limit 20** for fast iteration
3. **Monitor**: retrieval latency, consolidation time, DB size
4. **If latency spikes**: reduce top_k, increase Cross-Encoder reliance

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| Retrieval too slow (>1s) | Low | HNSW is O(log N), 25K nodes = fast |
| Context too large for LLM | Medium | Cross-Encoder pre-filter to top-5 |
| Consolidation timeout | Medium | Increase batch size, run async |
| M1 memory pressure | Low | 150MB for 25K vectors, 639MB for qwen3 model |

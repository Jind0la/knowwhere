# Phase 2 Evaluation — Embedding Upgrade: nomic-embed-text

**Date:** 2026-05-17  
**Run ID:** 199  
**Task:** t_0567f891  
**Status:** Historical — model switch benchmark only.

> **Note:** This document records the initial embedding model switch evaluation from May 17. The full Phase 2 completion with post-migration LongMemEval results is in [`docs/phase2-retrieval-quality-completion.md`](phase2-retrieval-quality-completion.md).

## What Changed

- Embedding model: bge-m3 (1024-dim) → nomic-embed-text (768-dim)
- All 2405 nodes re-embedded via `POST /nodes/reembed_all` in 77 seconds
- Runtime defaults already correct (provider.rs:268, runtime.rs:80)
- `.env.native` `OLLAMA_MODEL=nomic-embed-text` (set by previous worker)

## Evaluation Results

### Temporal Golden Queries (15q, boost=0.20, top_k=5)

| Metric | Phase 1 (bge-m3) | Phase 2 (nomic-embed-text) | Delta |
|--------|------------------|---------------------------|-------|
| No-boost avg score | 0.2072 | 0.2054 | -0.0018 |
| No-boost avg latency | 54.1ms | 56.3ms | +2.2ms |
| Boosted avg score | 0.3593 | 0.3637 | **+0.0044** |
| Boosted avg latency | 42.6ms | 34.9ms | **-7.7ms** |
| Δ score (boost effect) | +0.1521 | +0.1583 | +0.0062 |

14/15 queries benefit from boost, 1 correctly skipped (score >0.10 threshold).

### PersonaMem (20q, no boost, top_k=10)

| Metric | Phase 1 (bge-m3) | Phase 2 (nomic-embed-text) | Delta |
|--------|------------------|---------------------------|-------|
| avg_top_score | 0.2524 | 0.2227 | -0.0297 (-11.8%) |
| avg_latency | 115.1ms | 82.4ms | **-32.7ms (-28.4%)** |

## Analysis

nomic-embed-text is 768-dim vs bge-m3's 1024-dim. The dimensionality reduction yields:
- **28.4% faster** PersonaMem retrievals (82.4ms vs 115.1ms)
- **18% faster** boosted temporal queries (34.9ms vs 42.6ms)
- Slightly lower raw scores on PersonaMem (-11.8%)
- Marginally better scores on temporal queries (+1.2%)

**Recommendation:** The speed-quality trade-off favors nomic-embed-text. **Postscript**: The full Phase 2 evaluation on 42 stratified LongMemEval cases confirmed this choice — nomic-embed-text with turn-level storage, hybrid retrieval, and cross-encoder reranking achieves 72.97% Recall@5 (up from 7.1% pre-migration).

## Artifacts

- `eval/results/phase2_temporal_20260517.json` — 15 temporal queries, with/without boost
- `eval/results/phase2_personamem_20260517.json` — 20 PersonaMem queries
- Server running on `localhost:3737` with nomic-embed-text

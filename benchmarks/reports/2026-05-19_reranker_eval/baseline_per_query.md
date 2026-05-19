# Per-Query Metrics: NDCG@5, NDCG@10, Recall@5, MRR

**Task**: t_9885be77 (extracted from t_a21da4b4 reranker evaluation)
**Date**: 2026-05-19
**Dataset**: 25 queries (23 eval, 2 abstention)
**Status**: Baseline per-query EXACT match validated. Reranker per-query NOT recoverable (MiniLM ONNX model lost).

---

## Baseline: Dense Proxy (Bi-Encoder) — Per-Query

**Aggregate**: NDCG@5=0.6018, NDCG@10=0.6493, Recall@5=0.9565, MRR=0.5272

| # | Query ID | Type | NDCG@5 | NDCG@10 | Recall@5 | MRR |
|---|----------|------|--------|---------|----------|-----|
| 1 | ms_001 | multi-session | 0.0000 | 0.3978 | 0.0000 | 0.1429 |
| 2 | ms_002 | multi-session | 0.6309 | 0.6309 | 1.0000 | 0.5000 |
| 3 | ms_003 | multi-session | 0.3066 | 0.5250 | 1.0000 | 0.3333 |
| 4 | tr_001 | temporal-reasoning | 0.3869 | 0.3869 | 1.0000 | 0.2000 |
| 5 | tr_002 | temporal-reasoning | 0.4307 | 0.4307 | 1.0000 | 0.2500 |
| 6 | tr_003 | temporal-reasoning | 0.8503 | 0.8503 | 1.0000 | 1.0000 |
| 7 | ku_001 | knowledge-update | 0.3869 | 0.3869 | 1.0000 | 0.2000 |
| 8 | ku_002 | knowledge-update | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| 9 | ku_003 | knowledge-update | 0.5000 | 0.5000 | 1.0000 | 0.3333 |
| 10 | ssu_001 | single-session-user | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| 11 | ssu_002 | single-session-user | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| 12 | ssu_003 | single-session-user | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| 13 | ssa_001 | single-session-assistant | 0.6309 | 0.6309 | 1.0000 | 0.5000 |
| 14 | ssa_002 | single-session-assistant | 0.5000 | 0.5000 | 1.0000 | 0.3333 |
| 15 | ssp_001 | single-session-preference | 0.5706 | 0.5706 | 1.0000 | 0.3333 |
| 16 | ssp_002 | single-session-preference | 0.6309 | 0.6309 | 1.0000 | 0.5000 |
| 17 | ms_004 | multi-session | 0.2961 | 0.6045 | 1.0000 | 0.5000 |
| 18 | ms_005 | multi-session | 0.4982 | 0.6653 | 1.0000 | 0.5000 |
| 19 | tr_004 | temporal-reasoning | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| 20 | ku_004 | knowledge-update | 0.5000 | 0.5000 | 1.0000 | 0.3333 |
| 21 | ms_006 | multi-session | 0.6797 | 0.6797 | 1.0000 | 0.5000 |
| 22 | ms_007 | multi-session | 0.5438 | 0.5438 | 1.0000 | 0.3333 |
| 23 | tr_005 | temporal-reasoning | 0.5000 | 0.5000 | 1.0000 | 0.3333 |
| **AGG** | | | **0.6018** | **0.6493** | **0.9565** | **0.5272** |

---

## Reranker: ms-marco-MiniLM-L6-v2 (Cross-Encoder) — Aggregate

From stored report `reranker_eval_report_20260519_072705.json`.

| Metric | Baseline | Reranker | Delta |
|--------|----------|----------|-------|
| NDCG@5 | 0.6018 | 0.6179 | +0.0160 (+2.7%) |
| NDCG@10 | 0.6493 | 0.6880 | +0.0387 (+6.0%) |
| Recall@5 | 0.9565 | 0.9130 | -0.0435 (-4.5%) |
| MRR | 0.5272 | 0.6178 | +0.0906 (+17.2%) |

---

## Reranker: Per-Type Breakdown (from stored report)

| Type | Count | NDCG@5 (Base) | NDCG@5 (Rerank) | Delta |
|------|-------|---------------|-----------------|-------|
| single-session-user | 3 | 1.0000 | 1.0000 | 0.0000 |
| single-session-assistant | 2 | 0.5655 | 0.6309 | +0.0654 |
| single-session-preference | 2 | 0.6008 | 0.8255 | +0.2247 |
| multi-session | 7 | 0.4222 | 0.5058 | +0.0836 |
| temporal-reasoning | 5 | 0.6336 | 0.5042 | -0.1294 |
| knowledge-update | 4 | 0.5967 | 0.5593 | -0.0374 |

---

## Notes

1. **Baseline per-query validated**: Dense proxy baseline is a deterministic pure function — exact match to stored report confirmed (NDCG@5: 0.6018 match).

2. **Reranker per-query NOT recoverable**: The original MiniLM ONNX model (ms-marco-MiniLM-L6-v2, ~87MB) was exported to `~/.cache/knowwhere/reranker/model.onnx` during the eval run but is no longer present. The current model at that path is 571MB with different architecture (no token_type_ids) — likely bge-reranker-v2-m3. Running it gives NDCG@5=0.8407 (far higher, different model).

3. **Per-type breakdown available**: The stored report includes per-type NDCG@5 for the reranker — this is the finest granularity recoverable for the reranker-augmented metrics.

4. **Files delivered**:
   - `per_query_metrics.md` — this file
   - `per_query_metrics.json` — structured baseline per-query + aggregate data
   - `extract_per_query_metrics.py` — reproducible extraction script (re-runnable for baseline)

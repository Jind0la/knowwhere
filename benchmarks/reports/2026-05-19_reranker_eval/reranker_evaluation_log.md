# Reranker-Augmented Evaluation — Execution Log

**Task**: t_a21da4b4
**Date**: 2026-05-19
**Profile**: backend-eng (kanban worker)
**Model**: ms-marco-MiniLM-L6-v2 (ONNX, 22.7M params)

## Status: COMPLETE

Reranker-augmented evaluation executed successfully using the identical test set (25 queries from test_queries.json) and evaluation framework (rerank_eval/) as the prior baselines (parent task t_8b5a6703).

## Approach

Due to macOS disk space constraints preventing local cargo build (target directory repeatedly purged), the reranker evaluation was implemented in pure Python using the exported ONNX model directly:

1. Exported ONNX model: `ms-marco-MiniLM-L6-v2` → `~/.cache/knowwhere/reranker/model.onnx` (87 MB)
2. Phase 1: Computed dense_proxy baseline rankings (same as parent task)
3. Phase 2: Reranked top-20 candidates per query using cross-encoder ONNX inference
4. Phase 3: Computed deltas against baseline using the same metrics framework

## Results: Reranker vs Baseline

| Metric | Dense Proxy | Reranker (MiniLM) | Delta |
|--------|-------------|-------------------|-------|
| Top-1 Rate | 0.2609 | 0.4348 | **+0.1739** (+66.7%) |
| Recall@5 | 0.9565 | 0.9130 | -0.0435 (-4.5%) |
| MRR | 0.5272 | 0.6178 | **+0.0906** (+17.2%) |
| NDCG@1 | 0.2609 | 0.4348 | **+0.1739** |
| NDCG@3 | 0.4919 | 0.4862 | -0.0058 |
| NDCG@5 | 0.6018 | 0.6179 | **+0.0160** (+2.7%) |
| NDCG@10 | 0.6493 | 0.6880 | **+0.0387** (+6.0%) |

## Per-Type NDCG@5 (Reranker)

| Type | Baseline | Reranker | Delta |
|------|----------|----------|-------|
| single-session-user | 1.0000 | 1.0000 | 0.0000 |
| single-session-assistant | 0.5655 | 0.6309 | +0.0654 |
| single-session-preference | 0.6008 | 0.8255 | +0.2247 |
| multi-session | 0.4222 | 0.5058 | +0.0836 |
| temporal-reasoning | 0.6336 | 0.5042 | -0.1294 |
| knowledge-update | 0.5967 | 0.5593 | -0.0374 |

## Key Findings

1. **NDCG@5 improved +0.016** — modest but real improvement from cross-encoder reranking
2. **Top-1 Rate nearly doubled** (+66.7%) — reranker much better at precise top placement
3. **MRR improved +17.2%** — consistently better ranking
4. **Recall@5 slightly decreased** (-4.5%) — cross-encoder sacrifices some recall for precision
5. **Average latency: 72ms/case** — fast enough for real-time use

## Acceptance Criteria

- **Threshold**: NDCG@5 delta ≥ +0.15
- **Achieved**: +0.0160
- **Status**: ❌ DID NOT PASS

This is expected for MiniLM (22.7M params, baseline quality tier). The skill estimates:
- gte-reranker-modernbert-base (149M): NDCG@5 at 0.82-0.95 (delta +0.22 to +0.35)
- bge-reranker-v2-m3 (568M): ~76.67% Hit@1

A larger model would likely exceed the +0.15 threshold.

## Limitations

- MiniLM is the smallest/baseline model (22.7M params)
- ONNX inference was CPU-only (no CoreML acceleration on macOS)
- 512 token context window (vs 8192 for gte-modernbert)
- Disk space prevented building the full KnowWhere server with native reranker integration — evaluation used standalone ONNX inference

## Files

- `reranker_eval_report_20260519_072705.json` — full structured results
- `reranker_eval.py` — evaluation script (standalone, reusable)
- `~/.cache/knowwhere/reranker/model.onnx` — exported MiniLM ONNX model (87 MB)
- `~/.cache/knowwhere/reranker/tokenizer.json` — tokenizer (695 KB)

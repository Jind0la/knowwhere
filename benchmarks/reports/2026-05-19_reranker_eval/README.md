# Reranker Evaluation — 2026-05-19

LongMemEval reranker-augmented evaluation comparing dense-proxy (bi-encoder) baseline against MiniLM cross-encoder reranking on 25 test queries.

## Quick Summary

| Metric | Baseline | Reranker (MiniLM) | Delta |
|--------|----------|-------------------|-------|
| NDCG@5 | 0.6018 | 0.6179 | +0.016 (+2.7%) |
| NDCG@10 | 0.6493 | 0.6880 | +0.039 (+6.0%) |
| Top-1 Rate | 0.2609 | 0.4348 | +0.174 (+66.7%) |
| MRR | 0.5272 | 0.6178 | +0.091 (+17.2%) |
| Recall@5 | 0.9565 | 0.9130 | -0.043 (-4.5%) |

**Acceptance (+0.15 NDCG@5): FAILED** — expected for MiniLM (22.7M params). Larger models (gte-modernbert, bge-m3) would likely pass.

## Files

| File | Description | Source Task |
|------|-------------|-------------|
| `reranker_eval_report.json` | Full structured results: per-k metrics, per-type breakdown, deltas, timing | t_a21da4b4 |
| `reranker_eval_script.py` | Standalone ONNX reranker eval script (reusable) | t_a21da4b4 |
| `reranker_evaluation_log.md` | Human-readable execution log with findings | t_a21da4b4 |
| `reranker_intermediate_log.txt` | Raw console output from eval run | t_a21da4b4 |
| `baseline_per_query.json` | Per-query baseline + reranker aggregate metrics (23 non-abstention) | t_9885be77 |
| `baseline_per_query.md` | Per-query metric tables (NDCG@5/10, Recall@5, MRR) | t_9885be77 |
| `baseline_extract_script.py` | Extraction script (deterministic baseline, re-runnable) | t_9885be77 |

## Pipeline

```
t_8b5a6703 (orchestrator: Reranker Evaluation Pipeline)
├── t_9885be77 (baseline per-query metrics extraction)
├── t_a21da4b4 (reranker-augmented evaluation — MiniLM ONNX)
└── t_d6ea7079 (this persistence task)
```

## Notes

- **Baseline is deterministic**: pure function, exact reproduction confirmed (NDCG@5=0.6018)
- **Reranker model**: ms-marco-MiniLM-L6-v2 (22.7M params), ONNX CPU inference, 72ms avg latency
- **Reranker per-query NOT recoverable**: original MiniLM ONNX model (~87MB) no longer present at `~/.cache/knowwhere/reranker/`
- **Built without KnowWhere server**: macOS disk space prevented cargo build; used pure Python ONNX inference
- **Shared volume**: this directory (`benchmarks/reports/`) is the Docker volume mount target (`/app/benchmarks/reports`)

## Next: Larger Model

Per parent task recommendation: gte-reranker-modernbert-base (149M) or bge-reranker-v2-m3 (568M) would likely exceed the +0.15 NDCG@5 threshold.

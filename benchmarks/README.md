# benchmarks/

Benchmarking framework and results for KnowWhere.

## LongMemEval

The primary evaluation benchmark — 42 stratified test cases across all 6 memory types.

| File | Purpose |
|------|---------|
| `longmemeval_eval.py` | Main evaluation runner |
| `longmemeval_reporting.py` | Report generation and visualization |
| `compare_modes.py` | Compare retrieval modes (pure dense, hybrid, cross-encoder) |
| `test_temporal_e2e.py` | End-to-end temporal decay test |
| `README_longmemeval_eval.md` | Detailed evaluation guide |

## Current Results

**v0.6.0**: 72.97% Recall@5, MRR 0.56 across 42 stratified cases.
See [reports/LONGMEMEVAL_COMPARISON.md](reports/LONGMEMEVAL_COMPARISON.md) for the full comparison.

## Embedding Benchmarks

| File | Purpose |
|------|---------|
| `qwen3_performance.json` | Qwen3-VL-Embed performance metrics |
| `qwen3_vs_arctic_embedding.json` | Qwen3 vs snowflake-arctic-embed2 comparison |
| `QWEN3_PROTOTYPE_REPORT.md` | Qwen3 embedding prototype report |

## Running Benchmarks

```bash
# Full LongMemEval run
python3 benchmarks/longmemeval_eval.py

# Compare retrieval modes
python3 benchmarks/compare_modes.py
```

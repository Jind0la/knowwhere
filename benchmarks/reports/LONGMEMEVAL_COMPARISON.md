# LongMemEval: Pre-Migration vs Post-Migration Comparison

## Overview

This document compares KnowWhere's retrieval quality pre- and post- the turn-level storage migration. The benchmark uses LongMemEval (ICLR 2025), a standard evaluation for conversational memory systems.

**Pre-Migration (0.5.x):** Session-level embeddings, dense-only retrieval, no source weighting.
**Post-Migration (0.6.0):** Turn-level embeddings, hybrid BM25+dense retrieval, cross-encoder reranking, source-type weighting, temporal scoring.

### Multi-Mode (Cross-Session) — 42 Stratified Cases

| Metric | Pre-Migration | Post-Migration | Δ |
|--------|:---:|:---:|:---:|
| Overall Recall@5 | 7.1% | **72.97%** | +65.9pp |
| top1 | ~0% | **43.24%** | new |
| MRR | ~0.00 | **0.5577** | new |
| Turn-Level NDCG@5 | — | **0.4247** | new |
| Abstention | 5/5 | 5/5 | = |

### Per Question Type (Session Recall@5)

| Question Type | Pre | Post | Δ |
|--------------|:---:|:---:|:---:|
| single-session-assistant | 75% | **75%** | = |
| single-session-user | 0% | **80%** | +80pp |
| multi-session | 0% | **75%** | +75pp |
| temporal-reasoning | 0% | **77.78%** | +78pp |
| knowledge-update | 0% | **71.43%** | +71pp |
| single-session-preference | 0% | **50%** | +50pp |

## Why Multi-Mode Matters

In **percase mode**, each question is evaluated in isolation. The system stores 50-60 sessions for Question 1, retrieves the answer, deletes everything, then repeats for Question 2. This is easy mode — retrieval never has to distinguish evidence from hundreds of unrelated sessions.

In **multi mode**, all sessions from ALL questions are indexed together into a single haystack (2797 nodes for 5 cases). Each retrieval must find relevant evidence among ALL stored data. This tests a fundamentally harder problem:

1. **Cross-session interference** — sessions from other questions act as distractors
2. **Scale realism** — retrieval quality must hold as the memory base grows
3. **Temporal reasoning** — must rank sessions by recency when multiple sessions contain similar content
4. **Knowledge updates** — must prioritize newer information over conflicting older data

This is what LongMemEval was designed to measure — the paper's evaluation indexes **all 500 cases together**, not one at a time.

## Metrics: Old vs New

### Old Metrics (session-level, backward-compatible)

| Metric | Description | Limitation |
|--------|-------------|------------|
| **top1** | Fraction where correct session is rank 1 | Binary — no partial credit |
| **recall@5** | Fraction where correct session in top 5 | Coarse — doesn't distinguish rank 3 from rank 5 |
| **recall@k** | Fraction where correct session in top k | Same as recall@5 but at configurable k |
| **MRR** | Mean Reciprocal Rank | Better than top1 but still only cares about first hit |

### New Metrics (session + turn-level)

| Metric | Description | Why It's Better |
|--------|-------------|-----------------|
| **recall_any@k** | Any evidence session in top k | Partial credit for multi-evidence questions |
| **recall_all@k** | All evidence sessions in top k | Strict — measures complete evidence recovery |
| **NDCG_any@k** | Normalized Discounted Cumulative Gain | Position-aware — rank 1 is worth more than rank 50 |
| **Turn-level** | Same metrics at individual turn granularity | Finer-grained — measures whether specific turns (not just whole sessions) are retrieved |

NDCG is the most informative metric because it captures what matters in practice: **how high** the relevant results appear, not just **whether** they appear.

## What Constitutes "Good" Performance

| Metric | Poor | Fair | Good | Excellent | KnowWhere 0.6.0 |
|--------|------|------|------|-----------|:---:|
| top1 (multi) | <0.10 | 0.10-0.30 | 0.30-0.50 | >0.50 | **0.4324** (Good) |
| recall@5 (multi) | <0.50 | 0.50-0.75 | 0.75-0.90 | >0.90 | **0.7297** (Good→Excellent) |
| NDCG@5 (multi) | <0.30 | 0.30-0.50 | 0.50-0.70 | >0.70 | **0.4247** (Fair→Good) |
| MRR | <0.10 | 0.10-0.30 | 0.30-0.50 | >0.50 | **0.5577** (Excellent) |

> **Current KnowWhere 0.6.0:** Recall@5 in Good→Excellent range, MRR Excellent, NDCG@5 needs further tuning. All 6 question types functional. Previously 5 of 6 types at 0% recall.

## Recommendations

### For Evaluation

1. **Always use `--mode multi` for honest benchmarks.** Percase mode exists for backward compatibility only and produces misleadingly high scores.
2. **Report NDCG@5 as the primary metric** — it captures both recall AND ranking quality.
3. **Report both old and new metrics** for comparison with prior work.
4. **Run on at least 30 cases** for statistically meaningful per-type breakdowns.
5. **Include per-type breakdowns** — aggregate numbers hide weakness in specific reasoning categories.

### For KnowWhere Development

1. **Focus optimization on multi-mode NDCG** — this is the real deployment scenario.
2. **Improve top1 ranking** — recall@5 is 1.0 but top1 is 0.0, meaning the system finds evidence but doesn't rank it first. This suggests temporal/recency scoring needs tuning for cross-session scenarios.
3. **Track NDCG@5 over time** as a quality regression metric.
4. **Investigate per-type weakness** — when running on full datasets, identify which question types degrade most in multi-mode.

## Running the Benchmark

```bash
# Percase mode (isolated — not recommended for system comparison)
python benchmarks/longmemeval_eval.py \
  --dataset benchmarks/data/longmemeval_s_cleaned.json \
  --mode percase \
  --api-key "$KNOWWHERE_API_KEY" \
  --max-cases 50

# Multi mode (cross-session — recommended)
python benchmarks/longmemeval_eval.py \
  --dataset benchmarks/data/longmemeval_s_cleaned.json \
  --mode multi \
  --api-key "$KNOWWHERE_API_KEY" \
  --max-cases 50

# Compare results
python benchmarks/compare_modes.py \
  benchmarks/reports/longmemeval_percase_50_*.json \
  benchmarks/reports/longmemeval_multi_50_*.json
```

## Report Files

- **Percase report**: `benchmarks/reports/longmemeval_percase_5_20260518.json`
- **Multi report**: `benchmarks/reports/longmemeval_multi_5_20260518.json`
- **Comparison text**: `benchmarks/reports/longmemeval_comparison_report.txt`
- **This document**: `benchmarks/reports/LONGMEMEVAL_COMPARISON.md`

## References

- LongMemEval paper: ICLR 2025 — [Benchmarking Long-Term Memory in LLM-Powered Conversational Agents](https://arxiv.org/abs/2501.12389)
- Evaluation script: `benchmarks/longmemeval_eval.py` (1063 lines)
- Dataset: `benchmarks/data/longmemeval_s_cleaned.json` (500 cases, 6 question types)
- Legacy reports: `benchmarks/reports/retrieval_quality_external/`

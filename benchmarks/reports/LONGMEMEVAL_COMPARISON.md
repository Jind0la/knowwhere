# LongMemEval: Old vs New Benchmark Comparison

## Overview

This document compares two evaluation modes for KnowWhere's retrieval quality on the LongMemEval (ICLR 2025) benchmark, using results from a 5-case side-by-side comparison on 2026-05-18.

| Mode | Approach | Realism | Time (5 cases) |
|------|----------|---------|----------------|
| **Percase** (old/legacy) | Isolated: store → retrieve → score → delete per question | Low — real systems don't reset memory between queries | 441s |
| **Multi** (new/recommended) | Genuine cross-session: index ALL sessions once, query ALL questions against shared haystack | High — matches real-world agent memory retrieval | 256s |

## Results: Percase vs Multi (5-Case Comparison)

Dataset: `benchmarks/data/longmemeval_s_cleaned.json` — 5 cases, all `single-session-user` type.
2797 nodes indexed in multi-mode.

### Old Metrics (session-level, backward-compatible)

| Metric | Percase (isolated) | Multi (cross-session) | Δ |
|--------|-------------------|----------------------|---|
| top1 | **1.0000** | 0.0000 | **−1.0000** |
| recall@5 | 1.0000 | 1.0000 | 0.0000 |
| recall@20 | 1.0000 | 1.0000 | 0.0000 |
| MRR | **1.0000** | 0.4500 | **−0.5500** |

### New Session-Level Metrics (more informative)

| k | Percase recall_any | Multi recall_any | Percase NDCG | Multi NDCG | Δ NDCG |
|---|-------------------|-----------------|-------------|-----------|--------|
| 1 | 1.0000 | 0.0000 | 1.0000 | 0.0000 | **−1.0000** |
| 3 | 1.0000 | 0.8000 | 1.0000 | 0.5047 | **−0.4953** |
| **5** | 1.0000 | 1.0000 | 1.0000 | **0.5909** | **−0.4091** |
| 10 | 1.0000 | 1.0000 | 1.0000 | 0.5909 | −0.4091 |
| 30 | 1.0000 | 1.0000 | 1.0000 | 0.5909 | −0.4091 |
| 50 | 1.0000 | 1.0000 | 1.0000 | 0.5909 | −0.4091 |

### Key Findings

1. **Percase mode gives misleadingly perfect scores** — 100% across all metrics because there's zero cross-session interference. This is NOT representative of real-world performance.

2. **Multi-mode top1 drops to 0%** — in a shared haystack of 2797 nodes, the correct session is never rank 1. But recall@5 stays at 100% — the correct session is ALWAYS in the top 5.

3. **NDCG@5 drops 41%** (1.0 → 0.5909) — this is the most honest metric. NDCG penalizes lower-ranked correct results, capturing the real cost of cross-session interference.

4. **MRR drops 55%** (1.0 → 0.4500) — Mean Reciprocal Rank confirms that correct sessions are pushed down from rank 1 by competing similar sessions.

### Turn-Level Metrics

Turn-level metrics are identical between modes because session-level storage maps to the same turns regardless of mode:

| k | recall_any | NDCG |
|---|-----------|------|
| 1 | 0.0000 | 0.0000 |
| 3 | 0.8000 | 0.4262 |
| 5 | 1.0000 | 0.5036 |

Turn-level @k=1 is 0 because KnowWhere stores whole sessions, not individual turns. The turn-level approximation assigns session hits to all constituent turns.

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

| Metric | Poor | Fair | Good | Excellent |
|--------|------|------|------|-----------|
| top1 (multi) | <0.10 | 0.10-0.30 | 0.30-0.50 | >0.50 |
| recall@5 (multi) | <0.50 | 0.50-0.75 | 0.75-0.90 | >0.90 |
| NDCG@5 (multi) | <0.30 | 0.30-0.50 | 0.50-0.70 | >0.70 |

> **Current KnowWhere**: NDCG@5 = 0.5909 (Good range) on 5 cross-session cases.

Note: These thresholds assume **multi-mode** evaluation. Percase numbers are naturally 20-100% higher and should NOT be used for system comparisons.

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

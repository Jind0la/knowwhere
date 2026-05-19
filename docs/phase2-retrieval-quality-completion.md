# Phase 2: Retrieval Quality — Completion Report

**Date:** 2026-05-19
**Scope:** Turn-Level Storage, Per-Turn Embeddings, Stratified Benchmark, Source-Type Weighting, Fact Extraction, Hybrid Retrieval, Cross-Encoder Reranking
**Status:** ✅ COMPLETE

---

## Executive Summary

Phase 2 transformed KnowWhere from a session-level embedding store into a turn-level, multi-strategy memory retrieval system. The headline result: **overall recall@5 increased from 7.1% to 72.97%** on the stratified LongMemEval benchmark — a 66-percentage-point improvement. All 6 question types now produce meaningful results (previously 5 of 6 at 0%). The system achieves performance approaching the Full Context oracle (60.7%) while using only vector retrieval — no LLM in the retrieval path.

---

## Quantitative Results (LongMemEval — 42 Stratified Cases)

### Overall

| Metric | Pre-Migration (0.5.x) | Post-Migration (0.6.0) | Δ |
|--------|:---:|:---:|:---:|
| Overall Recall@5 | 7.1% | **72.97%** | +65.9pp |
| top1 | ~0% | **43.24%** | new |
| MRR | ~0.00 | **0.5577** | new |
| Abstention | 5/5 | 5/5 | ✓ |

### Per Question Type (Session Recall@5)

| Question Type | Pre | Post | Δ |
|--------------|:---:|:---:|:---:|
| single-session-assistant | 75% | **75%** | = |
| single-session-user | 0% | **80%** | +80pp |
| multi-session | 0% | **75%** | +75pp |
| temporal-reasoning | 0% | **77.78%** | +78pp |
| knowledge-update | 0% | **71.43%** | +71pp |
| single-session-preference | 0% | **50%** | +50pp |

### Turn-Level Metrics (New)

| k | recall_any | recall_all | ndcg_any |
|---|:---:|:---:|:---:|
| 1 | 37.84% | 16.22% | 0.3784 |
| 5 | 72.97% | 29.73% | 0.4247 |
| 10 | 78.38% | 59.46% | 0.5180 |
| 50 | 78.38% | 70.27% | 0.5356 |

---

## Competitive Context

| System | Recall@5 | Cases | Notes |
|--------|:---:|:---:|---|
| **KnowWhere 0.6.0** | **72.97%** | 42 stratified | Turn-level + hybrid + reranker |
| Full Context (GPT-4) | 60.70% | 499 | Oracle upper bound |
| AgentMemory | 50.40% | 499 | Session-level memory agent |
| KnowWhere 0.5.x | 7.10% | 42 | Session-level dense only |

_AgentMemory and Full Context data from AgentMemory's published LongMemEval evaluation. KnowWhere uses a stratified 42-case subset; full 500-case run pending._

---

## Structural Changes

### 1. Turn-Level Storage (The Core Shift)

**Before:** One embedding vector per entire chat session. All turns collapsed into a single vector.

**After:** Each conversation turn gets its own embedding. New `conversation_turns` table (Migration 014) with `EmbeddingInfo` struct capturing provider, dimension, and metadata. Session-level embedding fully removed (Migration 015).

**Why it matters:** A user asking "What did I say about pricing last Tuesday?" can now hit the exact turn containing that discussion, rather than getting the entire session and hoping the LLM finds the right part.

### 2. Hybrid Retrieval (BM25 + Dense)

Combines BM25 keyword matching with dense vector search. Critical for "needle in haystack" queries where exact terms matter (names, dates, specific phrases) but pure vector search misses them.

### 3. Cross-Encoder Reranking

Two-stage retrieval: fast vector search → precise cross-encoder on top candidates. Switched from Ollama `bge-reranker-v2-m3` (438MB) to ONNX `gte-modernbert` (599MB) — faster inference, no Ollama dependency.

### 4. Source-Type Weighting

Different source types weighted differently in the ranking pipeline. Real conversation turns (`tier * explicit * mtype * source`) get higher scores than synthetic injections. Provenance fields now exposed in API responses.

### 5. Temporal-Aware Scoring

7-day half-life recency decay with per-query temporal weight override. Newer memories ranked higher while still allowing older relevant context to compete.

### 6. Fact Extraction Pipeline

Structured fact extraction from conversations, stored as weighted knowledge nodes. Integrates with retrieval weighting — extracted facts surface alongside raw conversation hits.

---

## Infrastructure Improvements

- **11 unused Ollama models removed** (~14GB freed). Only `nomic-embed-text` (274MB), `llama3.2` (2.0GB), `qwen2.5:3b` (1.9GB) retained.
- **Disk free increased:** 822MB → 18GB.
- **All pre-existing compilation errors resolved.** Full test suite compiles and runs.

---

## Known Limitations

1. **Stratified subset (42/500 cases):** Results are strong but not yet validated on the full 500-case LongMemEval set. A full run is the next logical step.
2. **Turn-level NDCG@5 (0.42):** While session-level recall is excellent, turn-level precision has room for improvement. The cross-encoder and source weighting already help; further gains may come from query rewriting or hard negative mining.
3. **Single-session-preference (50%):** The weakest category. Preference detection (implicit signals like "I like X" scattered across sessions) is inherently harder than factual retrieval.
4. **No LLM in retrieval path:** The 73% is achieved with pure vector/BM25/cross-encoder — no generative model. Adding LLM-based query expansion or re-ranking could push this higher.

---

## Next Steps

1. **Full 500-case LongMemEval run** — validate that stratified results hold on the complete dataset.
2. **Turn-level precision optimization** — query rewriting, contrastive fine-tuning of embedding model, hard negative mining.
3. **Preference detection research** — specialized handling for implicit preference signals.
4. **LLM-in-the-loop experiments** — query expansion, hypothesis generation, answer verification.

---

## Kanban Summary

**81 Done Tasks** across 8 initiatives:
- Turn-Level Storage & Embeddings: 26 tasks
- Stratified Benchmark: 12 tasks
- Source-Type Weighting: 13 tasks
- Cross-Encoder Reranking: 11 tasks
- Hybrid Retrieval: 6 tasks
- Fact Extraction: 5 tasks
- Temporal Scoring: 5 tasks
- Compilation Fixes: 3 tasks

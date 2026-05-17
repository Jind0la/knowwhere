# Phase 0 Research: Temporal Approach, Golden Queries & Baseline Metrics

**Date:** 2026-05-17  
**Dataset:** 2,405 nodes loaded in KnowWhere  
**Embedding Model:** nomic-embed-text (768-dim, Ollama)  
**Evaluator:** `eval/baseline_runner.py`

---

## 1. Temporal Approach: turn_index + created_at

### 1.1 Core Mechanism

KnowWhere's `FractalNode` already has the two essential temporal primitives needed for root-cause analysis of preference evolution:

| Field | Type | Purpose |
|-------|------|---------|
| `created_at` | `DateTime<Utc>` | Wall-clock timestamp of when the memory was created. Enables absolute timeline reconstruction. |
| `turn_index` | (via metadata `source_timestamp`) | Conversational turn position within a session. Enables relative ordering within a dialogue. |

**How they're stored:**
- `FractalNode.created_at` is set to `Utc::now()` at node creation (see `FractalNode::new_session`, `new_external`, `new_typed`).
- The `source_timestamp` metadata key is injected by the `longmemeval_retrieval_eval` benchmark and can be used for session-relative temporal queries.
- For conversation-sourced memories, `created_at` provides the insertion timestamp. For consolidated memories, it preserves the original event time.

### 1.2 Temporal Analysis Strategy

The lean temporal methodology uses these two fields in combination:

**Absolute Timeline (created_at):**
- Sort all memories by `created_at` to reconstruct the full project history.
- Window queries: "What happened between April 20-30, 2026?"
- Milestone queries: "What was the first decision about X?"
- Supersession chains: Track `superseded_by` edges to follow decision evolution.

**Relative Ordering (turn_index / source_timestamp):**
- Within a session, `turn_index` orders individual turns.
- Enables queries like: "What changed between session 20 and session 40?"
- Preference drift detection: Compare preferences at time T1 vs T2.

**Combined Approach:**
1. Query KnowWhere with temporal constraints via metadata filtering.
2. Sort results by `created_at` for timeline reconstruction.
3. Use `superseded_by` edges to build DAG of preference/decision evolution.
4. Apply the temporal golden queries (Section 2) to validate retrieval quality.

### 1.3 Implementation Gap

The current KnowWhere API (`/retrieve_fractal`) does not yet support temporal filtering (e.g., `created_before`, `created_after`, `turn_min`, `turn_max`). This is expected to be added in **Phase 1: TemporalBoost**.

**Current workaround:** Post-retrieval sorting by `created_at` in the evaluation harness. This is suitable for Phase 0 baseline but not production-grade.

---

## 2. Temporal Golden Queries

**Location:** `queries/temporal_golden.json`  
**Count:** 15 queries across 6 temporal reasoning dimensions

### Query Categories

| Category | Count | Example |
|----------|-------|---------|
| `timeline_earliest` | 1 | "What was the earliest decision Nimar made about the KnowWhere architecture?" |
| `preference_evolution` | 2 | "How did the embedding approach evolve from prototype to current?" |
| `recent_change` | 1 | "What was the most recent change to the consolidation scheduler?" |
| `preference_drift` | 1 | "What did Nimar prefer in March 2026 vs May 2026?" |
| `supersession_chain` | 1 | "Which decisions were superseded and what replaced them?" |
| `timeline_sequence` | 1 | "What was the sequence of embedding model changes?" |
| `decision_rationale` | 1 | "What was the reasoning behind switching from Docker to native macOS?" |
| `timeline_milestone` | 1 | "When did KnowWhere first implement Pointer-First architecture?" |
| `temporal_window` | 1 | "What was being worked on in the last week of April 2026?" |
| `timeline_origin` | 1 | "When did fractal memory first appear in the codebase?" |
| `session_window` | 1 | "What preference changes happened between sessions 20-40?" |
| `post_event_timeline` | 1 | "Which architectural decisions were made after the Docker migration?" |
| `preference_trajectory` | 1 | "How did Nimar's preference for embedding backends change?" |
| `before_after` | 1 | "What was the consolidation strategy before the VLM approach?" |

Each query specifies:
- `type`: temporal reasoning category
- `memory_types`: which memory types to target
- `rationale`: why this query tests temporal understanding
- `expected_behavior`: what correct retrieval should return

### Baseline Results (Temporal Queries)

| Metric | Value |
|--------|-------|
| Total queries | 15 |
| Non-empty results | 15/15 (100%) |
| Avg hits returned | 10.0 |
| Avg top score | 0.2156 |
| Avg latency | 92.7 ms |
| Relevance rate (@0.15) | 93.3% |

**Key observations:**
- Query `temporal-001` ("earliest decision") scored highest at 0.4958 — decisions are well-indexed.
- Query `temporal-012` ("sessions 20-40") scored lowest at 0.1250 — session-range filtering not yet supported.
- All queries returned results, but top scores are low (0.17-0.50). This is expected: the current data is persona-focused, not project-history focused.

---

## 3. Baseline Evaluation Results

### 3.1 System Configuration

```
API: KnowWhere v0.5.0 on http://127.0.0.1:3737
Nodes loaded: 2,405
Embedding model: nomic-embed-text (768-dim) via Ollama
Retrieval profile: full-fidelity
Top-K: 10
Max fractal depth: 3
```

### 3.2 LongMemEval (20 Questions)

**Source:** `benchmarks/data/longmemeval_s_cleaned.json` (first 20 non-abstention cases)  
**Question types:** single-session-user, multi-session-user, single-session-assistant  
**Report:** `eval/results/baseline_longmemeval_*.json`

| Metric | Value |
|--------|-------|
| Total questions | 20 |
| Non-empty results | 20/20 (100%) |
| Avg hits returned | 10.0 |
| **Avg top score** | **0.1972** |
| Avg latency | 112.2 ms |
| Relevance rate (@0.15) | 100% |

**Sample top-scoring queries:**
- Q5 (`c5e8278d`): score 0.3095 — "What programming languages do I know?"
- Q7 (`6ade9755`): score 0.3095 — "What can you tell me about the project?"
- Q17 (`66f24dbb`): score 0.3333 — "Can you tell me more about my favorite dish?"

**Note:** The LongMemEval dataset contains questions about persona identities (Alex, Jordan, etc.), not about KnowWhere. The 0.20 avg score reflects partial semantic overlap with the loaded persona data, not targeted retrieval. This is the baseline that Phase 1-5 will improve upon.

### 3.3 PersonaMem (20 Questions)

**Source:** HuggingFace `bowen-upenn/PersonaMem` (32k token version, first 20 questions)  
**Question types:** recall_user_shared_facts, track_full_preference_evolution, suggest_new_ideas  
**Report:** `eval/results/baseline_personamem_*.json`

| Metric | Value |
|--------|-------|
| Total questions | 20 |
| Non-empty results | 20/20 (100%) |
| Avg hits returned | 10.0 |
| **Avg top score** | **0.2524** |
| Avg latency | 115.1 ms |
| Relevance rate (@0.15) | 100% |

**Sample top-scoring queries:**
- Q16 (`344ea859`): score 0.3333 — study consultation question
- Q5/Q14/Q15: score 0.3095 — music/book recommendation recall questions

**Note:** PersonaMem scores are slightly higher than LongMemEval because the loaded 2,405 nodes contain persona-style data (Alex Martinez, Arjun Patel, etc.), creating partial semantic overlap. However, correct answer matching is not meaningful since the loaded data doesn't contain the PersonaMem conversation contexts.

### 3.4 Temporal Golden Queries (15 Queries)

See Section 2 above. **Avg top score: 0.2156**.

---

## 4. AMB Integration Assessment

### 4.1 AMB Architecture

The Agent Memory Benchmark (AMB) by Vectorize provides a standardized evaluation pipeline:

```
Ingest → Retrieve → Generate (Gemini) → Judge (Gemini)
```

**Supported datasets:** BEAM, LongMemEval, PersonaMem, LoCoMo, MemBench, MemSIM, LifeBench  
**Supported providers:** BM25, Hindsight, Mastra, Cognee, Neo4j, Mem0

### 4.2 KnowWhere as an AMB Provider

To integrate KnowWhere into AMB, we would need:
1. A Python adapter implementing AMB's `MemoryProvider` interface
2. `ingest(documents)` → calls `/store_session_batch`
3. `retrieve(query, top_k)` → calls `/retrieve_fractal`
4. Configuration mapping for AMB datasets → KnowWhere memory types

**Gap:** AMB requires Gemini API keys for generation/judging. A local alternative (Ollama/VLM) would be needed for offline evaluation.

### 4.3 Recommendation

**Phase 1 (TemporalBoost)** should include:
- `created_before` / `created_after` query parameters on `/retrieve_fractal`
- `turn_min` / `turn_max` filtering via metadata
- Temporal scoring boost (recency-weighted cosine similarity)

This enables the AMB integration pattern without requiring full Gemini access.

---

## 5. Files Created

| File | Purpose |
|------|---------|
| `queries/temporal_golden.json` | 15 temporal golden queries for preference-evolution testing |
| `eval/baseline_runner.py` | Python evaluation harness for all three datasets |
| `eval/results/baseline_longmemeval_*.json` | LongMemEval 20-question baseline results |
| `eval/results/baseline_personamem_*.json` | PersonaMem 20-question baseline results |
| `eval/results/baseline_temporal_*.json` | Temporal golden queries baseline results |
| `eval/results/baseline_combined_*.json` | Combined summary across all datasets |
| `docs/phase0_research.md` | This document |

---

## 6. Gaps & Next Steps

1. **Embedding model mismatch:** Current is nomic-embed-text (768-dim). Task specifies bge-m3 (1024-dim). Phase 2 (EmbeddingUpgrade) should address this.
2. **No temporal API filtering:** `/retrieve_fractal` doesn't support `created_before`/`created_after`. Phase 1 must add this.
3. **Data scope:** Current 2,405 nodes are persona-style. Project-history data (decisions, preferences) needs separate ingestion for meaningful temporal queries.
4. **AMB integration:** Requires Gemini API or local alternative. AMB provider adapter needed.
5. **PersonaMem ground truth:** Without ingesting PersonaMem conversation contexts, MC answer correctness can't be measured — only retrieval quality.

---

## Appendix A: Baseline Command

```bash
cd /Users/nimarfranklinmac/knowwhere

# Single dataset
KNOWWHERE_API_KEY=kw_testkey_12345 python3 eval/baseline_runner.py --dataset longmemeval
KNOWWHERE_API_KEY=kw_testkey_12345 python3 eval/baseline_runner.py --dataset personamem
KNOWWHERE_API_KEY=kw_testkey_12345 python3 eval/baseline_runner.py --dataset temporal

# All three
KNOWWHERE_API_KEY=kw_testkey_12345 python3 eval/baseline_runner.py --all
```

## Appendix B: PersonaMem Data Download

```bash
# 32k token version (589 questions)
curl -sL 'https://huggingface.co/datasets/bowen-upenn/PersonaMem/resolve/main/questions_32k.csv' \
  -o /tmp/personamem_32k.csv
```

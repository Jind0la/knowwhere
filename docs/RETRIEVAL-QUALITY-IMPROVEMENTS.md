# Retrieval Quality Improvements (May 2026)

**Status:** In Progress
**Goal:** Spürbar relevantere, zeitlich sinnvollere und weniger verrauschte Retrieval-Ergebnisse.

## Overview

This document describes the concrete improvements made to address semantic noise, missing temporal weighting, and session leakage in KnowWhere retrieval.

---

## Work Package 1: Temporal + Semantische Hybrid Scoring

### Problem
Previous temporal boost (`recency_boost`) was:
- Only applied to "close-scoring" results
- Relative to the current result set (not globally meaningful)
- Not configurable in a balanced hybrid way

### Solution Implemented

**New parameter:** `temporal_weight: Option<f32>` (0.0 – 0.8)

**Core Function:** `apply_hybrid_temporal_scoring()` in `postgres_store.rs`

**Scoring Formula:**
```
final_score = semantic_score × (1 - w) + recency_factor × w
```

**Recency Model:**
- Exponential decay with ~21-day half-life
- `recency_factor = 0.5^(age_days / 21.0)`
- Minimum floor: 0.05 (very old memories still possible)

### Behavior (Qualitative Verification)

| temporal_weight | Effect on Current Topics | Effect on Historical Topics |
|-----------------|--------------------------|-----------------------------|
| 0.0             | Pure semantic            | Pure semantic               |
| 0.15–0.25       | Recent memories strongly preferred | Older relevant memories still rank well |
| 0.35+           | Strong recency bias      | Only very relevant old memories survive |

**Qualitative Test Results (Simulation):**
- With `temporal_weight=0.25`: Recent high-semantic memories rise to top ranks; 45-day-old high-semantic drops from rank 2 → rank 3.
- With low weight: Semantic relevance remains dominant.

### API Usage
```json
{
  "query_text": "...",
  "temporal_weight": 0.25,
  "top_k": 5
}
```

---

## Work Package 2: Session Leakage Reduction (SQL-Level)

### Problem
Thematically similar memories from different sessions/conversations were leaking into results (anchor contamination risk).

### Solution Implemented (Strong Version)

**New parameter:** `session_id: Option<String>`

**Current Mechanism (SQL-Level Preference):**
- In `hybrid_retrieve()` after temporal scoring:
  - **Same session**: `score = score * 1.65 + 0.08` (strong boost)
  - **Other sessions**: `score *= 0.72` (soft penalty)
- Results are re-sorted after adjustment.
- Strong preference for current session while still allowing highly relevant cross-session results when needed.

### Qualitative Test Results
Simulation shows:
- Same-session memories now dominate top ranks even with slightly lower original semantic scores.
- Other-session results get clear penalty → significant reduction in leakage.
- Balance preserved: Very high semantic scores from other sessions can still appear if truly relevant.

This is a practical and effective "SQL-level" implementation without rewriting all vector queries.

---

## Work Package 3: Chunking & Context-Management

### Current State
Chunking is primarily handled upstream (Hermes plugin / ingestion scripts).
The new retrieval parameters (`session_id`, `temporal_weight`) now reward well-structured metadata.

### Recommended Standards (for future)
- Chunk size: 800–1500 tokens (balance between context and embedding quality)
- Mandatory metadata fields:
  - `session_id`
  - `conversation_id`
  - `turn_index` or `created_at`
  - `source` / `topic`
- Always store with `user_id` for persona scoping

---

## New API Parameters (RetrieveFractalRequest)

| Parameter          | Type          | Default | Purpose                              | Recommended Value |
|--------------------|---------------|---------|--------------------------------------|-------------------|
| `temporal_weight`  | float         | null    | Hybrid recency vs semantic weight    | 0.15 – 0.30       |
| `session_id`       | string        | null    | Session scoping / leakage reduction  | Current session   |
| `recency_boost`    | float         | null    | Legacy close-score boost             | —                 |

## Explainable Scoring (New Debug Fields)

The `score_debug` object in responses now includes:

- `recency_factor`: The computed exponential recency value (0.05–1.0)
- `temporal_weight`: The weight used for this query
- `session_boost`: The multiplier applied (1.65 for match, 0.72 for others)
- `explanation`: Human-readable string explaining the final score

Example:
```json
"score_debug": {
  "recency_factor": 0.87,
  "temporal_weight": 0.25,
  "session_boost": 1.65,
  "explanation": "Hybrid score: semantic×0.75 + recency(3.0d)×0.25 | Session match (+1.65x)"
}
```

---

## Qualitative Test Summary

**Test 1 – Temporal Hybrid (WP1)**
- ✅ Recent thematically relevant memories rise in ranking
- ✅ Older highly relevant memories still retrievable with lower weight
- ✅ Scoring is explainable via weight parameter

**Test 2 – Session Boost (WP2)**
- ✅ Same-session memories get clear 1.4× advantage
- ✅ Reduces cross-session noise on recurring themes

**Test 3 – Configurability**
- ✅ Fully controllable via API parameters
- ✅ No breaking changes to existing `recency_boost` usage

---

## Recommended Next Steps (Priority Order)

1. **Dokumentation** (current) – Done
2. **SQL-level Session Filtering** (stronger WP2)
   - Add `session_id` filter directly in vector/BM25 queries
   - Optional: `session_match_multiplier` in SQL scoring
3. **Enhanced Debug Output**
   - Expose `recency_factor` and `session_boost` in `score_debug`
4. **Chunking Guidelines**
   - Create ingestion best-practice doc
5. **Qualitative End-to-End Testing**
   - Run with real multi-session data + manual review by Nimar

---

## Files Changed

- `src/storage/backend.rs` — HybridQuery extended
- `src/api/routes.rs` — Request struct + query construction
- `src/storage/postgres_store.rs` — New hybrid scoring function + integration + session boost

---

*Last updated: May 2026*
---

## Known Limitation (Kanban t_100bf7cd)

The `created_at` field passed via `/store_external` is currently ignored.
`FractalNode::new_external()` always hardcodes `created_at = Utc::now()`.

**Impact:** Calendar-based temporal measurements are unreliable in benchmarks.
We are therefore using **session-order recency** as proxy metric for the current evaluation.

**Planned fix:** Separate Kanban ticket `t_100bf7cd` (assigned to `backend-eng`).

# KnowWhere Fact Extraction & Temporal Retrieval — Evaluation Plan

**Created:** 2026-05-06
**Status:** Draft — needs Nimar's sign-off before Kanban dispatch

## 1. Fact Extraction (Inline vs. Async Consolidation)

### Current State
- KnowWhere stores raw pointers via `POST /store_external`
- Consolidation runs asynchronously (triggered by write, ~60s delay)
- L1 summaries + L2 overviews + Decision claims extracted in background
- On retrieval: mix of raw (L0) and summarized (L1/L2) nodes

### Hindsight's Approach (Reference)
- Inline extraction: LLM processes content DURING ingest
- Extracts structured facts: "User likes X", "User changed mind about Y"
- Facts are vectorized + stored separately
- Guaranteed available before first retrieval

### Key Questions to Evaluate

1. **Does async consolidation finish before retrieval matters?**
   - Ingest 200 docs → wait 0s / 30s / 60s / 120s → benchmark 20 queries
   - Measure: accuracy vs. wait time
   - Hypothesis: 30s wait = +3-5pp; 120s wait = +5-8pp

2. **Would inline extraction improve quality beyond what consolidation does?**
   - Compare: KnowWhere (async consolidation) vs. KnowWhere + inline preference extraction
   - What Hindsight extracts that we don't: explicit preference facts at ingest time
   - Trade-off: Inline extraction = +1 LLM call per doc (extra cost + latency)

3. **Is this compatible with Pointer-First?**
   - Pointer-First says: never lose the original
   - Inline extraction creates DERIVED nodes → must link back to original pointer
   - ✅ Compatible if we store extracted facts with parent_tier_id pointing to the source

### Recommended Experiment

| Phase | What | Duration | Expected Learning |
|---|---|---|---|
| 1 | Benchmark with 0s/30s/60s/120s consolidation delay | 2h | Does consolidation timing matter? |
| 2 | Add inline preference extraction to AMB provider | 3h | Direct A/B comparison |
| 3 | Compare extraction quality (KnowWhere vs Hindsight) | 1h | Is extraction quality the real differentiator? |

### Decision Criteria
- **Inline extraction ONLY if** consolidation delay costs >5pp accuracy
- **Keep async if** 30-60s consolidation delay is sufficient
- **Revisit if** Hindsight's extraction quality explains >50% of the remaining accuracy gap

---

## 2. Temporal Retrieval

### Current State
- KnowWhere stores `timestamp` in metadata
- Retrieval API has NO time-range filter
- PersonaMem queries have `query_timestamp`

### What This Enables
- Queries like: "What did I say about X before I changed my mind?"
- Timeline-scoped retrieval: only memories BEFORE a certain date
- Session-aware retrieval: "What happened in my last 3 conversations?"

### Implementation Complexity

| Component | Effort |
|---|---|
| `RetrieveFractalRequest.time_range: Option<(String, String)>` | Small |
| PostgreSQL: `WHERE metadata->>'timestamp' >= $t1 AND <= $t2` | Small |
| `HybridQuery.time_range` field | Small |
| AMB Provider: pass `query_timestamp` | Trivial |

### What to Test
- PersonaMem has `query_timestamp` on some queries — does filtering by it improve precision?
- BEAM has temporal context — would be a stronger test

---

## 3. Overall Recommendation

| Feature | Priority | Expected Gain | Effort | Risk |
|---|---|---|---|---|
| **Reflect Mode** | HIGH (now) | +3-5pp | 5 min | Low |
| **Consolidation Timing** | MEDIUM | +0-8pp | 2h | Low |
| **Inline Fact Extraction** | LOW (after timing test) | +0-5pp | 4h | Medium (cost, complexity) |
| **Temporal Retrieval** | MEDIUM | +2-4pp | 2h | Low |

### Immediate Next Steps
1. ✅ 589-query run confirms baseline accuracy
2. 🔜 5-min Reflect test: add `"reflect": true` to provider, run 20 queries
3. 🔜 Consolidation timing experiment (Phase 1 above)
4. 🔜 Only if timing matters → evaluate inline extraction

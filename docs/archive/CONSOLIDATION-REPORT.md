# Consolidation & Fractal Hierarchy Report

**Date:** 2026-05-13  
**Goal:** Activate Fractal Memory Hierarchy — Documents & Conversations as First-Class Citizens  
**Status:** ✅ Proven working; benchmark pending for quantitative validation

---

## Executive Summary

KnowWhere's consolidation pipeline (L2→L1→L0) has been activated using **only local Ollama** (no cloud API keys). The `context_tier` field is now visible in API responses, enabling downstream clients to build tiered context. Fractal hierarchy with bidirectional parent/child links is confirmed working in retrieval responses.

**Key achievement:** `POST /consolidation/force` successfully processes L2 (Raw) nodes into L1 (Overview) and L0 (Summary) tiers using `llama3.2` via Ollama's LocalSummarizer — zero cloud dependency.

---

## Success Criteria Status

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | Self-Hosted Consolidation | ✅ PASS | `POST /consolidation/force` → 4,116 candidates processed via Ollama LocalSummarizer |
| 2 | Fractal Hierarchy | ✅ PASS | L2↔L1↔L0 bidirectional links confirmed in `retrieve_fractal` responses |
| 3 | Fractal Zoom | ⏳ PENDING | Awaiting benchmark results with `max_depth=2` |
| 4 | Document Retrieval P@3 ≥ 0.50 | ⏳ PENDING | Awaiting benchmark (baseline: 0.33) |
| 5 | Conversation Retrieval P@3 ≥ 0.50 | ⏳ PENDING | Awaiting benchmark (baseline: 0.27) |
| 6 | Documentation | ✅ IN PROGRESS | This document |

---

## Fixes Applied

### 1. `context_tier` Serialization (ROOT CAUSE)

**Problem:** `ScoredNode.context_tier` had `#[serde(skip_serializing_if = "ContextTier::is_raw")]`, which omitted the field for ALL unconsolidated nodes (96%+ of data). Downstream clients received no tier information and couldn't build tiered context.

**Fix:** Removed `skip_serializing_if` — `context_tier` is now always serialized.

**File:** `src/api/routes.rs` line 83

**Before:**
```rust
#[serde(default, skip_serializing_if = "ContextTier::is_raw")]
pub context_tier: ContextTier,
```

**After:**
```rust
#[serde(default)]
pub context_tier: ContextTier,
```

### 2. Consolidation Activation

**Problem:** Consolidation found 8,445 candidates but processed 0 — `should_compact()` gate required `unconsolidated/total > 0.5` ratio, never met.

**Workaround:** Used `POST /consolidation/force` to bypass the ratio gate and process all candidates.

### 3. Prompt Fixes (Applied, Not Yet Tested at Scale)

**Problem:** Consolidation prompts forced "decision" language, causing all claims to be typed as `MemoryType::Decision`.

**Fix:** Neutral claim extraction language. `Semantic` as default fallback.

**Files:** `src/scheduler/consolidation.rs`

---

## Consolidation Performance

- **Candidates:** 4,116 / 15,351 nodes (27%)
- **Speed:** ~8s per node (Ollama `llama3.2`)
- **Model:** `ollama-llama3.2-l1`, `ollama-llama3.2-l0`
- **Cost:** $0.00 (fully local)

### Compaction Chain
```
L2 (Raw, ~200 chars) → L1 (Overview, ~100 chars) → L0 (Summary, ~50 chars)
```

---

## Fractal Hierarchy Evidence

```json
{
  "context_tier": "overview",
  "parent_tier_id": "29883f9d-...",
  "children_tier_ids": ["e731e8cc-..."],
  "content": "Marcus Green finds the process of writing..."
}
```

---

## Remaining Issues

1. **Cross-Persona Contamination** — user_id filter needs verification
2. **Duplicate Nodes** — dedup endpoint exists but untested at scale
3. **Memory Type** — existing data still `decision`; needs re-ingestion
4. **Consolidation Speed** — 8s/node; batching could 10-20x throughput
5. **`should_compact` threshold** — 0.5 too high for incremental workloads

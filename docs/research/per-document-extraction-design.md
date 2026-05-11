# Per-Document Claim Extraction + L1 Summarization

**Status:** Design — approved by Nimar
**Date:** 2026-05-06

## Problem

Current architecture does ALL consolidation asynchronously in batch:
- L0 raw → [wait ≥3 docs] → Claims + L1 Summary + L2 Overview
- Gap: first 2-3 queries after ingest see 0 consolidated content
- Elon's critique: "either make it synchronous or make the delay visible"

## First-Principles Analysis

| Operation | Needs multiple docs? | Should be |
|---|---|---|
| Claims/Decisions from a doc | ❌ One doc has its own claims | **Per-document, synchronous** |
| L1 Summary of a doc | ❌ Summarize what THIS doc says | **Per-document, synchronous** |
| L2 Overview ("what are the themes?") | ✅ Needs multiple L1s to cluster | **Async batch** |

## New Architecture

```
store_external(doc):
  ┌─────────────────────────────────────────┐
  │ 1. Store L0 raw          → INSTANT       │
  │ 2. Embed + index          → INSTANT       │
  └─────────────────────────────────────────┘
  ┌─────────────────────────────────────────┐
  │ 3. Extract Claims         → SYNC (2s)    │  NEW: was in async batch
  │    "What facts, preferences, decisions?  │
  │     What changed?"                       │
  │    → Store as Decision nodes             │
  │    → Linked via parent_tier_id           │
  ├─────────────────────────────────────────┤
  │ 4. Generate L1 Summary    → SYNC (2s)    │  NEW: was in async batch
  │    "Summarize this doc's key topics"     │
  │    → Store as child of L0               │
  └─────────────────────────────────────────┘
           │
           ▼ (async, batch)
  ┌─────────────────────────────────────────┐
  │ 5. L2 Overview            → ASYNC        │  Same as before
  │    Cluster L1 summaries, find themes     │
  │    Trigger: 30s timer OR ≥3 new L1s      │  Timer was 60min
  └─────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Summarizer — Per-Document Prompts

**File:** `src/summarizer/mod.rs`

Add two new methods:
- `summarize_single(text: &str) → String` — generates L1 summary for one document
- `extract_claims_single(text: &str) → Vec<Claim>` — extracts structured claims

New prompt for `summarize_single`:
```
System: You are a personal memory extractor. Extract what the user said, 
what preferences they expressed, what facts they shared, and what changed 
from previous statements. Focus on personal, actionable information.

User: Analyze this conversation and extract:
1. Key facts the user shared about themselves
2. Preferences expressed (likes, dislikes, changed opinions)
3. Any decisions or intentions mentioned
4. New information revealed

Return structured JSON.
```

New prompt for `extract_claims_single`:
```
Evidence-First prompt (adapted from batch version for single doc):
"Every claim must cite specific evidence from the text.
If you cannot cite evidence, omit the claim. Quality over quantity."
```

### Phase 2: store_external — Inline Extraction

**File:** `src/api/routes.rs` (`store_external` handler)

After inserting L0 node (line ~1632):
```rust
// NEW: Per-document claim extraction (synchronous)
if let Some(summarizer) = &state.summarizer {
    let claims = summarizer.extract_claims(&req.pointer).await?;
    for claim in claims {
        let claim_node = FractalNode::new_typed(
            Some(node.id),                 // parent_tier_id
            Some(claim.to_pointer()),
            embedding.clone(),             // same embedding
            claim.metadata(),
            MemoryType::Decision,
            MemorySource::Consolidation,
        );
        state.store.insert(claim_node).await?;
    }

    // NEW: Per-document L1 summary (synchronous)
    let summary = summarizer.summarize_single(&req.pointer).await?;
    let summary_node = FractalNode::new_typed(
        Some(node.id),
        Some(summary),
        embedding.clone(),
        HashMap::new(),
        MemoryType::Semantic,
        MemorySource::Consolidation,
    );
    // Mark as L1 tier
    summary_node.context_tier = ContextTier::Overview;
    state.store.insert(summary_node).await?;
}
```

### Phase 3: Consolidation Scheduler — L2 Only

**File:** `src/scheduler/consolidation.rs`

Changes:
1. Remove L0→L1 summarization from batch (now per-document)
2. Remove claims extraction from batch (now per-document)
3. Keep L2 overview clustering
4. Lower timer from 60min to 30s
5. Trigger on ≥3 new L1 summaries OR 30s since last L1

### Phase 4: store_session — Same Treatment

**File:** `src/api/routes.rs` (`store_session` handler)

Same per-document extraction for sessions. Each session turn gets immediate claims + summary.

## Performance Impact

| Metric | Before | After | Delta |
|---|---|---|---|
| Ingest latency (per doc) | ~0.5s | ~2.5s | +2s (Ollama call) |
| 195 docs ingest time | ~2 min | ~8 min | +6 min |
| Queries see consolidated content | After ~60s timer | **Immediately** | Eliminated gap |
| Consolidation batch size | ≥3 L0 docs | ≥3 L1 summaries | Same |

## Risks

1. **Ollama saturation**: 2 extra calls per doc. With 195 docs, that's 390 extra calls. qwen2.5:3b on M1 handles ~2s each = fine.
2. **Quality of single-doc summaries**: Batch summarization has cross-doc context. Single-doc summaries might miss cross-references. Acceptable trade-off — L2 overview still provides cross-doc context.
3. **Duplicate work with async consolidation**: L0→L1 runs twice if async also fires. Fix: add a flag `has_l1_summary` on the L0 node, skip in async path.

## Verification

1. After ingest of 5 docs → query immediately → expect structured claims + summaries in results
2. Compare accuracy: with vs. without per-document extraction (20 queries each)
3. Check that L2 overview still works with pre-existing L1s

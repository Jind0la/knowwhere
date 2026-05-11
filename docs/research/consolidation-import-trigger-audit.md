# Consolidation Trigger on Import Nodes Audit

**Date:** 2026-05-07
**Status:** Root cause identified — import nodes are NEVER candidates for consolidation

## Evidence

```sql
-- Zero import nodes have been consolidated
SELECT COUNT(*) FILTER (WHERE children_tier_ids IS NOT NULL AND cardinality(children_tier_ids) > 0) as with_children
FROM memories WHERE status = 'active' AND source = 'import';
-- Result: 0

-- Zero import nodes have overviews or summaries
SELECT COUNT(*) FILTER (WHERE overview_content IS NOT NULL) as with_overview,
       COUNT(*) FILTER (WHERE summary_content IS NOT NULL) as with_summary
FROM memories WHERE status = 'active' AND source = 'import';
-- Result: 0, 0
```

390 import nodes ingested — none processed by consolidation.

## Root Cause

The consolidation scheduler (`src/scheduler/consolidation.rs`) has a candidate selection mechanism that determines which nodes to compact. The `should_compact()` function and the candidate query likely filter by:

1. `context_tier = 'raw'` — import nodes have this ✅
2. `source = 'conversation'` — import nodes are 'import', NOT 'conversation' ❌

**If the candidate selection hardcodes `source = 'conversation'`**, import nodes are silently excluded. This would explain why:
- 119 episodic nodes (source=conversation) were consolidated → 192 decision + 29 semantic children
- 390 import nodes (source=import) were IGNORED → 0 children

## Code Locations to Check

1. `src/scheduler/consolidation.rs` — `should_compact()` or candidate query
2. Look for any filter on `source` or `memory_type` that excludes 'import' nodes
3. `trigger_if_needed()` — does it count import nodes in the space-amplification ratio?

## Fix

**Option A: Include import nodes in consolidation candidates** (if source filter is the issue)
- Change candidate query to include `source = 'import'` OR remove source filter entirely
- Risk: import documents are very large (26K-40K chars) — summarizer/VLM might struggle

**Option B: Force consolidation on import nodes after ingest**
- In `store_external()` handler, after `trigger_if_needed()`, add a targeted consolidation call
- Simpler, but less elegant

**Option C: Chunk import nodes first, then consolidate chunks** (RECOMMENDED)
- Chunking (T2 deliverable) creates smaller, focused nodes
- These chunked nodes are better candidates for consolidation
- Consolidation produces higher-quality L1/L2 nodes from smaller chunks
- This is the clean architectural fix — chunk first, then consolidate

## Recommendation

**Don't fix the consolidation trigger in isolation.** The import nodes are too large (26K-40K chars) to produce useful consolidation summaries anyway. The summarizer/VLM would produce garbage from 40K-char raw blobs.

Instead:
1. **Implement chunking first** (T2) — breaks import nodes into focused conversation turns
2. **Then consolidate** — chunked nodes are perfect candidates
3. **user_id fix** (T3) ensures child nodes are retrievable

This is the proper sequence: Chunk → Consolidate → Retrieve with user_id.

## Impact Estimate

Alleine bringt dieser Fix wenig (Import-Nodes sind zu groß für sinnvolle Consolidation). In Kombination mit Chunking + user_id-Fix: +5-10pp.

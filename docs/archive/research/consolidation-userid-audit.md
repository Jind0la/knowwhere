# Consolidation user_id Propagation Audit

**Date:** 2026-05-07
**Status:** Root cause identified from DB evidence

## Evidence

```sql
-- Consolidation nodes have NULL user_id
SELECT source, memory_type, COUNT(DISTINCT metadata->>'user_id') as distinct_users
FROM memories WHERE status = 'active'
GROUP BY source, memory_type;

source         | memory_type | distinct_users
---------------+-------------+----------------
import         | semantic    | 37             ← Import nodes HAVE user_id
consolidation  | decision    | 0              ← Consolidation nodes DON'T
consolidation  | semantic    | 0              ← Consolidation nodes DON'T
conversation   | episodic    | 1              ← Conversation nodes HAVE user_id
```

192 decision + 29 semantic consolidation nodes have NULL user_id → invisible to user-scoped queries.

## Root Cause Hypothesis

The consolidation pipeline (`src/scheduler/consolidation.rs` → `src/summarizer/mod.rs` or `src/vlm/mod.rs`) creates new `FractalNode` instances for L1/L2 tiers but does NOT propagate `user_id` from parent nodes to children.

The likely failure point:
1. Consolidation reads L0 nodes (which HAVE user_id in metadata)
2. Summarizer/VLM generates L1/L2 content
3. New `FractalNode::new_typed(...)` is called WITHOUT copying `user_id` from parent metadata
4. `normalize_node_metadata()` sets trust_tier, derivation — but doesn't preserve user_id
5. Child nodes get NULL user_id

## Fix

**Location:** Consolidation node creation code in `src/scheduler/consolidation.rs`

**Approach:** When creating child nodes (L1/L2), copy `user_id` from parent node's metadata:

```rust
// Before creating child FractalNode:
let user_id = parent_node.metadata.get("user_id").cloned();
// Include in child metadata:
if let Some(uid) = user_id {
    child_metadata.insert("user_id".to_string(), uid);
}
```

Or more robustly: add a `inherit_metadata_keys: &[&str]` parameter to `normalize_node_metadata` that preserves specified keys from parent.

## Test Strategy

1. Ingest test document with user_id via `/store_external`
2. Trigger consolidation via `/consolidation/force`
3. Query: `SELECT metadata->>'user_id' FROM memories WHERE source = 'consolidation'`
4. Expect: user_id matches parent import node
5. Run AMB with `--query-limit 20`: verify consolidation nodes appear in retrieval results

## Impact Estimate

- Current: 0 consolidation nodes visible to user-scoped queries → all retrieval is from raw import nodes
- With fix: 192 decision + 29 semantic nodes become retrievable → structured claims boost inference accuracy
- Expected AMB improvement: +5-8pp (especially for suggest_new_ideas, provide_preference_aligned_recommendations)

-- =============================================================================
-- Migration 012: Add children_tier_ids for Fractal Zooming
-- =============================================================================
--
-- Enables bidirectional tier navigation:
-- - parent_tier_id: L2 → L1 → L0 (upwards)
-- - children_tier_ids: L0 → [L1 nodes] → [L2 nodes] (downwards, fractal zooming)
--
-- This is the critical missing piece for hierarchical retrieval:
-- 1. Search on L0 (summaries) — fast, coarse
-- 2. For relevant L0 hits, zoom to L1 (overviews) via children_tier_ids
-- 3. For relevant L1 hits, zoom to L2 (raw) via children_tier_ids
--
-- The fractal effect: more data → better cluster overlap → faster retrieval

-- Add children_tier_ids column (UUID array)
ALTER TABLE memories 
ADD COLUMN IF NOT EXISTS children_tier_ids UUID[] DEFAULT ARRAY[]::UUID[];

-- Index for fast "find children of parent" queries
CREATE INDEX IF NOT EXISTS idx_memories_children_tier 
ON memories USING GIN (children_tier_ids) 
WHERE array_length(children_tier_ids, 1) > 0;

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('012_add_children_tier_ids')
ON CONFLICT (version) DO NOTHING;

-- =============================================================================
-- Migration 003: Tiered Context (L0/L1/L2)
-- 
-- Enables hierarchical context loading:
--   - L0 (summary): one-sentence summary
--   - L1 (overview): paragraph summary  
--   - L2 (raw): full original content
-- 
-- Memories default to 'raw' (L2) for backward compatibility.
-- Compaction generates L1 then L0 with parent_tier_id linking.
-- =============================================================================

-- New enum for context tier
CREATE TYPE context_tier AS ENUM ('summary', 'overview', 'raw');

-- Add tier columns to memories (all existing memories become 'raw' by default)
ALTER TABLE memories ADD COLUMN IF NOT EXISTS context_tier context_tier NOT NULL DEFAULT 'raw';
ALTER TABLE memories ADD COLUMN IF NOT EXISTS parent_tier_id UUID REFERENCES memories(id) ON DELETE SET NULL;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS summary_content TEXT;          -- L0: one-sentence
ALTER TABLE memories ADD COLUMN IF NOT EXISTS overview_content TEXT;          -- L1: paragraph

-- Indexes for tiered queries
CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(context_tier);
CREATE INDEX IF NOT EXISTS idx_memories_parent_tier ON memories(parent_tier_id) WHERE parent_tier_id IS NOT NULL;

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('003_add_tiered_context')
ON CONFLICT (version) DO NOTHING;

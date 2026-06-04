-- =============================================================================
-- Migration 019: Evidence Grounding — link memories to their source passages
-- =============================================================================

-- Add source_memory_id column to memories table
ALTER TABLE memories
    ADD COLUMN IF NOT EXISTS source_memory_id UUID REFERENCES memories(id);

-- Index for efficient source lookups
CREATE INDEX IF NOT EXISTS idx_memories_source_memory_id
    ON memories(source_memory_id);

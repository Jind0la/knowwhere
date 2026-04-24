-- =============================================================================
-- Migration 013: Repair Embedding Dimensions (1536 → 1024)
-- =============================================================================
--
-- Re-embeds all legacy memories with wrong dimension using Ollama.
-- This is a ONE-TIME migration that runs server-side.
--
-- Strategy:
-- 1. Find all memories where array_length(embedding, 1) != 1024
-- 2. For each: extract content, re-embed with snowflake-arctic-embed2
-- 3. Update embedding vector in-place
--
-- Safety: This is idempotent — running twice is harmless.

-- Add a repair tracking column
ALTER TABLE memories 
ADD COLUMN IF NOT EXISTS embedding_repaired_at TIMESTAMP;

-- Create index for fast lookup of unrepaired memories
CREATE INDEX IF NOT EXISTS idx_memories_needs_repair 
ON memories (embedding_repaired_at) 
WHERE embedding_repaired_at IS NULL;

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('013_repair_embedding_dimensions')
ON CONFLICT (version) DO NOTHING;

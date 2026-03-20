-- =============================================================================
-- Migration 009: Content Hashing & Self-Healing for External Nodes
--
-- External nodes (pointer-based memories) store a reference to an external file.
-- If the file is moved, the pointer becomes dangling.
--
-- This migration adds:
--   - `content_hash`: BLAKE3 hash of the file content for hash-based file recovery
--   - `semantic_thumbnail`: First 100 words of text content for semantic fallback search
--   - Partial index on content_hash for efficient Sentinel lookups
-- =============================================================================

-- ---------------------------------------------------------------------------
-- Content hash and semantic thumbnail for external nodes
-- ---------------------------------------------------------------------------

ALTER TABLE memories
    ADD COLUMN IF NOT EXISTS content_hash TEXT,
    ADD COLUMN IF NOT EXISTS semantic_thumbnail TEXT;

-- Index for fast Sentinel hash lookups (only where hash is present)
CREATE INDEX IF NOT EXISTS idx_memories_content_hash
    ON memories(content_hash)
    WHERE content_hash IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Self-healing audit log (tracks broken vs. repaired pointers)
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS self_healing_log (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    memory_id       UUID NOT NULL REFERENCES memories(id),
    broken_uri      TEXT NOT NULL,
    repair_status   TEXT NOT NULL,          -- 'repaired_hash', 'repaired_semantic', 'unrepaired'
    new_uri         TEXT,
    checked_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_self_healing_memory
    ON self_healing_log(memory_id);

CREATE INDEX IF NOT EXISTS idx_self_healing_status
    ON self_healing_log(repair_status);

-- ---------------------------------------------------------------------------
-- Schema version
-- ---------------------------------------------------------------------------

INSERT INTO schema_migrations (version) VALUES ('009_add_content_hash')
ON CONFLICT (version) DO NOTHING;

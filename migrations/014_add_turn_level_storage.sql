-- =============================================================================
-- Migration 014: Turn-Level Storage Schema
-- =============================================================================
--
-- Creates conversation_sessions and conversation_turns tables for per-turn
-- embedding pipeline. Each turn is a first-class entity with its own vector
-- embedding, enabling precise retrieval, session reconstruction, and
-- cross-session analysis.
--
-- Design doc: docs/turn-level-schema-design.md
--
-- NOTE: The session-level embedding column (`conversation_sessions.embedding`),
-- HNSW index (`idx_sessions_embedding_hnsw`), and helper function
-- (`compute_session_embedding`) added by this migration were **reversed**
-- by migration 015_drop_session_embedding. Session embeddings are no longer
-- computed, indexed, or used for retrieval. All retrieval now operates on
-- per-turn embeddings in `conversation_turns.embedding`.
-- =============================================================================

-- =============================================================================
-- TABLE: conversation_sessions
-- =============================================================================
CREATE TABLE IF NOT EXISTS conversation_sessions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    external_id     VARCHAR(255) UNIQUE,       -- Hermes/OpenClaw session ID
    title           TEXT,                       -- Auto-generated or user-provided
    participant_count INTEGER DEFAULT 2,        -- user + assistant
    turn_count      INTEGER NOT NULL DEFAULT 0, -- Denormalized counter
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at        TIMESTAMPTZ,
    metadata        JSONB DEFAULT '{}',         -- Platform, model, etc.
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_sessions_external ON conversation_sessions(external_id);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON conversation_sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_turn_count ON conversation_sessions(turn_count DESC);

-- =============================================================================
-- TABLE: conversation_turns
-- =============================================================================
CREATE TABLE IF NOT EXISTS conversation_turns (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id      UUID NOT NULL
                    REFERENCES conversation_sessions(id) ON DELETE CASCADE,
    turn_index      INTEGER NOT NULL,           -- 0-based, sequential within session
    speaker_role    VARCHAR(20) NOT NULL         -- 'user', 'assistant', 'system', 'tool'
                    CHECK (speaker_role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    content_preview VARCHAR(500) GENERATED ALWAYS AS (LEFT(content, 500)) STORED,
    embedding       vector(1024),                -- Matryoshka: truncate to 512/256/128
    token_count     INTEGER,
    metadata        JSONB DEFAULT '{}',          -- model, latency, tool_calls, etc.
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Every turn in a session must have a unique index
    CONSTRAINT unique_turn UNIQUE (session_id, turn_index)
);

-- Vector index (HNSW for fast k-NN)
CREATE INDEX IF NOT EXISTS idx_turns_embedding_hnsw ON conversation_turns
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- Turn ordering within a session
CREATE INDEX IF NOT EXISTS idx_turns_session_order ON conversation_turns(session_id, turn_index);

-- Filter by speaker
CREATE INDEX IF NOT EXISTS idx_turns_speaker ON conversation_turns(speaker_role);

-- Temporal queries
CREATE INDEX IF NOT EXISTS idx_turns_created ON conversation_turns(created_at DESC);

-- Full-text search on turn content
CREATE INDEX IF NOT EXISTS idx_turns_fts ON conversation_turns
    USING gin(to_tsvector('english', COALESCE(content, '')));

-- =============================================================================
-- COLUMN: session_embedding on conversation_sessions
-- =============================================================================
-- Session-level composite embedding: mean of all turn embeddings in the session.
-- Updated incrementally O(dim) on each new turn, not O(N*dim).
ALTER TABLE conversation_sessions ADD COLUMN IF NOT EXISTS embedding vector(1024);

-- HNSW index for session-level coarse retrieval
CREATE INDEX IF NOT EXISTS idx_sessions_embedding_hnsw ON conversation_sessions
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- =============================================================================
-- FK: memories.turn_id for backward-compatible linking
-- =============================================================================
ALTER TABLE memories ADD COLUMN IF NOT EXISTS turn_id UUID
    REFERENCES conversation_turns(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_memories_turn ON memories(turn_id) WHERE turn_id IS NOT NULL;

-- =============================================================================
-- Auto-update updated_at on sessions
-- =============================================================================
CREATE OR REPLACE FUNCTION update_sessions_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS update_sessions_updated_at ON conversation_sessions;
CREATE TRIGGER update_sessions_updated_at BEFORE UPDATE ON conversation_sessions
    FOR EACH ROW EXECUTE FUNCTION update_sessions_updated_at();

-- =============================================================================
-- HELPER: Compute session embedding from all turns (for backfill / repair)
-- =============================================================================
CREATE OR REPLACE FUNCTION compute_session_embedding(p_session_id UUID)
RETURNS vector(1024) AS $$
DECLARE
    avg_emb vector(1024);
BEGIN
    SELECT AVG(embedding) INTO avg_emb
    FROM conversation_turns
    WHERE session_id = p_session_id AND embedding IS NOT NULL;

    RETURN avg_emb;
END;
$$ LANGUAGE plpgsql;

-- =============================================================================
-- Schema version
-- =============================================================================
INSERT INTO schema_migrations (version) VALUES ('014_add_turn_level_storage')
ON CONFLICT (version) DO NOTHING;

-- =============================================================================
-- Migration 015: Drop Session-Level Embedding
-- =============================================================================
--
-- Per-turn embeddings are now the canonical vector representation.
-- The session-level aggregate embedding (AVG of turn embeddings) is no
-- longer needed — retrieval operates directly on conversation_turns.embedding.
--
-- Removes:
--   1. HNSW index on conversation_sessions.embedding
--   2. The embedding column from conversation_sessions
--   3. The compute_session_embedding() helper function
-- =============================================================================

-- Drop the HNSW index on session embeddings
DROP INDEX IF EXISTS idx_sessions_embedding_hnsw;

-- Drop the embedding column from conversation_sessions
ALTER TABLE conversation_sessions DROP COLUMN IF EXISTS embedding;

-- Drop the compute_session_embedding helper function
DROP FUNCTION IF EXISTS compute_session_embedding(UUID);

-- =============================================================================
-- Schema version
-- =============================================================================
INSERT INTO schema_migrations (version) VALUES ('015_drop_session_embedding')
ON CONFLICT (version) DO NOTHING;

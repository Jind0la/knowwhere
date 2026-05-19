-- =============================================================================
-- Migration 017: Add embedding metadata columns to conversation_turns
-- =============================================================================
--
-- Adds embedding_type (provider name) and embedding_dim (vector dimensionality)
-- to conversation_turns so the Turn data model can carry structured EmbeddingInfo
-- (provider, dimension, optional metadata) alongside the raw vector.
--
-- Existing rows are backfilled with sensible defaults:
--   - embedding_type → 'local_ollama' (the historical default provider)
--   - embedding_dim → inferred from array_length (1024 for the initial schema)
-- =============================================================================

-- Add provider type column
ALTER TABLE conversation_turns
    ADD COLUMN IF NOT EXISTS embedding_type VARCHAR(50);

-- Add dimension column
ALTER TABLE conversation_turns
    ADD COLUMN IF NOT EXISTS embedding_dim SMALLINT;

-- Backfill existing rows: infer dimension from the stored vector, default provider
-- to 'local_ollama' (the historical deployment default).
UPDATE conversation_turns
SET embedding_type = COALESCE(embedding_type, 'local_ollama'),
    embedding_dim  = COALESCE(embedding_dim, array_length(embedding::real[], 1)::SMALLINT)
WHERE embedding IS NOT NULL;

-- =============================================================================
-- Schema version
-- =============================================================================
INSERT INTO schema_migrations (version) VALUES ('017_add_embedding_info')
ON CONFLICT (version) DO NOTHING;

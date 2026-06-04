-- =============================================================================
-- Migration 018: Add fact_schemas table
-- =============================================================================
--
-- Creates the `fact_schemas` table for the entity graph / fact extraction
-- pipeline. Each row defines a valid (head_type, relation, tail_type) triple
-- pattern — the "schema" that governs what facts can be extracted and stored
-- as knowledge edges.
--
-- Columns:
--   id          — UUID primary key, auto-generated
--   schema_key  — unique human-readable key (e.g. "person_works_at_org")
--   head_type   — expected entity type for the subject (e.g. "person")
--   relation    — the predicate / relationship (e.g. "works_at")
--   tail_type   — expected entity type for the object (e.g. "organization")
--   frequency   — how often this schema pattern has been observed (default 1)
--   is_stable   — whether the schema has been validated / confirmed (default false)
--   created_at  — row creation timestamp
--   updated_at  — row last-modified timestamp
--
-- IDEMPOTENCY: Uses CREATE TABLE IF NOT EXISTS and CREATE INDEX IF NOT EXISTS.
--              The schema_migrations insert uses ON CONFLICT DO NOTHING.
--              Re-running this migration is safe.
-- =============================================================================

CREATE TABLE IF NOT EXISTS fact_schemas (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    schema_key  TEXT UNIQUE NOT NULL,
    head_type   TEXT NOT NULL,
    relation    TEXT NOT NULL,
    tail_type   TEXT NOT NULL,
    frequency   INTEGER NOT NULL DEFAULT 1,
    is_stable   BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_fact_schemas_schema_key ON fact_schemas(schema_key);
CREATE INDEX IF NOT EXISTS idx_fact_schemas_is_stable ON fact_schemas(is_stable);

-- =============================================================================
-- Schema version
-- =============================================================================
INSERT INTO schema_migrations (version) VALUES ('018_add_fact_schemas')
ON CONFLICT (version) DO NOTHING;

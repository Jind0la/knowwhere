-- =============================================================================
-- Migration 008: Energy Decay & Deduplication for Dream Mode
--
-- Part 1: Energy / Memory Decay (Ebbinghaus forgetting curve)
--   - Every memory has an `energy` level (0–100)
--   - Access boosts energy; time decays it
--   - Low-energy memories are candidates for compression in Dream Mode
--
-- Part 2: Deduplication Worker
--   - Finds memories with cosine similarity > 0.95
--   - Merges duplicates into a single consolidated memory
-- =============================================================================

-- ---------------------------------------------------------------------------
-- Energy decay fields on memories
-- ---------------------------------------------------------------------------

ALTER TABLE memories
    ADD COLUMN IF NOT EXISTS energy INT DEFAULT 50 CHECK (energy >= 0 AND energy <= 100),
    ADD COLUMN IF NOT EXISTS last_energy_update TIMESTAMPTZ DEFAULT NOW();

-- Index for efficient low-energy queries (used by Dream Mode compression)
CREATE INDEX IF NOT EXISTS idx_memories_low_energy
    ON memories(energy)
    WHERE energy < 20 AND status = 'active';

-- ---------------------------------------------------------------------------
-- Deduplication run log
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS deduplication_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pairs_found     INT NOT NULL DEFAULT 0,
    pairs_merged    INT NOT NULL DEFAULT 0,
    run_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dedup_runs_at
    ON deduplication_runs(run_at DESC);

-- ---------------------------------------------------------------------------
-- Schema version
-- ---------------------------------------------------------------------------

INSERT INTO schema_migrations (version) VALUES ('008_add_energy_decay')
ON CONFLICT (version) DO NOTHING;

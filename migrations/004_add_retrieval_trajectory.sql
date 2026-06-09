-- =============================================================================
-- Migration 004: Retrieval Trajectory Logging
--
-- Tracks every retrieval operation: what was searched, how many candidates
-- were found, which filters were applied, and why decisions were made.
-- =============================================================================

-- Table: individual retrieval runs (one per query)
CREATE TABLE IF NOT EXISTS retrieval_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Query info
    query_text          TEXT NOT NULL,
    embedding            vector(768),

    -- Timing & stats
    run_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    total_candidates    INT,
    retrieved_count      INT,
    execution_time_ms    INT,
    max_depth_used       INT,

    -- Extra context
    metadata            JSONB DEFAULT '{}'
);

-- Table: individual steps within a retrieval run
CREATE TABLE IF NOT EXISTS retrieval_trajectory (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id              UUID NOT NULL REFERENCES retrieval_runs(id) ON DELETE CASCADE,

    -- Step identification
    step_index          INT NOT NULL,
    step_type           VARCHAR(30) NOT NULL,  -- 'initial_search', 'fractal_zoom', 'rerank', 'governance_filter', 'bm25_search'

    -- Memory involved (NULL for aggregate steps)
    memory_id           UUID REFERENCES memories(id) ON DELETE SET NULL,

    -- Scoring
    score_before        FLOAT,
    score_after         FLOAT,
    rank                INT,

    -- Decision reasoning
    decision            TEXT,                    -- human-readable decision explanation
    filter_reason       TEXT,                    -- why something was filtered out

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_trajectory_run ON retrieval_trajectory(run_id);
CREATE INDEX IF NOT EXISTS idx_trajectory_step ON retrieval_trajectory(run_id, step_index);
CREATE INDEX IF NOT EXISTS idx_runs_at ON retrieval_runs(run_at DESC);
CREATE INDEX IF NOT EXISTS idx_runs_query_text ON retrieval_runs USING gin(to_tsvector('english', query_text));

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('004_add_retrieval_trajectory')
ON CONFLICT (version) DO NOTHING;

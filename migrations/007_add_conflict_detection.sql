-- =============================================================================
-- Migration 007: Conflict Detection for Dream Mode
-- 
-- Enables automatic detection and resolution of conflicting memories:
--   - Entity conflicts: same entity, different facts
--   - Temporal conflicts: same fact at different times
--   - Confidence conflicts: same fact, different confidence scores
--
-- Workflow:
--   1. Dream Mode runs conflict detection periodically
--   2. Conflicts are marked with conflict_state = 'pending'
--   3. Operator/LLM reviews via GET /conflicts
--   4. Resolution via POST /conflicts/{id}/resolve
--   5. Losing memories are marked superseded_by the winner
-- =============================================================================

-- Memory conflicts table: stores detected conflict groups
CREATE TABLE IF NOT EXISTS memory_conflicts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    
    -- The memories involved in this conflict
    conflicting_memory_ids UUID[] NOT NULL,
    
    -- Type of conflict
    conflict_type VARCHAR(20) NOT NULL CHECK (
        conflict_type IN ('entity', 'temporal', 'confidence')
    ),
    
    -- Human-readable description
    description TEXT NOT NULL,
    
    -- Timestamps
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    
    -- State: pending, resolved
    state VARCHAR(20) NOT NULL DEFAULT 'pending' CHECK (
        state IN ('pending', 'resolved')
    )
);

-- Conflict detection run history
CREATE TABLE IF NOT EXISTS conflict_detection_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conflicts_found INT NOT NULL DEFAULT 0,
    conflicts_resolved INT NOT NULL DEFAULT 0,
    run_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for conflict queries
CREATE INDEX IF NOT EXISTS idx_conflicts_state ON memory_conflicts(state);
CREATE INDEX IF NOT EXISTS idx_conflicts_type ON memory_conflicts(conflict_type);
CREATE INDEX IF NOT EXISTS idx_conflicts_detected ON memory_conflicts(detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_conflict_runs_at ON conflict_detection_runs(run_at DESC);

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('007_add_conflict_detection')
ON CONFLICT (version) DO NOTHING;

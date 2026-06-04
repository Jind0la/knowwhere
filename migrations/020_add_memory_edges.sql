-- Migration 020: Add memory_edges table for type-based bridging
-- Connects disconnected graph islands by creating weak edges between
-- entities that share the same memory_type during consolidation.
CREATE TABLE IF NOT EXISTS memory_edges (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id   UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    target_id   UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    edge_type   VARCHAR(50) NOT NULL DEFAULT 'RELATES_TO',
    weight      DOUBLE PRECISION NOT NULL DEFAULT 0.3,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_memory_edge UNIQUE (source_id, target_id, edge_type)
);

CREATE INDEX IF NOT EXISTS idx_memory_edges_source ON memory_edges(source_id);
CREATE INDEX IF NOT EXISTS idx_memory_edges_target ON memory_edges(target_id);
CREATE INDEX IF NOT EXISTS idx_memory_edges_type ON memory_edges(edge_type);

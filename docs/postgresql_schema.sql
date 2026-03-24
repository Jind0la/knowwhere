-- =============================================================================
-- KnowWhere v0.3 — PostgreSQL Schema
-- Phase A: PostgreSQL + pgvector Storage Layer
-- =============================================================================
-- This schema replaces the JSON-file persistence with a proper relational store.
-- It adds:
--   - 5 Memory Types (episodic, semantic, preference, procedural, meta)
--   - Knowledge Graph Edges
--   - Immutable Event Log (Layer 0)
--   - Full Governance Fields (confidence, sensitivity, supersession)
-- =============================================================================

-- Enable extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "vector";

-- =============================================================================
-- LAYER 0: IMMUTABLE EVENT LOG
-- Append-only. No UPDATE, no DELETE. This is the source of truth.
-- =============================================================================

CREATE TABLE IF NOT EXISTS events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type  VARCHAR(50) NOT NULL,
    payload     JSONB NOT NULL,   -- immutable once written
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for event sourcing replay
CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);
CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at DESC);

-- Rule: prevent updates and deletes (make it truly immutable)
CREATE OR REPLACE FUNCTION block_event_modification()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'Events table is immutable. UPDATE and DELETE are forbidden.';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS block_event_update ON events;
CREATE TRIGGER block_event_update BEFORE UPDATE ON events
    FOR EACH ROW EXECUTE FUNCTION block_event_modification();

DROP TRIGGER IF EXISTS block_event_delete ON events;
CREATE TRIGGER block_event_delete BEFORE DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION block_event_modification();

-- =============================================================================
-- MEMORIES TABLE (Core Entity — replaces JSON persistence)
-- =============================================================================

CREATE TABLE IF NOT EXISTS memories (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Memory Type (canonical 5-type system)
    memory_type     VARCHAR(20) NOT NULL CHECK (
        memory_type IN ('episodic', 'semantic', 'preference', 'procedural', 'meta')
    ),

    -- Content
    content         TEXT NOT NULL,
    content_preview VARCHAR(500),
    original_pointer TEXT,  -- For external nodes: reference to an external file (URI/path)

    -- Embedding (vector stored separately in usearch, but we keep pgvector too)
    embedding       vector(768),  -- 768-dim for nomic-embed-text-v2-moe

    -- Classification
    entities        JSONB DEFAULT '[]',   -- extracted entity names
    tags            TEXT[] DEFAULT ARRAY[]::TEXT[],

    -- Governance
    importance      INTEGER DEFAULT 5 CHECK (importance >= 1 AND importance <= 10),
    confidence      FLOAT DEFAULT 0.8 CHECK (confidence >= 0.0 AND confidence <= 1.0),
    sensitivity     VARCHAR(20) DEFAULT 'normal' CHECK (
        sensitivity IN ('normal', 'low', 'high', 'restricted')
    ),
    status          VARCHAR(20) DEFAULT 'active' CHECK (
        status IN ('active', 'draft', 'archived', 'deleted', 'superseded', 'stale')
    ),
    superseded_by   UUID REFERENCES memories(id) ON DELETE SET NULL,
    conflict_state  VARCHAR(20) DEFAULT 'none' CHECK (
        conflict_state IN ('none', 'pending', 'resolved')
    ),

    -- Source Tracking (provenance)
    source          VARCHAR(20) DEFAULT 'conversation' CHECK (
        source IN ('conversation', 'document', 'import', 'manual', 'consolidation')
    ),
    source_id       VARCHAR(255),          -- conversation_id, file_id, etc.
    provenance      JSONB DEFAULT '{}',     -- original_file, import_timestamp, etc.

    -- Fractal Structure (parent-child for zoom retrieval)
    parent_id       UUID REFERENCES memories(id) ON DELETE SET NULL,
    depth           INTEGER DEFAULT 0,      -- fractal zoom depth

    -- Access Tracking
    access_count    INT DEFAULT 0,
    last_accessed   TIMESTAMPTZ DEFAULT NOW(),

    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ,

    -- Metadata
    metadata        JSONB DEFAULT '{}'
);

-- HNSW Index for vector similarity
CREATE INDEX IF NOT EXISTS idx_memories_embedding_hnsw ON memories
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- Performance indexes
CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance DESC) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memories_last_accessed ON memories(last_accessed DESC) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_memories_parent ON memories(parent_id);
CREATE INDEX IF NOT EXISTS idx_memories_superseded ON memories(superseded_by) WHERE superseded_by IS NOT NULL;

-- Entity + tag search
CREATE INDEX IF NOT EXISTS idx_memories_entities ON memories USING GIN(entities);
CREATE INDEX IF NOT EXISTS idx_memories_tags ON memories USING GIN(tags);

-- Auto-update updated_at
CREATE OR REPLACE FUNCTION update_memories_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS update_memories_updated_at ON memories;
CREATE TRIGGER update_memories_updated_at BEFORE UPDATE ON memories
    FOR EACH ROW EXECUTE FUNCTION update_memories_updated_at();

-- Auto-generate content_preview
CREATE OR REPLACE FUNCTION generate_content_preview()
RETURNS TRIGGER AS $$
BEGIN
    NEW.content_preview = LEFT(NEW.content, 500);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS generate_memory_preview ON memories;
CREATE TRIGGER generate_memory_preview BEFORE INSERT OR UPDATE OF content ON memories
    FOR EACH ROW EXECUTE FUNCTION generate_content_preview();

-- =============================================================================
-- KNOWLEDGE EDGES TABLE (Graph Relationships)
-- =============================================================================

CREATE TABLE IF NOT EXISTS knowledge_edges (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Edge Endpoints
    from_node_id    UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    to_node_id      UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,

    -- Edge Characteristics
    edge_type       VARCHAR(50) NOT NULL CHECK (
        edge_type IN (
            'leads_to', 'related_to', 'contradicts', 'supports',
            'likes', 'dislikes', 'depends_on', 'evolves_into'
        )
    ),
    strength        FLOAT DEFAULT 0.7 CHECK (strength >= 0.0 AND strength <= 1.0),
    confidence      FLOAT DEFAULT 0.8 CHECK (confidence >= 0.0 AND confidence <= 1.0),

    -- Semantics
    causality       BOOLEAN DEFAULT FALSE,
    bidirectional   BOOLEAN DEFAULT FALSE,
    reason          TEXT,

    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Metadata
    metadata        JSONB DEFAULT '{}',

    CONSTRAINT no_self_reference CHECK (from_node_id != to_node_id)
);

-- Unique constraint: no duplicate edges of same type between same nodes
CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique
    ON knowledge_edges(from_node_id, to_node_id, edge_type);

CREATE INDEX IF NOT EXISTS idx_edges_from ON knowledge_edges(from_node_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON knowledge_edges(to_node_id);
CREATE INDEX IF NOT EXISTS idx_edges_type ON knowledge_edges(edge_type);
CREATE INDEX IF NOT EXISTS idx_edges_strength ON knowledge_edges(strength DESC) WHERE strength > 0.7;

-- =============================================================================
-- DREAM MODE: CONSOLIDATION HISTORY
-- =============================================================================

CREATE TABLE IF NOT EXISTS consolidation_history (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    consolidation_date       DATE NOT NULL DEFAULT CURRENT_DATE,

    -- Session Info
    session_id              VARCHAR(255),
    conversation_id         VARCHAR(255),

    -- Processing Stats
    session_transcript_len  INT DEFAULT 0,
    claims_extracted        INT DEFAULT 0,
    memories_processed      INT DEFAULT 0,
    new_memories_created    INT DEFAULT 0,
    merged_count            INT DEFAULT 0,
    conflicts_resolved      INT DEFAULT 0,
    edges_created           INT DEFAULT 0,

    -- Performance
    processing_time_ms      INT DEFAULT 0,

    -- Status
    status                  VARCHAR(20) DEFAULT 'pending' CHECK (
        status IN ('pending', 'running', 'completed', 'failed')
    ),
    error_message          TEXT,

    -- Timestamps
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_consolidation_date ON consolidation_history(consolidation_date DESC);
CREATE INDEX IF NOT EXISTS idx_consolidation_status ON consolidation_history(status) WHERE status != 'completed';

-- =============================================================================
-- DREAM MODE: AUDIT LOG
-- =============================================================================

CREATE TABLE IF NOT EXISTS audit_log (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL,
    issue_type      VARCHAR(50) NOT NULL CHECK (
        issue_type IN ('drift_detected', 'conflict_found', 'sensitivity_violation',
                       'stale_marked', 'supersession_chain', 'low_confidence')
    ),
    memory_id       UUID REFERENCES memories(id) ON DELETE CASCADE,
    severity        VARCHAR(10) DEFAULT 'info' CHECK (severity IN ('info', 'warning', 'critical')),
    description     TEXT,
    action_taken    VARCHAR(100),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_run ON audit_log(run_id);
CREATE INDEX IF NOT EXISTS idx_audit_memory ON audit_log(memory_id);
CREATE INDEX IF NOT EXISTS idx_audit_type ON audit_log(issue_type);

-- =============================================================================
-- API KEYS (for auth — lightweight, no full user system yet)
-- =============================================================================

CREATE TABLE IF NOT EXISTS api_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_prefix      VARCHAR(20) NOT NULL,
    key_hash        VARCHAR(255) NOT NULL UNIQUE,
    name            VARCHAR(255),
    scopes          TEXT[] DEFAULT ARRAY['read', 'write']::TEXT[],
    rate_limit      INT DEFAULT 1000,   -- requests per minute
    last_used_at    TIMESTAMPTZ,
    status          VARCHAR(20) DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_api_keys_status ON api_keys(status) WHERE status = 'active';

-- =============================================================================
-- HELPER FUNCTIONS
-- =============================================================================

-- Search memories by vector similarity (wrapper around pgvector)
CREATE OR REPLACE FUNCTION search_memories_vector(
    p_embedding     vector(768),
    p_limit         INT DEFAULT 10,
    p_memory_type   VARCHAR DEFAULT NULL,
    p_min_importance INT DEFAULT NULL
)
RETURNS TABLE (
    id              UUID,
    content         TEXT,
    memory_type     VARCHAR,
    importance      INT,
    similarity      FLOAT,
    created_at      TIMESTAMPTZ
) AS $$
BEGIN
    RETURN QUERY
    SELECT
        m.id,
        m.content,
        m.memory_type,
        m.importance,
        (1 - (m.embedding <=> p_embedding))::FLOAT AS similarity,
        m.created_at
    FROM memories m
    WHERE m.status = 'active'
        AND m.embedding IS NOT NULL
        AND (p_memory_type IS NULL OR m.memory_type = p_memory_type)
        AND (p_min_importance IS NULL OR m.importance >= p_min_importance)
    ORDER BY m.embedding <=> p_embedding
    LIMIT p_limit;
END;
$$ LANGUAGE plpgsql;

-- Get fractal children (for zoom retrieval)
CREATE OR REPLACE FUNCTION get_fractal_children(
    p_memory_id     UUID,
    p_max_depth     INT DEFAULT 3
)
RETURNS TABLE (
    id              UUID,
    content         TEXT,
    memory_type     VARCHAR,
    depth           INT,
    path            UUID[]
) AS $$
BEGIN
    RETURN QUERY
    WITH RECURSIVE fractal_tree AS (
        -- Base case
        SELECT m.id, m.content, m.memory_type, m.depth,
               ARRAY[m.id]::UUID[] AS path, 1 AS level
        FROM memories m
        WHERE m.parent_id = p_memory_id AND m.status = 'active'

        UNION ALL

        -- Recursive case
        SELECT m.id, m.content, m.memory_type, m.depth,
               ft.path || m.id,
               ft.level + 1
        FROM memories m
        INNER JOIN fractal_tree ft ON m.parent_id = ft.id
        WHERE m.status = 'active' AND ft.level < p_max_depth
    )
    SELECT ft.id, ft.content, ft.memory_type, ft.level AS depth, ft.path
    FROM fractal_tree ft
    ORDER BY ft.level;
END;
$$ LANGUAGE plpgsql;

-- Get related memories via knowledge graph
CREATE OR REPLACE FUNCTION get_related_memories(
    p_memory_id     UUID,
    p_depth         INT DEFAULT 1
)
RETURNS TABLE (
    memory_id       UUID,
    content         TEXT,
    memory_type     VARCHAR,
    edge_type       VARCHAR,
    strength        FLOAT,
    depth           INT
) AS $$
BEGIN
    RETURN QUERY
    WITH RECURSIVE graph_traverse AS (
        SELECT
            CASE WHEN e.from_node_id = p_memory_id THEN e.to_node_id ELSE e.from_node_id END AS memory_id,
            e.edge_type,
            e.strength,
            1 AS depth
        FROM knowledge_edges e
        WHERE e.from_node_id = p_memory_id OR e.to_node_id = p_memory_id

        UNION ALL

        SELECT
            CASE WHEN e.from_node_id = gt.memory_id THEN e.to_node_id ELSE e.from_node_id END,
            e.edge_type,
            e.strength,
            gt.depth + 1
        FROM knowledge_edges e
        INNER JOIN graph_traverse gt ON e.from_node_id = gt.memory_id OR e.to_node_id = gt.memory_id
        WHERE gt.depth < p_depth
    )
    SELECT gt.memory_id, m.content, m.memory_type, gt.edge_type, gt.strength, gt.depth
    FROM graph_traverse gt
    INNER JOIN memories m ON m.id = gt.memory_id
    WHERE m.status = 'active';
END;
$$ LANGUAGE plpgsql;

-- =============================================================================
-- SCHEMA VERSION
-- =============================================================================

CREATE TABLE IF NOT EXISTS schema_migrations (
    version     VARCHAR(50) PRIMARY KEY,
    applied_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO schema_migrations (version) VALUES ('002_postgresql_storage')
ON CONFLICT (version) DO NOTHING;

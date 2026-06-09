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

-- === 003_add_tiered_context.sql ===
-- =============================================================================
-- Migration 003: Tiered Context (L0/L1/L2)
--
-- Enables hierarchical context loading:
--   - L0 (summary): one-sentence summary
--   - L1 (overview): paragraph summary
--   - L2 (raw): full original content
--
-- Memories default to 'raw' (L2) for backward compatibility.
-- Compaction generates L1 then L0 with parent_tier_id linking.
-- =============================================================================

-- New enum for context tier
CREATE TYPE context_tier AS ENUM ('summary', 'overview', 'raw');

-- Add tier columns to memories (all existing memories become 'raw' by default)
ALTER TABLE memories ADD COLUMN IF NOT EXISTS context_tier context_tier NOT NULL DEFAULT 'raw';
ALTER TABLE memories ADD COLUMN IF NOT EXISTS parent_tier_id UUID REFERENCES memories(id) ON DELETE SET NULL;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS summary_content TEXT;          -- L0: one-sentence
ALTER TABLE memories ADD COLUMN IF NOT EXISTS overview_content TEXT;          -- L1: paragraph

-- Indexes for tiered queries
CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(context_tier);
CREATE INDEX IF NOT EXISTS idx_memories_parent_tier ON memories(parent_tier_id) WHERE parent_tier_id IS NOT NULL;

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('003_add_tiered_context')
ON CONFLICT (version) DO NOTHING;

-- === 004_add_retrieval_trajectory.sql ===
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

-- === 007_add_conflict_detection.sql ===
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

-- === 008_add_energy_decay.sql ===
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

-- === 009_add_content_hash.sql ===
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

-- === 010_add_auth_users.sql ===
-- =============================================================================
-- Migration 010: User Authentication Tables
--
-- Replaces the in-memory HashMap auth with PostgreSQL-backed user + API key storage.
-- API keys are bcrypt hashes of the actual key (kw_xxx format, like Stripe).
-- =============================================================================

-- Users table (beta user accounts)
CREATE TABLE IF NOT EXISTS auth_users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username        VARCHAR(255) UNIQUE NOT NULL,
    email           VARCHAR(255) UNIQUE NOT NULL,
    password_hash   VARCHAR(255) NOT NULL,  -- bcrypt hash of password
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_auth_users_username ON auth_users(username);
CREATE INDEX IF NOT EXISTS idx_auth_users_email ON auth_users(email);

-- API keys table (BLAKE3 hex fingerprint of kw_… secret; legacy rows may still hold bcrypt until first use)
CREATE TABLE IF NOT EXISTS auth_api_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    key_hash        VARCHAR(255) NOT NULL,  -- hex(BLAKE3(plaintext key)); legacy: bcrypt string starting with $2
    name            VARCHAR(255) DEFAULT 'default',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_auth_api_keys_key_hash ON auth_api_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_auth_api_keys_user ON auth_api_keys(user_id);

-- Schema version
INSERT INTO schema_migrations (version) VALUES ('010_add_auth_users')
ON CONFLICT (version) DO NOTHING;

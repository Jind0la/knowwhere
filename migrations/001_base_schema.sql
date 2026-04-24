-- Base schema for KnowWhere (reconstructed from code)
-- This creates all core tables needed before feature-specific migrations

-- Core memories table
CREATE TABLE IF NOT EXISTS memories (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    memory_type     VARCHAR(50) NOT NULL,
    content         TEXT,
    embedding       vector(768),
    entities        JSONB,
    tags            TEXT[],
    provenance      JSONB,
    source          VARCHAR(100),
    source_id       VARCHAR(255),
    importance      INTEGER DEFAULT 0,
    confidence      DOUBLE PRECISION,
    sensitivity     VARCHAR(20) DEFAULT 'normal',
    status          VARCHAR(20) DEFAULT 'active',
    access_count    INTEGER DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_accessed   TIMESTAMPTZ,
    superseded_by   UUID,
    conflict_state  VARCHAR(20) DEFAULT 'none',
    weight          DOUBLE PRECISION DEFAULT 1.0,
    parent_tier_id  UUID,
    summary_content TEXT,
    overview_content TEXT,
    tier            INTEGER DEFAULT 0,
    children_tier_ids UUID[],
    content_hash    VARCHAR(64),
    metadata        JSONB
);

-- Indexes for memories
CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status);
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
CREATE INDEX IF NOT EXISTS idx_memories_conflict ON memories(conflict_state);
CREATE INDEX IF NOT EXISTS idx_memories_parent ON memories(parent_tier_id);

-- Full-text search index
CREATE INDEX IF NOT EXISTS idx_memories_fts ON memories USING gin(to_tsvector('english', COALESCE(content, '')));

-- Events table (event sourcing)
CREATE TABLE IF NOT EXISTS events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type  VARCHAR(100) NOT NULL,
    payload     JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);
CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at);

-- Knowledge edges (relationships between memories)
CREATE TABLE IF NOT EXISTS knowledge_edges (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    from_node_id    UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    to_node_id      UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    edge_type       VARCHAR(50) NOT NULL,
    strength        DOUBLE PRECISION DEFAULT 1.0,
    metadata        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_edges_from ON knowledge_edges(from_node_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON knowledge_edges(to_node_id);
CREATE INDEX IF NOT EXISTS idx_edges_type ON knowledge_edges(edge_type);

-- Consolidation history
CREATE TABLE IF NOT EXISTS consolidation_history (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    consolidation_date  DATE NOT NULL DEFAULT CURRENT_DATE,
    session_id          VARCHAR(255),
    conversation_id     VARCHAR(255),
    memories_processed  INTEGER NOT NULL DEFAULT 0,
    new_memories_created INTEGER NOT NULL DEFAULT 0,
    edges_created       INTEGER NOT NULL DEFAULT 0,
    processing_time_ms  INTEGER NOT NULL DEFAULT 0,
    status              VARCHAR(50) NOT NULL DEFAULT 'success',
    error_message       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_consolidation_date ON consolidation_history(consolidation_date);

-- Audit log
CREATE TABLE IF NOT EXISTS audit_log (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    action      VARCHAR(100) NOT NULL,
    details     JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_log(created_at);

-- Memory namespaces (directory structure)
CREATE TABLE IF NOT EXISTS memory_namespaces (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    path        VARCHAR(500) UNIQUE NOT NULL,
    depth       INTEGER NOT NULL DEFAULT 0,
    parent_id   UUID REFERENCES memory_namespaces(id) ON DELETE CASCADE,
    description TEXT,
    memory_type_hint VARCHAR(50),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_namespaces_path ON memory_namespaces(path);
CREATE INDEX IF NOT EXISTS idx_namespaces_parent ON memory_namespaces(parent_id);

-- Agent skills
CREATE TABLE IF NOT EXISTS agent_skills (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_name      VARCHAR(255) NOT NULL,
    category        VARCHAR(100) NOT NULL,
    proficiency     INTEGER NOT NULL DEFAULT 0,
    components      JSONB,
    prerequisites   JSONB,
    namespace_id    UUID REFERENCES memory_namespaces(id) ON DELETE SET NULL,
    metadata        JSONB,
    last_used       TIMESTAMPTZ,
    success_rate    DOUBLE PRECISION DEFAULT 0.0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_skills_name ON agent_skills(skill_name);
CREATE INDEX IF NOT EXISTS idx_skills_category ON agent_skills(category);

-- Skill memories (link skills to memories)
CREATE TABLE IF NOT EXISTS skill_memories (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_id    UUID NOT NULL REFERENCES agent_skills(id) ON DELETE CASCADE,
    memory_id   UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(skill_id, memory_id)
);

-- Conflict detection
CREATE TABLE IF NOT EXISTS memory_conflicts (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conflicting_memory_ids    UUID[] NOT NULL,
    conflict_type           VARCHAR(50) NOT NULL,
    description             TEXT,
    detected_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at             TIMESTAMPTZ,
    state                   VARCHAR(20) DEFAULT 'pending',
    resolution_strategy     VARCHAR(50)
);

CREATE INDEX IF NOT EXISTS idx_conflicts_state ON memory_conflicts(state);
CREATE INDEX IF NOT EXISTS idx_conflicts_detected ON memory_conflicts(detected_at);

CREATE TABLE IF NOT EXISTS conflict_detection_runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    conflicts_found INTEGER NOT NULL DEFAULT 0,
    resolved_count INTEGER NOT NULL DEFAULT 0,
    execution_time_ms INTEGER NOT NULL DEFAULT 0
);

-- Energy decay / deduplication
CREATE TABLE IF NOT EXISTS deduplication_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    pairs_found         INTEGER NOT NULL DEFAULT 0,
    pairs_merged        INTEGER NOT NULL DEFAULT 0,
    execution_time_ms   INTEGER NOT NULL DEFAULT 0
);

-- Self-healing log
CREATE TABLE IF NOT EXISTS self_healing_log (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    memory_id       UUID REFERENCES memories(id) ON DELETE CASCADE,
    issue_type      VARCHAR(100) NOT NULL,
    description     TEXT,
    fixed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    auto_fixed      BOOLEAN DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_healing_memory ON self_healing_log(memory_id);

-- Retrieval trajectory
CREATE TABLE IF NOT EXISTS retrieval_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query_text          TEXT,
    embedding           vector(768),
    run_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    total_candidates    INTEGER NOT NULL DEFAULT 0,
    retrieved_count     INTEGER NOT NULL DEFAULT 0,
    execution_time_ms   INTEGER NOT NULL DEFAULT 0,
    max_depth_used      INTEGER NOT NULL DEFAULT 0,
    metadata            JSONB
);

CREATE INDEX IF NOT EXISTS idx_retrieval_runs_at ON retrieval_runs(run_at);

CREATE TABLE IF NOT EXISTS retrieval_trajectory (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES retrieval_runs(id) ON DELETE CASCADE,
    step_index      INTEGER NOT NULL,
    step_type       VARCHAR(50) NOT NULL,
    memory_id       UUID REFERENCES memories(id) ON DELETE SET NULL,
    score_before    DOUBLE PRECISION,
    score_after     DOUBLE PRECISION,
    rank            INTEGER,
    decision        VARCHAR(50),
    filter_reason   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trajectory_run ON retrieval_trajectory(run_id);

-- Auth tables (created by run_auth_migrations but also here for completeness)
CREATE TABLE IF NOT EXISTS auth_users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username        VARCHAR(255) UNIQUE NOT NULL,
    email           VARCHAR(255) UNIQUE NOT NULL,
    password_hash   VARCHAR(255) NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_auth_users_username ON auth_users(username);
CREATE INDEX IF NOT EXISTS idx_auth_users_email ON auth_users(email);

CREATE TABLE IF NOT EXISTS auth_api_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    key_hash        VARCHAR(255) NOT NULL,
    name            VARCHAR(255) DEFAULT 'default',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ,
    revoked_at      TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_auth_api_keys_key_hash ON auth_api_keys(key_hash);

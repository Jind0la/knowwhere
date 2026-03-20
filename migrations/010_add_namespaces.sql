-- =============================================================================
-- Migration 010: Directory Namespace
--
-- Hierarchical address space for memories, similar to viking:// URIs.
-- Namespaces allow organizing memories by kind (user, agent, resources, etc.)
-- and enable namespace-scoped search.
-- =============================================================================

-- ---------------------------------------------------------------------------
-- Memory Namespaces Table
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS memory_namespaces (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    path                TEXT NOT NULL UNIQUE,                     -- 'user/preferences', 'agent/skills'
    depth               INT NOT NULL,
    parent_id           UUID REFERENCES memory_namespaces(id),
    description         TEXT,
    memory_type_hint    VARCHAR(20),                             -- optional hint for the type of memories in this namespace
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Link Memories to Namespaces
-- ---------------------------------------------------------------------------

ALTER TABLE memories
    ADD COLUMN IF NOT EXISTS namespace_id UUID REFERENCES memory_namespaces(id);

-- ---------------------------------------------------------------------------
-- Indexes
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS idx_namespaces_path ON memory_namespaces(path);
CREATE INDEX IF NOT EXISTS idx_namespaces_parent ON memory_namespaces(parent_id);
CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace_id) WHERE namespace_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Pre-seeded Namespaces
-- ---------------------------------------------------------------------------

INSERT INTO memory_namespaces (path, depth, description) VALUES
    ('user/preferences',    2, 'User preferences and settings'),
    ('user/profile',        2, 'User profile information'),
    ('agent/skills',        2, 'Agent capabilities and skills'),
    ('agent/experience',    2, 'Learned experiences and patterns'),
    ('agent/procedures',    2, 'Agent workflows and procedures'),
    ('resources/docs',     2, 'External document references'),
    ('resources/cameras',   2, 'Camera/IoT device references'),
    ('session/history',     2, 'Session transcripts and events'),
    ('memory/meta',         2, 'Meta-information about memory system')
ON CONFLICT (path) DO NOTHING;

-- ---------------------------------------------------------------------------
-- Schema version
-- ---------------------------------------------------------------------------

INSERT INTO schema_migrations (version) VALUES ('010_add_namespaces')
ON CONFLICT (version) DO NOTHING;

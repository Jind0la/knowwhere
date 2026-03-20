-- =============================================================================
-- Migration 011: Agent Skills Management
--
-- Explicit skill registry for the agent — what it can do, how well,
-- when it was last used, and which memories document that skill.
-- Skills are stored in the agent/skills namespace by default.
-- =============================================================================

-- ---------------------------------------------------------------------------
-- Agent Skills Table
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS agent_skills (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_name          TEXT NOT NULL,
    category            VARCHAR(50) NOT NULL,                           -- 'language', 'tool', 'domain', 'framework'
    proficiency         INT DEFAULT 5 CHECK (proficiency >= 1 AND proficiency <= 10),
    last_used           TIMESTAMPTZ,
    success_rate        FLOAT DEFAULT 0.0 CHECK (success_rate >= 0.0 AND success_rate <= 1.0),
    components          TEXT[] DEFAULT ARRAY[]::TEXT[],
    prerequisites       TEXT[] DEFAULT ARRAY[]::TEXT[],
    namespace_id        UUID REFERENCES memory_namespaces(id),
    metadata            JSONB DEFAULT '{}',
    created_at          TIMESTAMPTZ DEFAULT NOW(),
    updated_at          TIMESTAMPTZ DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Skill ↔ Memory Association
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS skill_memories (
    skill_id        UUID REFERENCES agent_skills(id) ON DELETE CASCADE,
    memory_id       UUID REFERENCES memories(id) ON DELETE CASCADE,
    relation_type    VARCHAR(30) DEFAULT 'referenced_in',   -- 'referenced_in', 'example_of', 'practices'
    created_at      TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (skill_id, memory_id)
);

-- ---------------------------------------------------------------------------
-- Indexes
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS idx_skills_category ON agent_skills(category);
CREATE INDEX IF NOT EXISTS idx_skills_proficiency ON agent_skills(proficiency DESC);
CREATE INDEX IF NOT EXISTS idx_skills_last_used ON agent_skills(last_used DESC);
CREATE INDEX IF NOT EXISTS idx_skill_memories_skill ON skill_memories(skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_memories_memory ON skill_memories(memory_id);

-- ---------------------------------------------------------------------------
-- Schema version
-- ---------------------------------------------------------------------------

INSERT INTO schema_migrations (version) VALUES ('011_add_skills')
ON CONFLICT (version) DO NOTHING;

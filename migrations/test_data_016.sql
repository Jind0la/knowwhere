-- =============================================================================
-- Test Dataset for Migration 016: Session-to-Turn Backfill
-- =============================================================================
--
-- Seeds the `memories` table with realistic session data patterns to test
-- the backfill migration. Designed to be run against a clean DB that has
-- migrations 001-015 already applied.
--
-- Usage:
--   psql $DATABASE_URL -f migrations/test_data_016.sql
--
-- Then:
--   psql $DATABASE_URL -f migrations/016_backfill_session_to_turn.sql
--   python3 scripts/test_migration_016.py
-- =============================================================================

BEGIN;

-- Ensure vector extension exists (from 001_base_schema)
CREATE EXTENSION IF NOT EXISTS vector;

-- =============================================================================
-- Pattern A: Multi-chunk session (3 chunks + raw node)
-- Simulates a 3-turn conversation between user and assistant.
-- =============================================================================

-- Raw node (full session content) — should be SKIPPED by migration
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'a0000000-0000-0000-0000-000000000001'::UUID,
    'episodic',
    'user: Hello there
assistant: Hi! How can I help you today?
user: I need to know about fractal memory
assistant: Fractal memory is a hierarchical storage system...',
    NULL,  -- vector(1024) — test uses NULL to avoid dimension mismatch
    '{
        "session_id": "test-session-a",
        "is_full_content": true,
        "chunk_ids": [
            "a0000000-0000-0000-0000-000000000002",
            "a0000000-0000-0000-0000-000000000003",
            "a0000000-0000-0000-0000-000000000004"
        ]
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '2 days'
);

-- Chunk 0: user turn
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'a0000000-0000-0000-0000-000000000002'::UUID,
    'episodic',
    'user: Hello there',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-a",
        "is_chunk": true,
        "chunk_index": 0,
        "session_chunk_count": 3
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '2 days'
);

-- Chunk 1: assistant turn
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'a0000000-0000-0000-0000-000000000003'::UUID,
    'episodic',
    'assistant: Hi! How can I help you today?',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-a",
        "is_chunk": true,
        "chunk_index": 1,
        "session_chunk_count": 3
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '2 days'
);

-- Chunk 2: user turn
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'a0000000-0000-0000-0000-000000000004'::UUID,
    'episodic',
    'user: I need to know about fractal memory',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-a",
        "is_chunk": true,
        "chunk_index": 2,
        "session_chunk_count": 3
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '2 days'
);

-- =============================================================================
-- Pattern A (continued): Another multi-chunk session with assistant first
-- 2 chunks: assistant then user
-- =============================================================================

-- Raw node
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'b0000000-0000-0000-0000-000000000001'::UUID,
    'episodic',
    'assistant: Here is your daily summary
user: Great, thank you!',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-b",
        "is_full_content": true,
        "chunk_ids": [
            "b0000000-0000-0000-0000-000000000002",
            "b0000000-0000-0000-0000-000000000003"
        ]
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '1 day'
);

-- Chunk 0: assistant
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'b0000000-0000-0000-0000-000000000002'::UUID,
    'episodic',
    'assistant: Here is your daily summary',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-b",
        "is_chunk": true,
        "chunk_index": 0,
        "session_chunk_count": 2
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '1 day'
);

-- Chunk 1: user
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'b0000000-0000-0000-0000-000000000003'::UUID,
    'episodic',
    'user: Great, thank you!',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-b",
        "is_chunk": true,
        "chunk_index": 1,
        "session_chunk_count": 2
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '1 day'
);

-- =============================================================================
-- Pattern B: Single-turn session (turn-by-turn, not chunked)
-- 2 turns stored independently with turn_index
-- =============================================================================

-- Turn 0
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'c0000000-0000-0000-0000-000000000001'::UUID,
    'episodic',
    'user: What is the weather?',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-c",
        "turn_index": 0
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '3 hours'
);

-- Turn 1
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'c0000000-0000-0000-0000-000000000002'::UUID,
    'episodic',
    'assistant: The weather today is sunny with a high of 72°F.',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-c",
        "turn_index": 1
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '3 hours'
);

-- =============================================================================
-- Edge Case 1: Memory with NO session_id — should be IGNORED
-- =============================================================================
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'e0000000-0000-0000-0000-000000000001'::UUID,
    'semantic',
    'Standalone memory with no session',
    NULL,  -- vector(1024) — NULL for test
    '{"source": "manual"}'::JSONB,
    'manual',
    NOW() - INTERVAL '5 days'
);

-- =============================================================================
-- Edge Case 2: Memory with session_id but no turn_index or chunk_index — skipped
-- =============================================================================
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'e0000000-0000-0000-0000-000000000002'::UUID,
    'episodic',
    'anon: This memory has session_id but no turn info at all',
    NULL,  -- vector(1024) — NULL for test
    '{"session_id": "test-session-orphan"}'::JSONB,
    'conversation',
    NOW() - INTERVAL '4 days'
);

-- =============================================================================
-- Edge Case 3: Already-migrated memory (turn_id IS NOT NULL) — skipped
-- Pre-create the session and turn, then link the memory
-- =============================================================================

-- Session
INSERT INTO conversation_sessions (id, external_id, started_at)
VALUES (
    'd0000000-0000-0000-0000-000000000001'::UUID,
    'test-session-d',
    NOW() - INTERVAL '6 hours'
) ON CONFLICT (external_id) DO NOTHING;

-- Turn
INSERT INTO conversation_turns (id, session_id, turn_index, speaker_role, content, created_at)
VALUES (
    'd0000000-0000-0000-0000-000000000002'::UUID,
    'd0000000-0000-0000-0000-000000000001'::UUID,
    0,
    'user',
    'Already migrated content',
    NOW() - INTERVAL '6 hours'
) ON CONFLICT (session_id, turn_index) DO NOTHING;

-- Memory with turn_id already set
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at, turn_id
) VALUES (
    'd0000000-0000-0000-0000-000000000003'::UUID,
    'episodic',
    'user: Already migrated content',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-d",
        "is_chunk": true,
        "chunk_index": 0,
        "session_chunk_count": 1
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '6 hours',
    'd0000000-0000-0000-0000-000000000002'::UUID
);

-- =============================================================================
-- Edge Case 4: Chunk with unusual speaker roles (system, tool)
-- =============================================================================

-- Raw node
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'f0000000-0000-0000-0000-000000000001'::UUID,
    'episodic',
    'system: Initializing session
user: Start the analysis
tool: Analysis complete',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-e",
        "is_full_content": true,
        "chunk_ids": ["f0000000-0000-0000-0000-000000000002",
                      "f0000000-0000-0000-0000-000000000003",
                      "f0000000-0000-0000-0000-000000000004"]
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '12 hours'
);

-- System chunk
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'f0000000-0000-0000-0000-000000000002'::UUID,
    'episodic',
    'system: Initializing session',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-e",
        "is_chunk": true,
        "chunk_index": 0,
        "session_chunk_count": 3
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '12 hours'
);

-- User chunk
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'f0000000-0000-0000-0000-000000000003'::UUID,
    'episodic',
    'user: Start the analysis',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-e",
        "is_chunk": true,
        "chunk_index": 1,
        "session_chunk_count": 3
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '12 hours'
);

-- Tool chunk
INSERT INTO memories (
    id, memory_type, content, embedding, metadata, source, created_at
) VALUES (
    'f0000000-0000-0000-0000-000000000004'::UUID,
    'episodic',
    'tool: Analysis complete',
    NULL,  -- vector(1024) — NULL for test
    '{
        "session_id": "test-session-e",
        "is_chunk": true,
        "chunk_index": 2,
        "session_chunk_count": 3
    }'::JSONB,
    'conversation',
    NOW() - INTERVAL '12 hours'
);

-- =============================================================================
-- Summary of test data:
--   Session A: 3 chunks (user, assistant, user) + 1 raw node = 4 memories
--              → expects 3 turns, 0 raw skipped
--   Session B: 2 chunks (assistant, user) + 1 raw node = 3 memories
--              → expects 2 turns, 0 raw skipped
--   Session C: 2 single-turn records = 2 memories
--              → expects 2 turns
--   Session D: 1 already-migrated memory = 1 memory
--              → expects 0 new turns
--   Session E: 3 chunks (system, user, tool) + 1 raw node = 4 memories
--              → expects 3 turns
--   Edge: 1 no-session memory → ignored
--   Edge: 1 no-turn-index memory → skipped
--
--   TOTAL: 16 memories (15 with session_id, 1 without)
--   ELIGIBLE: 10 turn records created (A:3 + B:2 + C:2 + E:3)
--   SKIPPED: 1 already-migrated (D) + 3 raw nodes + 1 no-turn-index + 1 no-session = 6
--   POST-MIGRATION: 11 total test turns (10 new + 1 pre-existing D)
--   SESSIONS: 5 (A, B, C, D pre-existing, E)
--   SPEAKERS: 6 user, 3 assistant, 1 system, 1 tool
-- =============================================================================

COMMIT;

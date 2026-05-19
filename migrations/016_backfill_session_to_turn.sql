-- =============================================================================
-- Migration 016: Backfill Session-Aggregate Data → Turn-Level Storage
-- =============================================================================
--
-- Converts existing session records from the `memories` table into first-class
-- `conversation_sessions` and `conversation_turns` records. Handles two storage
-- patterns:
--
--   Pattern A (multi-chunk):  Sessions chunked via `chunk_into_rounds()`.
--                             Each chunk has `metadata.is_chunk = true` and
--                             `metadata.chunk_index` (0-based). A raw node holds
--                             the full session (`metadata.is_full_content = true`).
--
--   Pattern B (single-turn): Sessions stored turn-by-turn with
--                             `metadata.turn_index` and `metadata.session_id`.
--
-- IDEMPOTENCY: Re-running this migration is safe. It only processes memories
-- where `turn_id IS NULL` (not yet migrated) and uses ON CONFLICT DO NOTHING
-- for turn records. Already-migrated rows are skipped.
--
-- ROLLBACK: See 016_backfill_session_to_turn.down.sql
--
-- VERIFICATION: After migration, verify with:
--   SELECT COUNT(*) FROM conversation_turns WHERE created_at > (SELECT MAX(created_at) FROM schema_migrations WHERE version = '016_backfill_session_to_turn');
--   SELECT COUNT(*) FROM memories WHERE metadata->>'session_id' IS NOT NULL AND turn_id IS NOT NULL;
--
-- Design: docs/turn-level-schema-design.md §5.3
-- =============================================================================

BEGIN;

-- =============================================================================
-- Helper: Infer speaker role from content prefix patterns.
-- Mirrors: src/memory/conversation.rs → SpeakerRole::infer_from_content()
--          scripts/backfill_turn_storage.py → infer_speaker_role()
-- =============================================================================
CREATE OR REPLACE FUNCTION infer_speaker_role(content TEXT)
RETURNS VARCHAR(20) AS $$
DECLARE
    lower_text TEXT;
BEGIN
    lower_text := lower(trim(content));
    IF lower_text LIKE 'user:%' OR lower_text LIKE 'human:%' THEN
        RETURN 'user';
    ELSIF lower_text LIKE 'assistant:%' OR lower_text LIKE 'ai:%' THEN
        RETURN 'assistant';
    ELSIF lower_text LIKE 'system:%' THEN
        RETURN 'system';
    ELSIF lower_text LIKE 'tool:%' OR lower_text LIKE 'function:%' THEN
        RETURN 'tool';
    END IF;
    -- Heuristic: assistant-like phrasing
    IF lower_text LIKE '%i can%' OR lower_text LIKE '%i''ll%' OR lower_text LIKE '%here is%' THEN
        RETURN 'assistant';
    END IF;
    RETURN 'user';
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- =============================================================================
-- Helper: Determine turn_index from metadata.
-- Pattern A (chunked): metadata->>'chunk_index'  (0-based)
-- Pattern B (single):  metadata->>'turn_index'   (0-based from request)
-- Returns NULL for non-turn records (raw nodes, overviews).
-- =============================================================================
CREATE OR REPLACE FUNCTION get_turn_index(metadata JSONB)
RETURNS INTEGER AS $$
BEGIN
    -- Pattern A: chunked session
    IF metadata @> '{"is_chunk": true}' THEN
        RETURN (metadata->>'chunk_index')::INTEGER;
    END IF;
    -- Pattern B: single-turn
    IF metadata ? 'turn_index' THEN
        RETURN (metadata->>'turn_index')::INTEGER;
    END IF;
    RETURN NULL;  -- Not a turn record (raw node, overview, etc.)
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- =============================================================================
-- Helper: Check if a memory row is eligible for turn migration.
-- Excludes: raw nodes (is_full_content), overviews, already-migrated (turn_id NOT NULL),
--           rows without session_id, rows without determinable turn_index.
-- =============================================================================
CREATE OR REPLACE FUNCTION is_eligible_for_turn_migration(
    p_metadata JSONB,
    p_turn_id UUID
)
RETURNS BOOLEAN AS $$
BEGIN
    -- Already migrated
    IF p_turn_id IS NOT NULL THEN
        RETURN FALSE;
    END IF;
    -- Must have session_id
    IF p_metadata->>'session_id' IS NULL THEN
        RETURN FALSE;
    END IF;
    -- Skip raw full-content nodes (these are aggregates, not individual turns)
    IF p_metadata @> '{"is_full_content": true}' THEN
        RETURN FALSE;
    END IF;
    -- Must have a determinable turn_index
    IF get_turn_index(p_metadata) IS NULL THEN
        RETURN FALSE;
    END IF;
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- =============================================================================
-- MAIN MIGRATION: Process all eligible memories and create turn records.
-- =============================================================================
DO $$
DECLARE
    rec RECORD;
    v_session_uuid UUID;
    v_turn_uuid UUID;
    v_turn_index INTEGER;
    v_speaker_role VARCHAR(20);
    v_processed INTEGER := 0;
    v_skipped_already_migrated INTEGER := 0;
    v_skipped_no_session INTEGER := 0;
    v_skipped_raw_node INTEGER := 0;
    v_skipped_no_turn_index INTEGER := 0;
    v_errors INTEGER := 0;
    v_session_cache_count INTEGER := 0;
    v_total_eligible INTEGER;
BEGIN
    -- Count eligible rows
    SELECT COUNT(*) INTO v_total_eligible
    FROM memories
    WHERE metadata->>'session_id' IS NOT NULL;

    RAISE NOTICE 'Found % total memories with session_id metadata', v_total_eligible;
    RAISE NOTICE 'Processing eligible records (chunk nodes and single-turn nodes only)...';

    -- Process all eligible records in a single pass
    FOR rec IN
        SELECT
            id,
            content,
            embedding,
            metadata,
            created_at,
            turn_id
        FROM memories
        WHERE metadata->>'session_id' IS NOT NULL
        ORDER BY created_at
    LOOP
        -- Check eligibility
        IF NOT is_eligible_for_turn_migration(rec.metadata, rec.turn_id) THEN
            -- Count skip reasons for reporting
            IF rec.turn_id IS NOT NULL THEN
                v_skipped_already_migrated := v_skipped_already_migrated + 1;
            ELSIF rec.metadata @> '{"is_full_content": true}' THEN
                v_skipped_raw_node := v_skipped_raw_node + 1;
            ELSIF get_turn_index(rec.metadata) IS NULL THEN
                v_skipped_no_turn_index := v_skipped_no_turn_index + 1;
            ELSIF rec.metadata->>'session_id' IS NULL THEN
                v_skipped_no_session := v_skipped_no_session + 1;
            END IF;
            CONTINUE;
        END IF;

        BEGIN
            -- 1. Upsert conversation session (by external_id)
            INSERT INTO conversation_sessions (external_id, started_at)
            VALUES (rec.metadata->>'session_id', COALESCE(rec.created_at, NOW()))
            ON CONFLICT (external_id) DO NOTHING;

            -- Get the session UUID (either newly created or existing)
            SELECT id INTO v_session_uuid
            FROM conversation_sessions
            WHERE external_id = rec.metadata->>'session_id';

            IF v_session_uuid IS NULL THEN
                v_errors := v_errors + 1;
                RAISE WARNING 'Failed to find/create session for external_id=%', rec.metadata->>'session_id';
                CONTINUE;
            END IF;

            -- 2. Determine turn_index and speaker_role
            v_turn_index := get_turn_index(rec.metadata);
            v_speaker_role := infer_speaker_role(COALESCE(rec.content, ''));

            -- 3. Upsert conversation turn (idempotent on unique_turn constraint)
            INSERT INTO conversation_turns (
                session_id, turn_index, speaker_role, content, embedding, metadata, created_at
            ) VALUES (
                v_session_uuid,
                v_turn_index,
                v_speaker_role,
                COALESCE(rec.content, ''),
                rec.embedding,
                rec.metadata::JSONB,
                COALESCE(rec.created_at, NOW())
            )
            ON CONFLICT (session_id, turn_index) DO NOTHING
            RETURNING id INTO v_turn_uuid;

            -- If a conflict occurred (already exists), fetch existing id
            IF v_turn_uuid IS NULL THEN
                SELECT id INTO v_turn_uuid
                FROM conversation_turns
                WHERE session_id = v_session_uuid AND turn_index = v_turn_index;
            END IF;

            -- 4. Link memory → turn (idempotent: only sets if currently NULL)
            UPDATE memories
            SET turn_id = v_turn_uuid
            WHERE id = rec.id AND turn_id IS NULL;

            v_processed := v_processed + 1;

        EXCEPTION WHEN OTHERS THEN
            v_errors := v_errors + 1;
            RAISE WARNING 'Error processing memory id=%: %', rec.id, SQLERRM;
            IF v_errors > 100 THEN
                RAISE EXCEPTION 'Too many errors (%), aborting migration', v_errors;
            END IF;
        END;
    END LOOP;

    -- 5. Update session turn_counts and updated_at
    UPDATE conversation_sessions cs
    SET
        turn_count = (SELECT COUNT(*) FROM conversation_turns WHERE session_id = cs.id),
        updated_at = NOW()
    WHERE cs.external_id IN (
        SELECT DISTINCT metadata->>'session_id'
        FROM memories
        WHERE metadata->>'session_id' IS NOT NULL
    );

    -- Report final stats
    SELECT COUNT(*) INTO v_session_cache_count
    FROM conversation_sessions cs
    WHERE cs.external_id IN (
        SELECT DISTINCT metadata->>'session_id'
        FROM memories
        WHERE metadata->>'session_id' IS NOT NULL
    );

    RAISE NOTICE '========================================';
    RAISE NOTICE 'Migration 016 Complete:';
    RAISE NOTICE '  Processed:        % turns created', v_processed;
    RAISE NOTICE '  Sessions:         %', v_session_cache_count;
    RAISE NOTICE '  Skipped (already migrated): %', v_skipped_already_migrated;
    RAISE NOTICE '  Skipped (raw nodes):        %', v_skipped_raw_node;
    RAISE NOTICE '  Skipped (no turn_index):    %', v_skipped_no_turn_index;
    RAISE NOTICE '  Skipped (no session_id):    %', v_skipped_no_session;
    RAISE NOTICE '  Errors:            %', v_errors;
    RAISE NOTICE '========================================';
END;
$$;

-- =============================================================================
-- Schema version
-- =============================================================================
INSERT INTO schema_migrations (version) VALUES ('016_backfill_session_to_turn')
ON CONFLICT (version) DO NOTHING;

COMMIT;

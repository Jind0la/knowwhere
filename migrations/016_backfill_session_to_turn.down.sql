-- =============================================================================
-- Migration 016 DOWN: Rollback session-to-turn backfill
-- =============================================================================
--
-- Reverses the backfill by:
--   1. Clearing memories.turn_id for all records touched by this migration
--   2. Deleting conversation_turns created during the backfill
--   3. Optionally cleaning up conversation_sessions that now have zero turns
--
-- SAFETY: This only removes turn records that were linked from memories
-- with session_id metadata. Turns created via the normal store_turn code path
-- (which links via the application, not via memories.turn_id) are preserved.
-- =============================================================================

BEGIN;

DO $$
DECLARE
    v_turns_deleted INTEGER := 0;
    v_links_cleared INTEGER := 0;
    v_sessions_cleaned INTEGER := 0;
BEGIN
    RAISE NOTICE 'Rolling back migration 016_backfill_session_to_turn...';

    -- 1. Delete conversation_turns that were created by the backfill.
    --    These are identified by having a matching memory with turn_id set.
    DELETE FROM conversation_turns ct
    USING memories m
    WHERE ct.id = m.turn_id
      AND m.metadata->>'session_id' IS NOT NULL
      AND m.turn_id IS NOT NULL;
    GET DIAGNOSTICS v_turns_deleted = ROW_COUNT;

    -- 2. Clear turn_id links for all migrated memories
    UPDATE memories
    SET turn_id = NULL
    WHERE metadata->>'session_id' IS NOT NULL
      AND turn_id IS NOT NULL;
    GET DIAGNOSTICS v_links_cleared = ROW_COUNT;

    -- 3. Clean up conversation_sessions that now have zero turns
    --    Only delete sessions that were created for backfill (no remaining turns)
    DELETE FROM conversation_sessions cs
    WHERE cs.id IN (
        SELECT DISTINCT m2.turn_session_id
        FROM (
            -- Get session IDs that had memories linked to them
            SELECT DISTINCT ct_inner.session_id AS turn_session_id
            FROM memories m_inner
            JOIN conversation_turns ct_inner ON ct_inner.id = m_inner.turn_id
            WHERE m_inner.metadata->>'session_id' IS NOT NULL
            UNION
            SELECT DISTINCT cs2.id
            FROM conversation_sessions cs2
            WHERE cs2.external_id IN (
                SELECT m3.metadata->>'session_id'
                FROM memories m3
                WHERE m3.metadata->>'session_id' IS NOT NULL
            )
        ) m2
        WHERE NOT EXISTS (
            SELECT 1 FROM conversation_turns ct2
            WHERE ct2.session_id = m2.turn_session_id
        )
    );
    GET DIAGNOSTICS v_sessions_cleaned = ROW_COUNT;

    RAISE NOTICE '========================================';
    RAISE NOTICE 'Rollback Complete:';
    RAISE NOTICE '  Turns deleted:      %', v_turns_deleted;
    RAISE NOTICE '  Memory links cleared: %', v_links_cleared;
    RAISE NOTICE '  Sessions cleaned:   %', v_sessions_cleaned;
    RAISE NOTICE '========================================';
END;
$$;

-- Remove helpers created by the up migration (clean up after ourselves)
DROP FUNCTION IF EXISTS infer_speaker_role(TEXT);
DROP FUNCTION IF EXISTS get_turn_index(JSONB);
DROP FUNCTION IF EXISTS is_eligible_for_turn_migration(JSONB, UUID);

-- Remove the schema_migrations entry
DELETE FROM schema_migrations WHERE version = '016_backfill_session_to_turn';

COMMIT;

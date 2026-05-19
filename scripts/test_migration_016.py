#!/usr/bin/env python3
"""
Verification script for Migration 016: Session-to-Turn Backfill.

Tests:
  1. Seed test dataset
  2. Run the up migration
  3. Verify row counts match expectations
  4. Test idempotency (re-run migration, verify no duplicates)
  5. Test rollback (run down migration, verify cleanup)
  6. Re-run up migration (verify re-migration works after rollback)

Usage:
    # Full test cycle
    python3 scripts/test_migration_016.py

    # Dry run (only verify counts, no migration execution)
    DRY_RUN=1 python3 scripts/test_migration_016.py

    # Custom DB
    DATABASE_URL=postgresql://localhost:5432/mydb python3 scripts/test_migration_016.py

Expected test dataset:
    Session A: 3 chunks + 1 raw      → 3 turns created, 1 raw skipped
    Session B: 2 chunks + 1 raw      → 2 turns created, 1 raw skipped
    Session C: 2 single-turn records → 2 turns created
    Session D: 1 already-migrated    → 0 turns created (skip)
    Session E: 3 chunks + 1 raw      → 3 turns created, 1 raw skipped
    Edge 1: 1 no-session memory     → 0 turns created (skip)
    Edge 2: 1 no-turn-index memory  → 0 turns created (skip)

    TOTAL ELIGIBLE: 3+2+2+0+3 = 10 turn records
    TOTAL SKIPPED:  4 raw + 1 migrated + 1 no-session + 1 no-turn-index = 7
"""

import asyncio
import os
import sys
import subprocess
from datetime import datetime, timezone
from pathlib import Path

try:
    import asyncpg
except ImportError:
    print("ERROR: asyncpg required. Install with: pip install asyncpg")
    sys.exit(1)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
PROJECT_ROOT = Path(__file__).resolve().parent.parent
DATABASE_URL = os.environ.get(
    "DATABASE_URL", "postgresql://127.0.0.1:5433/hindsight"
)
DRY_RUN = os.environ.get("DRY_RUN", "").lower() in ("1", "true", "yes")

MIGRATION_UP = PROJECT_ROOT / "migrations" / "016_backfill_session_to_turn.sql"
MIGRATION_DOWN = (
    PROJECT_ROOT / "migrations" / "016_backfill_session_to_turn.down.sql"
)
TEST_DATA = PROJECT_ROOT / "migrations" / "test_data_016.sql"

# Expected results after migration
# New turns: 3(A) + 2(B) + 2(C) + 0(D, already migrated) + 3(E) = 10
# Pre-existing: session D has 1 turn from test data = 1
# Total test turns after migration: 10 + 1 = 11
EXPECTED_NEW_TURNS = 10
EXPECTED_TOTAL_TEST_TURNS = 11  # 10 new + 1 pre-existing (session D)
EXPECTED_SESSIONS = 5  # A, B, C, D (pre-existing), E
EXPECTED_LINKED_MEMORIES = 11  # 10 new links + 1 pre-existing (session D)
SKIPPED_RAW_NODES = 3  # A, B, E each have 1 raw node


def run_sql(filepath: Path) -> subprocess.CompletedProcess:
    """Run a SQL file against the database using psql."""
    cmd = [
        "psql",
        DATABASE_URL,
        "-v",
        "ON_ERROR_STOP=1",
        "-f",
        str(filepath),
    ]
    if DRY_RUN:
        print(f"  [DRY RUN] Would run: {' '.join(cmd)}")
        return subprocess.CompletedProcess(args=cmd, returncode=0, stdout=b"", stderr=b"")
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60)


async def verify_counts(conn: asyncpg.Connection, label: str) -> dict:
    """Query current state and compare against expectations."""
    results = {}

    # Total memories with session_id
    results["total_memories_with_session"] = await conn.fetchval(
        "SELECT COUNT(*) FROM memories WHERE metadata->>'session_id' IS NOT NULL"
    )

    # Total memories with session_id AND turn_id set (linked)
    results["linked_memories"] = await conn.fetchval(
        "SELECT COUNT(*) FROM memories WHERE metadata->>'session_id' IS NOT NULL AND turn_id IS NOT NULL"
    )

    # Raw nodes (is_full_content = true)
    results["raw_nodes"] = await conn.fetchval(
        "SELECT COUNT(*) FROM memories WHERE metadata @> '{\"is_full_content\": true}'"
    )

    # Chunk nodes (is_chunk = true)
    results["chunk_nodes"] = await conn.fetchval(
        "SELECT COUNT(*) FROM memories WHERE metadata @> '{\"is_chunk\": true}'"
    )

    # Conversation sessions
    results["total_sessions"] = await conn.fetchval(
        "SELECT COUNT(*) FROM conversation_sessions"
    )

    # Sessions created for test data
    results["test_sessions"] = await conn.fetchval(
        "SELECT COUNT(*) FROM conversation_sessions WHERE external_id LIKE 'test-session-%'"
    )

    # Conversation turns total
    results["total_turns"] = await conn.fetchval(
        "SELECT COUNT(*) FROM conversation_turns"
    )

    # Turns in test sessions
    results["test_turns"] = await conn.fetchval(
        """SELECT COUNT(*) FROM conversation_turns ct
           JOIN conversation_sessions cs ON ct.session_id = cs.id
           WHERE cs.external_id LIKE 'test-session-%'"""
    )

    # Turns by session
    session_turns = await conn.fetch(
        """SELECT cs.external_id, COUNT(*) as turn_count
           FROM conversation_turns ct
           JOIN conversation_sessions cs ON ct.session_id = cs.id
           WHERE cs.external_id LIKE 'test-session-%'
           GROUP BY cs.external_id
           ORDER BY cs.external_id"""
    )
    results["session_turn_counts"] = {
        row["external_id"]: row["turn_count"] for row in session_turns
    }

    # Speaker roles in test turns
    speaker_counts = await conn.fetch(
        """SELECT speaker_role, COUNT(*) as cnt
           FROM conversation_turns ct
           JOIN conversation_sessions cs ON ct.session_id = cs.id
           WHERE cs.external_id LIKE 'test-session-%'
           GROUP BY speaker_role
           ORDER BY speaker_role"""
    )
    results["speaker_counts"] = {
        row["speaker_role"]: row["cnt"] for row in speaker_counts
    }

    # Turn index ordering (spot check)
    results["turn_index_range_a"] = await conn.fetchval(
        """SELECT json_agg(turn_index ORDER BY turn_index)
           FROM conversation_turns ct
           JOIN conversation_sessions cs ON ct.session_id = cs.id
           WHERE cs.external_id = 'test-session-a'"""
    )

    print(f"\n--- {label} ---")
    print(f"  Memories with session_id:  {results['total_memories_with_session']}")
    print(f"  Memories linked (turn_id): {results['linked_memories']}")
    print(f"  Raw nodes (is_full_content): {results['raw_nodes']}")
    print(f"  Chunk nodes:               {results['chunk_nodes']}")
    print(f"  Conversation sessions:     {results['total_sessions']}  (test: {results['test_sessions']})")
    print(f"  Conversation turns:        {results['total_turns']}  (test: {results['test_turns']})")
    print(f"  Turn counts by session:    {results['session_turn_counts']}")
    print(f"  Speaker distribution:      {results['speaker_counts']}")
    print(f"  Session A turn order:      {results['turn_index_range_a']}")

    return results


async def verify_migration_assertions(results: dict, expected_turns: int):
    """Assert that migration results match expectations."""
    errors = []

    # Test turns created
    test_turns = results.get("test_turns", 0)
    if test_turns != expected_turns:
        errors.append(
            f"Expected {expected_turns} test turns, got {test_turns}"
        )

    # All eligible memories should be linked
    linked = results.get("linked_memories", 0)
    if linked != expected_turns:
        errors.append(
            f"Expected {expected_turns} linked memories, got {linked}"
        )

    # Session A should have 3 turns (user, assistant, user)
    session_counts = results.get("session_turn_counts", {})
    if session_counts.get("test-session-a") != 3:
        errors.append(
            f"Session A: expected 3 turns, got {session_counts.get('test-session-a')}"
        )
    if session_counts.get("test-session-b") != 2:
        errors.append(
            f"Session B: expected 2 turns, got {session_counts.get('test-session-b')}"
        )
    if session_counts.get("test-session-c") != 2:
        errors.append(
            f"Session C: expected 2 turns, got {session_counts.get('test-session-c')}"
        )
    if session_counts.get("test-session-e") != 3:
        errors.append(
            f"Session E: expected 3 turns, got {session_counts.get('test-session-e')}"
        )

    # Session D should NOT have any NEW turns (already had 1)
    # (the pre-existing turn from test_data should still be there)
    if session_counts.get("test-session-d", 0) < 1:
        errors.append(
            f"Session D: expected at least 1 turn (pre-existing), got {session_counts.get('test-session-d', 0)}"
        )

    # Speaker distribution spot-check
    speaker_counts = results.get("speaker_counts", {})
    # Session A: user, assistant, user → at least these across all test sessions
    print(f"  Speaker check: user={speaker_counts.get('user')}, "
          f"assistant={speaker_counts.get('assistant')}, "
          f"system={speaker_counts.get('system')}, "
          f"tool={speaker_counts.get('tool')}")

    # Session A turn order: [0, 1, 2]
    turn_order = results.get("turn_index_range_a")
    if turn_order is not None:
        import json
        order = json.loads(turn_order) if isinstance(turn_order, str) else turn_order
        if order != [0, 1, 2]:
            errors.append(f"Session A turn order: expected [0,1,2], got {order}")

    if errors:
        print("\n❌ ASSERTION FAILURES:")
        for e in errors:
            print(f"  - {e}")
        return False
    else:
        print("\n✅ All assertions passed!")
        return True


async def cleanup_test_data(conn: asyncpg.Connection):
    """Remove all test data from the database."""
    print("\n--- Cleaning up test data ---")
    await conn.execute(
        """DELETE FROM conversation_turns
           WHERE session_id IN (
               SELECT id FROM conversation_sessions WHERE external_id LIKE 'test-session-%'
           )"""
    )
    await conn.execute(
        "DELETE FROM conversation_sessions WHERE external_id LIKE 'test-session-%'"
    )
    await conn.execute(
        "DELETE FROM memories WHERE id::text LIKE '%000000-0000-0000-0000-%'"
    )
    print("  Test data cleaned.")


async def main():
    print(f"=== Migration 016 Test Suite ===")
    print(f"Database: {DATABASE_URL}")
    print(f"Dry run:  {DRY_RUN}")
    print(f"Up:       {MIGRATION_UP}")
    print(f"Down:     {MIGRATION_DOWN}")
    print(f"Test data: {TEST_DATA}")
    print()

    # -----------------------------------------------------------------------
    # Phase 0: Ensure prerequisite schema exists
    # -----------------------------------------------------------------------
    print("--- Phase 0: Ensuring prerequisite schema ---")
    # Create vector extension + base schema
    setup_sql = "CREATE EXTENSION IF NOT EXISTS vector;"
    result = subprocess.run(
        ["psql", DATABASE_URL, "-c", setup_sql],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        print(f"❌ Failed to create vector extension:\n{result.stderr}")
        sys.exit(1)

    result = run_sql(PROJECT_ROOT / "migrations" / "001_base_schema.sql")
    if result.returncode != 0:
        print(f"❌ Failed to apply base schema:\n{result.stderr}")
        sys.exit(1)

    # Create schema_migrations table (not in base schema)
    result = subprocess.run(
        ["psql", DATABASE_URL, "-c",
         "CREATE TABLE IF NOT EXISTS schema_migrations (version VARCHAR(255) PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW());"],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        print(f"❌ Failed to create schema_migrations:\n{result.stderr}")

    # Apply migrations 014 and 015
    for mig in ["014_add_turn_level_storage.sql", "015_drop_session_embedding.sql"]:
        result = run_sql(PROJECT_ROOT / "migrations" / mig)
        if result.returncode != 0:
            print(f"❌ Failed to apply {mig}:\n{result.stderr}")
            sys.exit(1)
    print("  ✅ Prerequisite schema ready.")

    # -----------------------------------------------------------------------
    # Phase 1: Seed test data
    # -----------------------------------------------------------------------
    print("--- Phase 1: Seeding test data ---")
    result = run_sql(TEST_DATA)
    if result.returncode != 0:
        print(f"❌ Failed to seed test data:\n{result.stderr}")
        sys.exit(1)
    print("  ✅ Test data seeded.")

    # -----------------------------------------------------------------------
    # Phase 2: Run up migration
    # -----------------------------------------------------------------------
    print("\n--- Phase 2: Running up migration ---")
    result = run_sql(MIGRATION_UP)
    if result.returncode != 0:
        print(f"❌ Migration failed:\n{result.stderr}")
        # Try to clean up
        conn = await asyncpg.connect(DATABASE_URL)
        try:
            await cleanup_test_data(conn)
        finally:
            await conn.close()
        sys.exit(1)
    print("  ✅ Migration complete.")
    # Show migration output
    if result.stdout:
        for line in result.stdout.strip().split("\n"):
            line = line.strip()
            if line:
                print(f"  {line}")

    # -----------------------------------------------------------------------
    # Phase 3: Verify results
    # -----------------------------------------------------------------------
    conn = await asyncpg.connect(DATABASE_URL)
    try:
        print("\n--- Phase 3: Verifying migration results ---")
        results = await verify_counts(conn, "After Migration")
        expected_total = EXPECTED_TOTAL_TEST_TURNS
        passed = await verify_migration_assertions(results, expected_total)
        if not passed:
            await cleanup_test_data(conn)
            sys.exit(1)

        # Save for later comparisons
        test_turns_initial = results.get("test_turns", 0)

        # -------------------------------------------------------------------
        # Phase 4: Test idempotency (re-run migration)
        # -------------------------------------------------------------------
        print("\n--- Phase 4: Testing idempotency ---")
        result2 = run_sql(MIGRATION_UP)
        if result2.returncode != 0:
            print(f"❌ Idempotency re-run failed:\n{result2.stderr}")
            await cleanup_test_data(conn)
            sys.exit(1)

        results2 = await verify_counts(conn, "After Idempotent Re-run")
        test_turns2 = results2.get("test_turns", 0)
        if test_turns2 != test_turns_initial:
            print(f"❌ Idempotency check failed: {test_turns2} turns after re-run (expected {test_turns_initial})")
            await cleanup_test_data(conn)
            sys.exit(1)
        print("  ✅ Idempotency confirmed — no duplicate turns created.")

        # -------------------------------------------------------------------
        # Phase 5: Test rollback (down migration)
        # -------------------------------------------------------------------
        print("\n--- Phase 5: Testing rollback ---")
        result3 = run_sql(MIGRATION_DOWN)
        if result3.returncode != 0:
            print(f"❌ Rollback failed:\n{result3.stderr}")
            await cleanup_test_data(conn)
            sys.exit(1)
        print("  ✅ Down migration complete.")
        if result3.stdout:
            for line in result3.stdout.strip().split("\n"):
                line = line.strip()
                if line:
                    print(f"  {line}")

        results3 = await verify_counts(conn, "After Rollback")
        test_turns3 = results3.get("test_turns", 0)
        linked3 = results3.get("linked_memories", 0)
        if test_turns3 != 0:
            print(f"❌ Rollback failed: {test_turns3} test turns remain (expected 0)")
            sys.exit(1)
        if linked3 != 0:
            print(f"❌ Rollback failed: {linked3} memories still linked (expected 0)")
            sys.exit(1)
        print("  ✅ Rollback confirmed — all turns removed, links cleared.")

        # -------------------------------------------------------------------
        # Phase 6: Re-run up migration after rollback
        # -------------------------------------------------------------------
        print("\n--- Phase 6: Re-migrating after rollback ---")
        result4 = run_sql(MIGRATION_UP)
        if result4.returncode != 0:
            print(f"❌ Re-migration failed:\n{result4.stderr}")
            sys.exit(1)

        results4 = await verify_counts(conn, "After Re-Migration")
        test_turns4 = results4.get("test_turns", 0)
        if test_turns4 != test_turns_initial:
            print(f"❌ Re-migration check failed: {test_turns4} turns (expected {test_turns_initial})")
            sys.exit(1)
        print("  ✅ Re-migration restored all turn records.")

        # -------------------------------------------------------------------
        # Cleanup
        # -------------------------------------------------------------------
        await cleanup_test_data(conn)
        print("\n🎉 All tests passed — Migration 016 is working correctly!")

    finally:
        await conn.close()


if __name__ == "__main__":
    asyncio.run(main())

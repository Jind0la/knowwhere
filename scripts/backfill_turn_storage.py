#!/usr/bin/env python3
"""
Migration script: backfill existing session-chunk data into turn-level storage.

Scans the `memories` table for rows with `metadata->>'session_id'` set,
creates corresponding `conversation_sessions` and `conversation_turns` records,
and links `memories.turn_id` to the new turn.

Usage:
    BACKFILL_DRY_RUN=1 python backfill_turn_storage.py   # preview only
    python backfill_turn_storage.py                        # execute

Requires:
    DATABASE_URL env var (PostgreSQL connection string)
    psycopg2 or asyncpg

Design: docs/turn-level-schema-design.md §5.3
"""

import asyncio
import os
import sys
from datetime import datetime, timezone

try:
    import asyncpg
except ImportError:
    print("ERROR: asyncpg required. Install with: pip install asyncpg")
    sys.exit(1)

DRY_RUN = os.environ.get("BACKFILL_DRY_RUN", "").lower() in ("1", "true", "yes")
DATABASE_URL = os.environ.get("DATABASE_URL", "postgresql://127.0.0.1:5433/hindsight")

BATCH_SIZE = 100  # process rows in batches


def infer_speaker_role(content: str) -> str:
    """Infer speaker role from content prefix patterns (same logic as Rust SpeakerRole::infer_from_content)."""
    lower = content.strip().lower()
    if lower.startswith("user:") or lower.startswith("human:"):
        return "user"
    if lower.startswith("assistant:") or lower.startswith("ai:"):
        return "assistant"
    if lower.startswith("system:"):
        return "system"
    if lower.startswith("tool:") or lower.startswith("function:"):
        return "tool"
    # Heuristic
    if "i can" in lower or "i'll" in lower or "here is" in lower:
        return "assistant"
    return "user"


async def backfill(conn: asyncpg.Connection):
    # Count total eligible rows
    total = await conn.fetchval(
        "SELECT COUNT(*) FROM memories WHERE metadata->>'session_id' IS NOT NULL"
    )
    print(f"Found {total} memories with session_id metadata")

    if DRY_RUN:
        print("DRY RUN — no changes will be made\n")
        rows = await conn.fetch(
            "SELECT id, metadata FROM memories WHERE metadata->>'session_id' IS NOT NULL LIMIT 20"
        )
        for row in rows:
            meta = row["metadata"]
            sid = meta.get("session_id", "?")
            turn_idx = meta.get("turn_index", "?")
            print(f"  memory={row['id']} session={sid} turn_index={turn_idx}")
        print(f"\nWould process {total} rows across {total // BATCH_SIZE + 1} batches")
        return

    session_cache: dict[str, str] = {}  # external_id -> session_uuid
    processed = 0
    errors = 0

    offset = 0
    while offset < total:
        rows = await conn.fetch(
            """SELECT id, content, embedding, metadata, created_at
               FROM memories
               WHERE metadata->>'session_id' IS NOT NULL
               ORDER BY created_at
               LIMIT $1 OFFSET $2""",
            BATCH_SIZE,
            offset,
        )

        async with conn.transaction():
            for row in rows:
                try:
                    meta = row["metadata"]
                    external_id = meta.get("session_id")
                    turn_index = meta.get("turn_index", 0)
                    if external_id is None:
                        continue

                    # Upsert session
                    if external_id not in session_cache:
                        existing = await conn.fetchval(
                            "SELECT id FROM conversation_sessions WHERE external_id = $1",
                            external_id,
                        )
                        if existing:
                            session_cache[external_id] = str(existing)
                        else:
                            new_sid = await conn.fetchval(
                                """INSERT INTO conversation_sessions (external_id, started_at)
                                   VALUES ($1, $2) RETURNING id""",
                                external_id,
                                row["created_at"] or datetime.now(timezone.utc),
                            )
                            session_cache[external_id] = str(new_sid)

                    session_uuid = session_cache[external_id]

                    # Upsert turn
                    speaker = infer_speaker_role(row["content"])
                    turn_id = await conn.fetchval(
                        """INSERT INTO conversation_turns
                           (session_id, turn_index, speaker_role, content, embedding, metadata, created_at)
                           VALUES ($1, $2, $3, $4, $5, $6, $7)
                           ON CONFLICT (session_id, turn_index) DO UPDATE
                           SET content = EXCLUDED.content,
                               embedding = EXCLUDED.embedding,
                               speaker_role = EXCLUDED.speaker_role
                           RETURNING id""",
                        session_uuid,
                        turn_index,
                        speaker,
                        row["content"],
                        row["embedding"],
                        meta,
                        row["created_at"] or datetime.now(timezone.utc),
                    )

                    # Link memory -> turn
                    await conn.execute(
                        "UPDATE memories SET turn_id = $1 WHERE id = $2",
                        turn_id,
                        row["id"],
                    )

                    processed += 1

                except Exception as e:
                    errors += 1
                    print(f"  ERROR memory={row['id']}: {e}", file=sys.stderr)
                    if errors > 50:
                        print("Too many errors, aborting", file=sys.stderr)
                        raise

        # Recompute session embeddings for affected sessions
        for ext_id, sid in session_cache.items():
            await conn.execute(
                """UPDATE conversation_sessions
                   SET embedding = (SELECT AVG(embedding) FROM conversation_turns
                                    WHERE session_id = $1 AND embedding IS NOT NULL),
                       turn_count = (SELECT COUNT(*) FROM conversation_turns WHERE session_id = $1),
                       updated_at = NOW()
                   WHERE id = $1""",
                sid,
            )

        offset += BATCH_SIZE
        print(f"  Processed {processed}/{total} ({errors} errors) — batch {offset // BATCH_SIZE}")

    print(f"\nDone. Processed {processed} memories, {errors} errors, {len(session_cache)} sessions created")


async def main():
    print(f"Connecting to {DATABASE_URL}...")
    conn = await asyncpg.connect(DATABASE_URL)
    try:
        await backfill(conn)
    finally:
        await conn.close()


if __name__ == "__main__":
    asyncio.run(main())

#!/usr/bin/env python3
"""
Import historical Hermes sessions into KnowWhere.

Reads session JSON files from ~/.hermes/sessions/, extracts user+assistant
messages as transcripts, and sends them to KnowWhere via store_session_batch
(one Ollama embed call per session — fast).

Usage:
  python3 import_hermes_sessions.py           # Import last 30 days
  python3 import_hermes_sessions.py --all     # Import everything
  python3 import_hermes_sessions.py --days 7  # Import last 7 days
  python3 import_hermes_sessions.py --dry-run # Show what would be imported
"""

import argparse
import json
import os
import sys
import time
import urllib.request
import urllib.error
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import List, Dict, Any

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

KNOWWHERE_ENDPOINT = os.getenv("KNOWWHERE_ENDPOINT", "http://127.0.0.1:3737")
KNOWWHERE_API_KEY = os.getenv("KNOWWHERE_API_KEY", "kw_testkey_12345")
HERMES_SESSIONS_DIR = Path.home() / ".hermes" / "sessions"
BATCH_SIZE = 20  # Sessions per batch (don't overwhelm Ollama)
REQUEST_TIMEOUT = 120  # Seconds per batch


# ---------------------------------------------------------------------------
# Session Discovery
# ---------------------------------------------------------------------------

def discover_sessions(since_days: int = None) -> List[Path]:
    """Find session JSON files, optionally filtered by age."""
    if not HERMES_SESSIONS_DIR.exists():
        print(f"Session directory not found: {HERMES_SESSIONS_DIR}")
        return []

    files = sorted(
        HERMES_SESSIONS_DIR.glob("session_*.json"),
        key=os.path.getmtime,
        reverse=True,
    )

    if since_days is not None:
        cutoff = datetime.now() - timedelta(days=since_days)
        cutoff_ts = cutoff.timestamp()
        files = [f for f in files if os.path.getmtime(f) >= cutoff_ts]

    return files


def extract_transcript(session_path: Path) -> Dict[str, Any]:
    """Extract user+assistant turns from a session JSON file.

    Returns a dict with session_id, session_start, and a list of turns.
    Each turn is a content string with role prefix.
    """
    try:
        with open(session_path) as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError):
        return None

    if not isinstance(data, dict) or "messages" not in data:
        return None

    messages = data.get("messages", [])
    if not messages:
        return None

    session_id = data.get("session_id", session_path.stem)
    session_start = data.get("session_start", "")
    platform = data.get("platform", "unknown")

    # Extract user + assistant turns, skip tool calls
    turns = []
    for msg in messages:
        role = msg.get("role", "")
        if role not in ("user", "assistant"):
            continue

        content = msg.get("content", "")
        if isinstance(content, list):
            # Multimodal content — extract text parts
            content = " ".join(
                p.get("text", "") for p in content if isinstance(p, dict)
            )

        if not content or not content.strip():
            continue

        # Truncate very long messages
        if len(content) > 2000:
            content = content[:2000]

        turns.append(f"[{role}] {content}")

    if not turns:
        return None

    return {
        "session_id": session_id,
        "session_start": session_start,
        "platform": platform,
        "turns": turns,
        "turn_count": len(turns),
    }


# ---------------------------------------------------------------------------
# KnowWhere API
# ---------------------------------------------------------------------------

def kw_health() -> bool:
    """Check if KnowWhere is reachable."""
    try:
        req = urllib.request.Request(
            f"{KNOWWHERE_ENDPOINT}/health",
            headers={"Authorization": f"Bearer {KNOWWHERE_API_KEY}"},
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())
            return data.get("status") == "ok"
    except Exception:
        return False


def kw_store_batch(sessions: List[Dict[str, Any]]) -> Dict[str, Any]:
    """POST /store_session_batch — store multiple sessions in one call.

    Each session is stored as one node with its full transcript.
    Uses batch embedding (one Ollama call for all chunks across all sessions).
    """
    payload = []
    for session in sessions:
        transcript = "\n".join(session["turns"])
        payload.append({
            "content": transcript,
                "source": "conversation",
            "memory_type": "episodic",
            "metadata": {
                "source": "hermes:import",
                "session_id": session["session_id"],
                "session_date": session["session_start"],
                "platform": session["platform"],
                "turn_count": session["turn_count"],
                "imported_at": datetime.now(timezone.utc).isoformat(),
                "trust_tier": "primary",
            },
        })

    body = json.dumps({"sessions": payload}).encode()
    req = urllib.request.Request(
        f"{KNOWWHERE_ENDPOINT}/store_session_batch",
        data=body,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {KNOWWHERE_API_KEY}",
        },
        method="POST",
    )

    with urllib.request.urlopen(req, timeout=REQUEST_TIMEOUT) as resp:
        return json.loads(resp.read().decode())


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Import Hermes sessions into KnowWhere fractal memory"
    )
    parser.add_argument(
        "--days", type=int, default=30,
        help="Import sessions from last N days (default: 30)"
    )
    parser.add_argument(
        "--all", action="store_true",
        help="Import all sessions (ignores --days)"
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Show what would be imported, don't store"
    )
    parser.add_argument(
        "--batch-size", type=int, default=BATCH_SIZE,
        help=f"Sessions per batch (default: {BATCH_SIZE})"
    )
    args = parser.parse_args()

    # Health check
    if not args.dry_run:
        print("Checking KnowWhere...", end=" ", flush=True)
        if not kw_health():
            print("❌ Unreachable")
            print(f"  Make sure KnowWhere is running at {KNOWWHERE_ENDPOINT}")
            sys.exit(1)
        print("✅")

    # Discover sessions
    since_days = None if args.all else args.days
    files = discover_sessions(since_days)
    print(f"\nFound {len(files)} session files", end="")
    if since_days:
        print(f" (last {since_days} days)")
    else:
        print(" (all time)")

    if not files:
        print("Nothing to import.")
        return

    # Extract transcripts
    print("Extracting transcripts...")
    sessions = []
    skipped = 0
    for f in files:
        transcript = extract_transcript(f)
        if transcript:
            sessions.append(transcript)
        else:
            skipped += 1

    total_turns = sum(s["turn_count"] for s in sessions)
    print(f"  {len(sessions)} valid sessions, {total_turns} turns, {skipped} skipped")

    if not sessions:
        print("No valid sessions to import.")
        return

    if args.dry_run:
        print("\n--- DRY RUN — would import ---")
        for s in sessions[:10]:
            print(f"  {s['session_id']}: {s['turn_count']} turns ({s['platform']})")
        if len(sessions) > 10:
            print(f"  ... and {len(sessions) - 10} more")
        print(f"\nTotal: {len(sessions)} sessions, {total_turns} turns")
        return

    # Import in batches
    print(f"\nImporting in batches of {args.batch_size}...")
    batches = [
        sessions[i : i + args.batch_size]
        for i in range(0, len(sessions), args.batch_size)
    ]

    imported = 0
    failed = 0
    start_time = time.time()

    for i, batch in enumerate(batches):
        batch_turns = sum(s["turn_count"] for s in batch)
        print(
            f"  Batch {i+1}/{len(batches)}: {len(batch)} sessions, {batch_turns} turns...",
            end=" ", flush=True,
        )

        try:
            result = kw_store_batch(batch)
            batch_ids = [r.get("id", "?")[:8] for r in result.get("results", [])]
            imported += len(batch)
            elapsed = time.time() - start_time
            rate = imported / elapsed * 60 if elapsed > 0 else 0
            print(f"✅ ({len(batch_ids)} stored, {rate:.0f} sess/min)")
        except urllib.error.HTTPError as e:
            failed += len(batch)
            body = e.read().decode()[:200]
            print(f"❌ HTTP {e.code}: {body}")
        except Exception as e:
            failed += len(batch)
            print(f"❌ {e}")

    # Summary
    elapsed = time.time() - start_time
    print(f"\n{'='*50}")
    print(f"Import complete: {imported} imported, {failed} failed")
    print(f"Time: {elapsed:.0f}s ({elapsed/60:.1f} min)")
    if imported > 0:
        print(f"Rate: {imported / elapsed * 60:.0f} sessions/min")
    print(f"\nVerify: curl {KNOWWHERE_ENDPOINT}/health")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""
KnowWhere Seed Script — Session-Transkripte → KnowWhere API

Extrahiert Hermes-Session-Transkripte und schiebt sie via /store_session
in eine laufende KnowWhere-Instanz.

Usage:
  python3 seed_knowwhere.py --kw-url https://knowwhere.railway.app --api-key xxx --limit 5
  python3 seed_knowwhere.py --kw-url http://localhost:3737 --theme knowwhere --limit 20
"""

import argparse
import json
import os
import sys
import time
import sqlite3
from pathlib import Path

# ── Session DB Pfad ──────────────────────────────────────────
SESSION_DB = Path.home() / ".hermes" / "sessions.db"

# ── Themen-Filter (Stichworte für relevante Sessions) ────────
THEMES = {
    "knowwhere": ["knowwhere", "memory", "retrieval", "fractal", "matryoshka", "embedding",
                   "hnsw", "rrf", "hybrid", "dream pipeline", "reranker", "usearch",
                   "voyage", "deepseek", "h-mem", "horma", "longmemeval", "benchmark"],
    "krankenfahrt": ["krankenfahrt", "fahrgast", "fahrlenker", "telegram bot",
                      "railway", "taxi", "transport"],
    "akn": ["adaptive knowledge network", "projektvorschlag", "produktmanagement",
            "weiterbildung", "wissensnetz"],
    "hermes": ["hermes agent", "operator", "era", "skill", "cron", "memory",
               "delegation", "orchestrator"],
    "leafgo": ["leafgo", "website", "onepager", "client"],
}


def get_sessions(theme: str = None, limit: int = 50) -> list[dict]:
    """Extrahiert Sessions aus der Hermes SQLite-DB."""
    if not SESSION_DB.exists():
        print(f"❌ Session-DB nicht gefunden: {SESSION_DB}")
        sys.exit(1)

    conn = sqlite3.connect(str(SESSION_DB))
    conn.row_factory = sqlite3.Row

    if theme and theme in THEMES:
        keywords = THEMES[theme]
        # Baue WHERE-Klausel mit LIKE für jedes Keyword
        conditions = " OR ".join([f"m.content LIKE '%{kw}%'" for kw in keywords])
        query = f"""
            SELECT DISTINCT s.id as session_id, s.title, s.started_at
            FROM sessions s
            JOIN messages m ON m.session_id = s.id
            WHERE m.role = 'user' AND ({conditions})
            ORDER BY s.started_at DESC
            LIMIT ?
        """
    else:
        query = """
            SELECT id as session_id, title, started_at
            FROM sessions
            ORDER BY started_at DESC
            LIMIT ?
        """

    sessions = []
    for row in conn.execute(query, (limit,)):
        sessions.append({
            "session_id": row["session_id"],
            "title": row["title"] or "Untitled",
            "started_at": row["started_at"],
            "turns": [],
        })

    # Lade Messages für jede Session
    for sess in sessions:
        rows = conn.execute("""
            SELECT role, content, created_at, turn_id
            FROM messages
            WHERE session_id = ? AND role IN ('user', 'assistant')
            ORDER BY created_at ASC
        """, (sess["session_id"],))

        for r in rows:
            if r["content"] and len(r["content"].strip()) > 10:
                sess["turns"].append({
                    "role": r["role"],
                    "content": r["content"].strip(),
                    "turn_id": r["turn_id"],
                })

    conn.close()

    # Filtere Sessions mit zu wenigen Turns
    sessions = [s for s in sessions if len(s["turns"]) >= 3]
    return sessions


def store_session(kw_url: str, api_key: str, content: str, source_type: str = "human",
                  session_id: str | None = None, metadata: dict | None = None) -> dict | None:
    """Sendet einen Turn an KnowWhere /store_session."""
    import urllib.request
    import urllib.error

    payload = {
        "content": content,
        "source_type": source_type,
    }
    if session_id:
        payload["session_id"] = session_id
    if metadata:
        payload["metadata"] = metadata  # type: ignore

    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{kw_url.rstrip('/')}/store_session",
        data=data,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        body = e.read().decode()[:200]
        print(f"  ⚠️ HTTP {e.code}: {body}")
        return None
    except Exception as e:
        print(f"  ⚠️ Fehler: {e}")
        return None


def main():
    parser = argparse.ArgumentParser(description="Seed KnowWhere mit Hermes-Sessions")
    parser.add_argument("--kw-url", required=True, help="KnowWhere URL (z.B. http://localhost:3737)")
    parser.add_argument("--api-key", default=os.getenv("KNOWWHERE_API_KEY", "kw_testkey_12345"),
                        help="KnowWhere API Key")
    parser.add_argument("--theme", choices=list(THEMES.keys()),
                        help="Nur Sessions zu einem Thema seeden")
    parser.add_argument("--limit", type=int, default=10,
                        help="Max Sessions (default: 10)")
    parser.add_argument("--dry-run", action="store_true",
                        help="Nur anzeigen, nicht senden")
    parser.add_argument("--delay", type=float, default=0.5,
                        help="Verzögerung zwischen API-Calls (Sekunden)")
    args = parser.parse_args()

    # ── Health Check ─────────────────────────────────────────
    import urllib.request
    try:
        with urllib.request.urlopen(f"{args.kw_url.rstrip('/')}/health", timeout=10) as resp:
            health = json.loads(resp.read())
            print(f"✅ KnowWhere erreichbar — Status: {health.get('status')}, "
                  f"Nodes: {health.get('node_count', '?')}")
    except Exception as e:
        print(f"❌ KnowWhere nicht erreichbar: {e}")
        sys.exit(1)

    # ── Sessions extrahieren ──────────────────────────────────
    print(f"\n📂 Extrahiere Sessions (theme={args.theme or 'alle'}, limit={args.limit})...")
    sessions = get_sessions(theme=args.theme, limit=args.limit)
    print(f"   {len(sessions)} Sessions mit ≥3 Turns gefunden.\n")

    if args.dry_run:
        for i, s in enumerate(sessions):
            print(f"  [{i+1}] {s['title'][:80]} — {len(s['turns'])} turns")
        print(f"\n🏁 Dry-run. Keine Daten gesendet.")
        return

    # ── In KnowWhere speichern ────────────────────────────────
    total_turns = 0
    stored = 0
    errors = 0

    for sess_idx, sess in enumerate(sessions):
        print(f"[{sess_idx+1}/{len(sessions)}] {sess['title'][:70]} "
              f"({len(sess['turns'])} turns)")

        for turn in sess["turns"]:
            # Baue angereicherten Content mit Kontext
            prefix = "👤 Nimar:" if turn["role"] == "user" else "🤖 Hermes:"
            content = f"[Session: {sess['title']}] {prefix} {turn['content']}"
            content = content[:8000]  # Nicht zu lang

            meta = {
                "source": "hermes_session",
                "session_id": sess["session_id"],
                "turn_role": turn["role"],
                "theme": args.theme or "general",
            }

            result = store_session(
                args.kw_url, args.api_key, content,
                source_type="human" if turn["role"] == "user" else "assistant",
                session_id=sess["session_id"],
                metadata=meta,
            )

            total_turns += 1
            if result:
                stored += 1
            else:
                errors += 1

            time.sleep(args.delay)  # Drosselung

        print(f"  ✅ {len(sess['turns'])} turns verarbeitet")

    print(f"\n🏁 Fertig: {stored}/{total_turns} turns gespeichert, {errors} Fehler")

    # ── Abschluss-Healthcheck ─────────────────────────────────
    try:
        with urllib.request.urlopen(f"{args.kw_url.rstrip('/')}/health", timeout=10) as resp:
            health = json.loads(resp.read())
            print(f"📊 KnowWhere jetzt: {health.get('node_count', '?')} Nodes")
    except Exception:
        pass


if __name__ == "__main__":
    main()

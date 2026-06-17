#!/usr/bin/env python3
"""Re-embed all KnowWhere nodes from 768d (nomic) to 1024d (Voyage code-3).

Usage: python3 migrate_voyage.py [--dry-run] [--batch-size 100]
"""

import json, os, sys, time, argparse
from pathlib import Path

STATE_FILE = os.path.expanduser("~/knowwhere/data/state.json")
BACKUP_FILE = os.path.expanduser(f"~/knowwhere/data/state.json.backup-voyage-{int(time.time())}")
VOYAGE_URL = "https://api.voyageai.com/v1/embeddings"
VOYAGE_MODEL = "voyage-code-3"
TARGET_DIM = 1024
SOURCE_DIM = 768


def get_api_key():
    """Get Voyage API key from environment or zshrc."""
    key = os.environ.get("VOYAGE_API_KEY", "")
    if key:
        return key
    # Try reading from ~/.zshrc
    zshrc = os.path.expanduser("~/.zshrc")
    if os.path.exists(zshrc):
        with open(zshrc) as f:
            for line in f:
                if "VOYAGE_API_KEY" in line and "export" in line:
                    # Extract: export VOYAGE_API_KEY="pa-..."
                    parts = line.split("=", 1)
                    if len(parts) == 2:
                        key = parts[1].strip().strip('"').strip("'")
                        return key
    return ""


def batch_embed(api_key, texts, session):
    """Call Voyage API for a batch of texts. Returns list of vectors."""
    import requests
    resp = session.post(
        VOYAGE_URL,
        headers={"Authorization": f"Bearer {api_key}"},
        json={
            "model": VOYAGE_MODEL,
            "input": texts,
            "input_type": "document",
        },
        timeout=120,
    )
    resp.raise_for_status()
    data = resp.json()
    # Sort by index to preserve order
    items = sorted(data["data"], key=lambda x: x["index"])
    return [item["embedding"] for item in items]


def main():
    parser = argparse.ArgumentParser(description="Migrate KnowWhere embeddings to Voyage 1024d")
    parser.add_argument("--dry-run", action="store_true", help="Show what would happen without writing")
    parser.add_argument("--batch-size", type=int, default=100, help="Batch size for Voyage API (max 128)")
    parser.add_argument("--limit", type=int, default=0, help="Limit to N nodes (for testing)")
    args = parser.parse_args()

    # Check API key
    api_key = get_api_key()
    if not api_key:
        print("ERROR: VOYAGE_API_KEY not found. Set it in environment or ~/.zshrc")
        sys.exit(1)
    print(f"✓ Voyage API key found ({api_key[:12]}...)")

    # Load state
    print(f"Loading {STATE_FILE}...")
    with open(STATE_FILE, "r") as f:
        data = json.load(f)
    
    nodes = data.get("nodes", {})
    total = len(nodes)
    print(f"  {total} nodes loaded")

    # Find nodes needing migration
    to_migrate = []
    for node_id, node in nodes.items():
        vec = node.get("vector", [])
        if len(vec) == SOURCE_DIM:
            content = node.get("content", "")
            if content and content.strip():
                to_migrate.append((node_id, content))
        elif len(vec) == TARGET_DIM:
            pass  # Already migrated
        elif len(vec) == 0:
            pass  # No vector
        else:
            print(f"  WARNING: Node {node_id} has unexpected dimension {len(vec)}")

    if args.limit:
        to_migrate = to_migrate[:args.limit]
    
    print(f"  {len(to_migrate)} nodes need re-embedding ({SOURCE_DIM}d → {TARGET_DIM}d)")
    
    if not to_migrate:
        print("Nothing to migrate!")
        return

    if args.dry_run:
        print(f"\nDRY RUN — would re-embed {len(to_migrate)} nodes in batches of {args.batch_size}")
        print(f"Estimated API calls: {(len(to_migrate) + args.batch_size - 1) // args.batch_size}")
        print(f"Estimated cost: ~${len(to_migrate) * 200 / 1_000_000 * 0.10:.3f} (Voyage $0.10/1M tokens)")
        return

    # Backup
    print(f"\nBacking up to {BACKUP_FILE}...")
    import shutil
    shutil.copy2(STATE_FILE, BACKUP_FILE)
    print(f"  Backup: {os.path.getsize(BACKUP_FILE) / 1024 / 1024:.0f} MB")

    # Migrate in batches
    import requests
    session = requests.Session()
    batch_size = min(args.batch_size, 128)
    migrated = 0
    failed = 0
    t_start = time.time()

    for i in range(0, len(to_migrate), batch_size):
        batch = to_migrate[i : i + batch_size]
        ids = [nid for nid, _ in batch]
        texts = [content for _, content in batch]

        try:
            vectors = batch_embed(api_key, texts, session)
            for node_id, vec in zip(ids, vectors):
                nodes[node_id]["vector"] = vec
                migrated += 1
        except Exception as e:
            print(f"  Batch {i // batch_size} FAILED: {e}")
            failed += len(batch)
            continue

        pct = (i + len(batch)) / len(to_migrate) * 100
        elapsed = time.time() - t_start
        rate = migrated / elapsed if elapsed > 0 else 0
        eta = (len(to_migrate) - migrated) / rate if rate > 0 else 0
        print(f"  {migrated}/{len(to_migrate)} ({pct:.0f}%) — {rate:.0f} nodes/s — ETA {eta:.0f}s")

    # Save
    print(f"\nSaving updated state.json...")
    with open(STATE_FILE, "w") as f:
        json.dump(data, f)
    
    elapsed = time.time() - t_start
    print(f"\n{'='*50}")
    print(f"Migration complete: {migrated} re-embedded, {failed} failed")
    print(f"Time: {elapsed:.0f}s ({elapsed/60:.1f} min)")
    print(f"Cost: ~${migrated * 200 / 1_000_000 * 0.10:.3f}")
    print(f"Backup: {BACKUP_FILE}")
    print(f"\nNext step: restart KnowWhere — it will rebuild USearch indices on startup")


if __name__ == "__main__":
    main()

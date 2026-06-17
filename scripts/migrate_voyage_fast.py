#!/usr/bin/env python3
"""Re-embed KnowWhere nodes from Ollama 768d to Voyage 1024d.

Reads node data via jq (streaming), batches to Voyage API, writes updated JSON.

Usage: python3 migrate_voyage_fast.py [--limit N] [--batch-size 100] [--dry-run]
"""

import json, os, sys, time, subprocess, argparse, shutil

STATE_FILE = os.path.expanduser("~/knowwhere/data/state.json")
BACKUP_FILE = os.path.expanduser(f"~/knowwhere/data/state.json.backup-voyage-{int(time.time())}")
TMP_FILE = STATE_FILE + ".migrating"
VOYAGE_URL = "https://api.voyageai.com/v1/embeddings"
VOYAGE_MODEL = "voyage-code-3"


def get_api_key():
    key = os.environ.get("VOYAGE_API_KEY", "")
    if key:
        return key
    zshrc = os.path.expanduser("~/.zshrc")
    if os.path.exists(zshrc):
        with open(zshrc) as f:
            for line in f:
                if "VOYAGE_API_KEY" in line and "export" in line:
                    parts = line.split("=", 1)
                    if len(parts) == 2:
                        return parts[1].strip().strip('"').strip("'")
    return ""


def extract_nodes(state_file, limit=0):
    """Use jq to stream node data: one JSON object per line."""
    jq_filter = '.nodes | to_entries[] | {id: .key, content: (.value.content // ""), vector_len: (.value.vector | length)}'
    cmd = ["jq", "-r", "-c", jq_filter, state_file]
    
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    nodes = []
    for line in proc.stdout:
        node = json.loads(line)
        vl = node.get("vector_len", 0)
        if vl == 768:  # Needs migration
            content = node.get("content", "")
            if content.strip():
                nodes.append((node["id"], content))
                if limit and len(nodes) >= limit:
                    break
    
    proc.wait()
    return nodes


def batch_embed(api_key, texts, session):
    import requests
    resp = session.post(
        VOYAGE_URL,
        headers={"Authorization": f"Bearer {api_key}"},
        json={"model": VOYAGE_MODEL, "input": texts, "input_type": "document"},
        timeout=300,
    )
    resp.raise_for_status()
    data = resp.json()
    items = sorted(data["data"], key=lambda x: x["index"])
    return [item["embedding"] for item in items]


def update_json(state_file, updates, output_file):
    """Rewrite state.json with updated vectors. updates = {node_id: [1024d vector]}"""
    import json as _json
    
    with open(state_file, "r") as f:
        data = _json.load(f)
    
    nodes = data.get("nodes", {})
    for node_id, vec in updates.items():
        if node_id in nodes:
            nodes[node_id]["vector"] = vec
    
    with open(output_file, "w") as f:
        _json.dump(data, f)
    
    return len(updates)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--batch-size", type=int, default=100)
    parser.add_argument("--limit", type=int, default=0)
    args = parser.parse_args()

    api_key = get_api_key()
    if not api_key:
        print("ERROR: VOYAGE_API_KEY not found")
        sys.exit(1)
    print(f"✓ API key: {api_key[:12]}...")

    print(f"Extracting nodes needing migration (768d → 1024d)...")
    nodes = extract_nodes(STATE_FILE, args.limit)
    total = len(nodes)
    print(f"  {total} nodes need re-embedding")

    if args.dry_run:
        batches = (total + args.batch_size - 1) // args.batch_size
        print(f"\nDRY RUN: {total} nodes, {batches} batches of ≤{args.batch_size}")
        print(f"Estimated API calls: {batches}")
        print(f"Estimated cost: ~${total * 200 / 1_000_000 * 0.10:.3f}")
        return

    if total == 0:
        print("Nothing to migrate!")
        return

    # Backup
    print(f"Backing up to {BACKUP_FILE}...")
    shutil.copy2(STATE_FILE, BACKUP_FILE)
    print(f"  Backup: {os.path.getsize(BACKUP_FILE) / 1024 / 1024:.0f} MB")

    # Batch embed
    import requests
    session = requests.Session()
    updates = {}
    t_start = time.time()
    batch_size = min(args.batch_size, 128)

    for i in range(0, total, batch_size):
        batch = nodes[i : i + batch_size]
        ids = [nid for nid, _ in batch]
        texts = [content for _, content in batch]

        try:
            vectors = batch_embed(api_key, texts, session)
            for nid, vec in zip(ids, vectors):
                updates[nid] = vec
        except Exception as e:
            print(f"  Batch {i // batch_size} FAILED: {e}")
            continue

        pct = min(100, (i + len(batch)) / total * 100)
        elapsed = time.time() - t_start
        rate = len(updates) / elapsed if elapsed > 0 else 0
        eta = (total - len(updates)) / rate if rate > 0 else 0
        print(f"  {len(updates)}/{total} ({pct:.0f}%) — {rate:.0f}/s — ETA {eta:.0f}s")

    if not updates:
        print("ERROR: No nodes were successfully re-embedded!")
        sys.exit(1)

    # Write back
    print(f"\nWriting {len(updates)} updated vectors to {TMP_FILE}...")
    updated = update_json(STATE_FILE, updates, TMP_FILE)
    print(f"  {updated} nodes updated in JSON")

    # Atomic replace
    os.replace(TMP_FILE, STATE_FILE)
    
    elapsed = time.time() - t_start
    print(f"\n{'='*50}")
    print(f"✓ Migration complete: {updated} nodes re-embedded in {elapsed:.0f}s")
    print(f"  Cost: ~${updated * 200 / 1_000_000 * 0.10:.3f}")
    print(f"  Backup: {BACKUP_FILE}")
    print(f"\nNext: restart KnowWhere — it will rebuild USearch indices")


if __name__ == "__main__":
    main()

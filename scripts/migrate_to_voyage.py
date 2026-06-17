#!/usr/bin/env python3
"""Re-embed all KnowWhere nodes from Ollama 768d to Voyage code-3 1024d.

Usage:
    python3 scripts/migrate_to_voyage.py [--dry-run] [--batch-size 128]
    
Requires VOYAGE_API_KEY in environment.
"""

import json
import os
import sys
import time
import argparse
import requests

VOYAGE_URL = "https://api.voyageai.com/v1/embeddings"
MODEL = "voyage-code-3"
BATCH_SIZE = 128
STATE_FILE = "data/state.json"
BACKUP_FILE = "data/state.json.pre-voyage.bak"


def load_nodes(path):
    """Stream-load nodes from state.json. Returns list of (node_id, text, node_obj)."""
    print(f"Loading {path} ({os.path.getsize(path)/1024/1024:.0f} MB)...")
    t0 = time.time()
    with open(path) as f:
        data = json.load(f)
    nodes_dict = data.get("nodes", {})
    elapsed = time.time() - t0
    print(f"  Loaded {len(nodes_dict)} nodes in {elapsed:.1f}s")
    return nodes_dict, data


def extract_texts(nodes_dict):
    """Extract text + pointer from nodes that need re-embedding."""
    texts = []
    node_ids = []
    dims = {}
    
    for nid, node in nodes_dict.items():
        # Check current dimension
        vec = node.get("vector", [])
        d = len(vec)
        dims[d] = dims.get(d, 0) + 1
        
        text = node.get("content") or node.get("original_pointer") or ""
        text = text.strip()
        if text:
            texts.append(text)
            node_ids.append(nid)
    
    print(f"  Dimensions found: {dims}")
    print(f"  Nodes with text: {len(texts)}")
    return texts, node_ids


def embed_batch(api_key, texts):
    """Call Voyage API for a batch of texts."""
    resp = requests.post(
        VOYAGE_URL,
        headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
        json={"model": MODEL, "input": texts},
        timeout=60,
    )
    resp.raise_for_status()
    data = resp.json()
    embeddings = [d["embedding"] for d in sorted(data["data"], key=lambda x: x["index"])]
    return embeddings


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--batch-size", type=int, default=BATCH_SIZE)
    parser.add_argument("--limit", type=int, default=0, help="Limit nodes (for testing)")
    args = parser.parse_args()

    api_key = os.environ.get("VOYAGE_API_KEY")
    if not api_key:
        print("ERROR: VOYAGE_API_KEY not set", file=sys.stderr)
        sys.exit(1)

    # Load
    nodes_dict, full_data = load_nodes(STATE_FILE)
    texts, node_ids = extract_texts(nodes_dict)
    
    if args.limit:
        texts = texts[:args.limit]
        node_ids = node_ids[:args.limit]
        print(f"  LIMITED to {args.limit} nodes for testing")
    
    if args.dry_run:
        batches = (len(texts) + args.batch_size - 1) // args.batch_size
        print(f"\nDRY RUN: Would process {len(texts)} nodes in {batches} batches")
        print(f"  Batch size: {args.batch_size}")
        print(f"  Model: {MODEL}")
        print(f"  Target dimension: 1024")
        return

    # Backup
    if not os.path.exists(BACKUP_FILE):
        print(f"\nCreating backup: {BACKUP_FILE}")
        os.system(f"cp '{STATE_FILE}' '{BACKUP_FILE}'")
    else:
        print(f"\nBackup exists, skipping: {BACKUP_FILE}")

    # Process in batches
    total = len(texts)
    updated = 0
    failed = 0
    t_start = time.time()
    
    for i in range(0, total, args.batch_size):
        batch_texts = texts[i : i + args.batch_size]
        batch_ids = node_ids[i : i + args.batch_size]
        batch_num = i // args.batch_size + 1
        
        try:
            embeddings = embed_batch(api_key, batch_texts)
            for nid, emb in zip(batch_ids, embeddings):
                if nid in nodes_dict:
                    nodes_dict[nid]["vector"] = emb
                    updated += 1
            progress = min(i + args.batch_size, total)
            elapsed = time.time() - t_start
            rate = progress / elapsed if elapsed > 0 else 0
            eta = (total - progress) / rate if rate > 0 else 0
            print(f"  Batch {batch_num}: {progress}/{total} ({progress*100/total:.1f}%) "
                  f"[{rate:.0f} nodes/s, ETA {eta:.0f}s]  "
                  f"updated={updated} failed={failed}")
        except Exception as e:
            print(f"  Batch {batch_num} FAILED: {e}", file=sys.stderr)
            failed += len(batch_ids)

    # Save
    print(f"\nSaving updated state.json...")
    t_save = time.time()
    with open(STATE_FILE, "w") as f:
        json.dump(full_data, f)  # nodes_dict references are inside full_data
    print(f"  Saved in {time.time() - t_save:.1f}s")

    total_time = time.time() - t_start
    print(f"\nDone. {updated} updated, {failed} failed in {total_time:.0f}s")
    if failed > 0:
        print(f"WARNING: {failed} nodes failed. Check logs.")


if __name__ == "__main__":
    main()

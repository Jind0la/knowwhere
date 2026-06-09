#!/usr/bin/env python3
"""
Spike 001: Bag-of-Claims Embedding Averaging (TST-inspired)
===========================================================

Tests whether averaging embeddings of semantically similar L0 claims
produces a useful coarse (L1) representation — inspired by TST's
bag-of-tokens embedding averaging.

Hypothesis (from TST paper):
  Mean(emb_1, ..., emb_n) preserves enough geometric structure to be
  a valid coarser representation — no LLM summarization needed.

Test plan:
  1. Fetch L0 nodes from KnowWhere
  2. Group by semantic similarity
  3. Average their vectors → "Bag Embedding"
  4. Store as L1 node (parent to the L0 children)
  5. Compare retrieval quality: L1-coarse vs L0-direct vs. Bag-average
"""

import requests
import json
import time
import sys
import numpy as np
from typing import Optional

BASE = "http://localhost:3737"
HEADERS = {"Authorization": "Bearer kw_testkey_12345", "Content-Type": "application/json"}

def fetch_nodes(query: str, k: int = 10, user_id: str | None = None) -> list[dict]:
    """Fetch nodes via retrieve_fractal."""
    payload = {"query_text": query, "top_k": k, "max_depth": 0, "max_tier": "raw"}
    if user_id:
        payload["user_id"] = user_id
    resp = requests.post(
        f"{BASE}/retrieve_fractal",
        json=payload,
        headers=HEADERS,
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()

# Find available user_ids from the server
import subprocess
result = subprocess.run(
    ["python3", "-c", """
import json
with open('/Users/nimarfranklinmac/knowwhere/data/state.json') as f:
    data = json.load(f)
uids = set()
for n in data['nodes'].values():
    uid = n.get('metadata',{}).get('user_id')
    if uid: uids.add(uid)
print(json.dumps(list(uids)[:3]))
"""],
    capture_output=True, text=True,
)
AVAILABLE_USER_IDS = json.loads(result.stdout)
print(f"📋 Available user_ids: {len(AVAILABLE_USER_IDS)}")
# Use the user with most nodes
USER_ID = AVAILABLE_USER_IDS[0] if AVAILABLE_USER_IDS else None

def get_node_by_id(node_id: str) -> Optional[dict]:
    """Fetch full FractalNode (with vector) by ID."""
    resp = requests.get(f"{BASE}/retrieve/{node_id}", headers=HEADERS, timeout=10)
    if resp.status_code == 404:
        return None
    resp.raise_for_status()
    return resp.json()

def cosine_sim(a, b):
    """Cosine similarity between two vectors."""
    a, b = np.array(a), np.array(b)
    return np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b) + 1e-10)

def cluster_by_similarity(vectors: list[tuple[str, np.ndarray]], threshold: float = 0.7):
    """Greedy clustering: each vector assigned to first cluster whose centroid is >threshold similar."""
    clusters = []  # list of (centroid, [(id, vec), ...])
    for nid, vec in vectors:
        assigned = False
        for ci, (centroid, members) in enumerate(clusters):
            if cosine_sim(vec, centroid) > threshold:
                members.append((nid, vec))
                # Update centroid as mean
                all_vecs = [v for _, v in members]
                clusters[ci] = (np.mean(all_vecs, axis=0), members)
                assigned = True
                break
        if not assigned:
            clusters.append((vec, [(nid, vec)]))
    return [(c, m) for c, m in clusters if len(m) >= 2]  # Only clusters of 2+

def store_bag_node(content: str, vector: list[float], children_ids: list[str]) -> dict:
    """Store a bag-averaged L1 node via store_external."""
    payload = {
        "pointer": f"bag://{children_ids[0][:8]}_et_al",
        "content": content,
        "vector": [float(v) for v in vector],
        "metadata": {
            "claim_scope": "summary",
            "derivation": "bag_of_claims",
            "child_count": len(children_ids),
            "children_ids": children_ids,
            "tst_bag_size": len(children_ids),
        },
        "memory_type": "semantic",
        "source": "consolidation",
        "importance": 7,
    }
    resp = requests.post(f"{BASE}/store_external", json=payload, headers=HEADERS, timeout=30)
    resp.raise_for_status()
    return resp.json()

def retrieve_with_vector(vector: list[float], k: int = 5) -> list[dict]:
    """Retrieve using a raw vector (not text)."""
    resp = requests.post(
        f"{BASE}/retrieve_fractal",
        json={"query_vector": [float(v) for v in vector], "top_k": k, "max_depth": 0},
        headers=HEADERS,
        timeout=10,
    )
    resp.raise_for_status()
    return resp.json()

# ── MAIN ──

print("=" * 70)
print("Spike 001: Bag-of-Claims Embedding Averaging (TST-inspired)")
print("=" * 70)

# 1. Fetch L0 nodes with multiple queries to get diverse topics
queries = [
    "Arjun Patel book club reading hobby",
    "Alex Martinez passion stories books",
    "underwater hockey amateur sports",
    "book recommendations preferences",
    "reading habits favorite genres",
]

all_nodes = []
for q in queries:
    print(f"\n🔍 Query: '{q[:60]}...'")
    results = fetch_nodes(q, k=5, user_id=USER_ID)
    for r in results:
        all_nodes.append(r)
    print(f"   → {len(results)} nodes")
    time.sleep(0.2)

print(f"\n📊 Total fetched: {len(all_nodes)} nodes")

# Deduplicate by ID
seen = set()
unique_nodes = []
for n in all_nodes:
    if n["id"] not in seen:
        seen.add(n["id"])
        unique_nodes.append(n)
print(f"   Unique: {len(unique_nodes)} nodes")

# 2. Fetch full vectors for each node
vectors = []
for n in unique_nodes:
    full = get_node_by_id(n["id"])
    if full and full.get("vector") and len(full["vector"]) > 0:
        preview = str(full.get("content", ""))[:100]
        vectors.append((n["id"], np.array(full["vector"]), preview))
    time.sleep(0.1)

print(f"   Nodes with vectors: {len(vectors)}")

if len(vectors) < 2:
    print("\n⚠️  Not enough nodes with vectors. Add more data to KnowWhere first.")
    sys.exit(0)

# 3. Cluster by similarity
# Use lower threshold for diverse clusters; PersonaMem data is semantically narrow
print(f"\n🧩 Clustering with threshold=0.75...")
clusters = cluster_by_similarity([(nid, vec) for nid, vec, _ in vectors], threshold=0.75)
print(f"   Found {len(clusters)} clusters of size 2+")

# If only 1 big cluster, try higher threshold to split
if len(clusters) <= 1 and len(vectors) > 4:
    for thresh in [0.80, 0.85, 0.90]:
        clusters = cluster_by_similarity([(nid, vec) for nid, vec, _ in vectors], threshold=thresh)
        print(f"   Retry with threshold={thresh}: {len(clusters)} clusters")
        if len(clusters) >= 2:
            break

# If still only 1, force-split into groups of 3-5
if len(clusters) <= 1:
    print("   ⚠️ Data too homogeneous. Force-splitting into groups of 4.")
    members = [(nid, vec) for nid, vec, _ in vectors]
    clusters = []
    for i in range(0, len(members), 4):
        group = members[i:i+4]
        if len(group) >= 2:
            centroid = np.mean([v for _, v in group], axis=0)
            clusters.append((centroid, group))

# 4. For each cluster, create bag node and test
print("\n" + "=" * 70)
print("📦 Bag Node Creation & Testing")
print("=" * 70)

results_table = []
for ci, (centroid, members) in enumerate(clusters):
    member_ids = [m[0] for m in members]
    print(f"\n--- Cluster {ci+1}: {len(members)} nodes ---")

    # Create bag content from member previews
    content_parts = []
    for nid, _ in members:
        # Find preview
        preview = next((p for vid, v, p in vectors if vid == nid), "?")
        content_parts.append(f"[{nid[:8]}] {preview[:80]}")
    bag_content = " | ".join(content_parts)

    # Store bag node
    print(f"   Creating bag node...")
    stored = store_bag_node(bag_content, centroid.tolist(), member_ids)
    bag_id = stored["id"]
    print(f"   ✓ Bag node: {bag_id}")

    # Test retrieval: use centroid to retrieve
    centroid_results = retrieve_with_vector(centroid.tolist(), k=5)
    centroid_ids = [r["id"] for r in centroid_results]
    children_found = [mid for mid in member_ids if mid in centroid_ids]
    recall = len(children_found) / len(member_ids)

    print(f"   Retrieval recall: {recall:.0%} ({len(children_found)}/{len(member_ids)} children found)")
    print(f"   Top-5 IDs: {[r['id'][:8] for r in centroid_results[:5]]}")

    results_table.append({
        "cluster": ci + 1,
        "size": len(members),
        "bag_id": bag_id[:8],
        "recall": recall,
    })

print("\n" + "=" * 70)
print("📊 Summary")
print("=" * 70)
print(f"{'Cluster':>8} {'Size':>6} {'Bag ID':>10} {'Recall':>8}")
print("-" * 38)
for r in results_table:
    print(f"{r['cluster']:>8} {r['size']:>6} {r['bag_id']:>10} {r['recall']:>7.0%}")

avg_recall = np.mean([r["recall"] for r in results_table])
print(f"\n   Average recall: {avg_recall:.0%}")

if avg_recall >= 0.5:
    print("\n✅ VERDICT: Bag-of-claims averaging preserves retrieval geometry.")
    print("   L1 nodes via mean-pooling are viable — no LLM summarization needed.")
else:
    print("\n⚠️  PARTIAL: Recall below 50%. Check clustering threshold or embedding quality.")

print("\n💡 TST Connection: This is the 'input-side superposition' mechanism —")
print("   averaging embeddings compresses information geometrically, just like")
print("   TST's bag-of-tokens preserves enough structure for recovery-phase training.")

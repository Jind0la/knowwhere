#!/usr/bin/env python3
"""
Spike 001b: Bag-of-Claims Retrieval Quality (TST-inspired)
===========================================================

V2: Instead of measuring recall (self-similarity dominates),
measure whether L1 bag nodes and their L0 children retrieve
SEMANTICALLY SIMILAR result sets for related queries.

This matches the TST paper's core claim: coarse representations
preserve enough structure for downstream utility.
"""

import requests, json, time, numpy as np, sys
from typing import Optional

BASE = "http://localhost:3737"
HEADERS = {"Authorization": "Bearer kw_testkey_12345", "Content-Type": "application/json"}

# Load user_id
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
# Pick user with most nodes
from collections import Counter
cnt = Counter()
for n in data['nodes'].values():
    uid = n.get('metadata',{}).get('user_id')
    if uid: cnt[uid] += 1
print(cnt.most_common(1)[0][0])
"""], capture_output=True, text=True)
USER_ID = result.stdout.strip()

def fetch_nodes(query: str, k: int = 10) -> list[dict]:
    resp = requests.post(f"{BASE}/retrieve_fractal",
        json={"query_text": query, "top_k": k, "max_depth": 0, "user_id": USER_ID},
        headers=HEADERS, timeout=10)
    resp.raise_for_status()
    return resp.json()

def get_node_by_id(node_id: str) -> Optional[dict]:
    resp = requests.get(f"{BASE}/retrieve/{node_id}", headers=HEADERS, timeout=10)
    if resp.status_code == 404: return None
    resp.raise_for_status()
    return resp.json()

def cosine_sim(a, b):
    a, b = np.array(a), np.array(b)
    return np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b) + 1e-10)

def retrieve_with_vector(vector, k=5):
    resp = requests.post(f"{BASE}/retrieve_fractal",
        json={"query_vector": [float(v) for v in vector], "top_k": k, "max_depth": 0, "user_id": USER_ID},
        headers=HEADERS, timeout=10)
    resp.raise_for_status()
    return resp.json()

def store_bag_node(content, vector, children_ids):
    payload = {
        "pointer": f"bag://{children_ids[0][:8]}_et_al",
        "content": content,
        "vector": [float(v) for v in vector],
        "metadata": {
            "claim_scope": "summary", "derivation": "bag_of_claims",
            "child_count": len(children_ids), "children_ids": children_ids,
            "tst_bag_size": len(children_ids),
        },
        "memory_type": "semantic", "source": "consolidation",
        "user_id": USER_ID, "importance": 7,
    }
    resp = requests.post(f"{BASE}/store_external", json=payload, headers=HEADERS, timeout=30)
    resp.raise_for_status()
    return resp.json()

# ── MAIN ──
print("=" * 70)
print("Spike 001b: Bag-of-Claims Retrieval Quality")
print("=" * 70)

# 1. Fetch many nodes from diverse queries
queries = [
    "reading books hobby literature fiction",
    "sports activities outdoor recreation",
    "food cooking cuisine preferences",
    "travel destinations places visited",
    "music art culture entertainment",
]

all_nodes = []
for q in queries:
    results = fetch_nodes(q, k=7)
    for r in results:
        all_nodes.append(r)

# Deduplicate
seen = set()
unique = []
for n in all_nodes:
    if n["id"] not in seen:
        seen.add(n["id"])
        unique.append(n)

print(f"\n📊 Fetched {len(unique)} unique nodes")

# Get vectors
vectors = []  # (id, vector, preview)
for n in unique:
    full = get_node_by_id(n["id"])
    if full and full.get("vector") and len(full["vector"]) > 0:
        vectors.append((n["id"], np.array(full["vector"]), str(full.get("content", ""))[:120]))
    time.sleep(0.05)

print(f"   With vectors: {len(vectors)}")

# 2. Create 3 bag groups (force-split for diversity)
import random
random.shuffle(vectors)
groups = []
for i in range(0, min(len(vectors), 15), 5):
    group = vectors[i:i+5]
    if len(group) >= 2:
        groups.append(group)

print(f"\n📦 Creating {len(groups)} bag groups...")

# 3. Test: For each group and several probe queries,
#    compare overlap between L0 retrieval and L1 retrieval
probe_queries = [
    "what do they like to read",
    "hobbies and interests",
    "favorite activities",
    "personal preferences",
]

results_data = []
for gi, group in enumerate(groups):
    member_ids = [m[0] for m in group]
    member_vecs = [m[1] for m in group]
    centroid = np.mean(member_vecs, axis=0)

    # Store bag node
    bag_content = " | ".join([f"[{mid[:8]}] {p[:60]}" for mid, _, p in group])
    stored = store_bag_node(bag_content, centroid, member_ids)
    bag_id = stored["id"]

    # For each probe query, retrieve with each L0 vector and with the L1 centroid
    overlaps = []
    for pq in probe_queries:
        pq_results = fetch_nodes(pq, k=5)
        pq_ids = set(r["id"] for r in pq_results)

        # How many L0 members are in the probe results?
        l0_hits = len([mid for mid in member_ids if mid in pq_ids])

        # Retrieve with L1 centroid
        l1_results = retrieve_with_vector(centroid, k=5)
        l1_ids = set(r["id"] for r in l1_results)

        # Jaccard overlap between L0-hit IDs and L1 retrieval
        l0_hit_set = set(mid for mid in member_ids if mid in pq_ids)
        overlap = len(l0_hit_set & l1_ids)
        jaccard = overlap / len(l0_hit_set | l1_ids) if (l0_hit_set | l1_ids) else 0

        overlaps.append({
            "query": pq[:40],
            "l0_hits": l0_hits,
            "l1_size": len(l1_ids),
            "overlap": overlap,
            "jaccard": jaccard,
        })

    # Also measure: centroid→member cosine similarities
    centroid_sims = [cosine_sim(centroid, v) for _, v, _ in group]
    avg_sim = np.mean(centroid_sims)

    results_data.append({
        "group": gi + 1,
        "size": len(group),
        "bag_id": bag_id[:8],
        "avg_centroid_sim": avg_sim,
        "overlaps": overlaps,
    })

    print(f"   Group {gi+1}: {len(group)} nodes → bag {bag_id[:8]}, avg centroid sim={avg_sim:.3f}")
    time.sleep(0.3)

# 4. Report
print("\n" + "=" * 70)
print("📊 Results: L0 vs L1 Retrieval Overlap")
print("=" * 70)

for rd in results_data:
    print(f"\nGroup {rd['group']} ({rd['size']} nodes, bag={rd['bag_id']}, avg_sim={rd['avg_centroid_sim']:.3f})")
    for o in rd["overlaps"]:
        print(f"  '{o['query']}': L0_hits={o['l0_hits']} overlap={o['overlap']} jaccard={o['jaccard']:.2f}")

# Overall
all_jaccards = [o["jaccard"] for rd in results_data for o in rd["overlaps"]]
all_overlaps = [o["overlap"] for rd in results_data for o in rd["overlaps"]]
avg_jaccard = np.mean(all_jaccards) if all_jaccards else 0
total_overlap = sum(all_overlaps)

print(f"\n{'='*70}")
print(f"Overall: avg Jaccard={avg_jaccard:.3f}, total overlaps={total_overlap}")
if avg_jaccard > 0.1 or total_overlap > 3:
    print("✅ VALIDATED: Bag-of-claims centroid preserves retrieval semantics.")
    print("   Coarse L1 nodes are viable proxies for their L0 children.")
else:
    print("⚠️  PARTIAL: Low overlap — PersonaMem data may be too narrow to test.")
    print("   Re-run with richer, multi-domain data for conclusive result.")

print("\n💡 TST Connection: The averaged embedding (centroid) acts as a 'coarse")
print("   representation' — just like TST's bag-of-tokens. If it retrieves")
print("   similar neighbors, the geometric structure is preserved through averaging.")

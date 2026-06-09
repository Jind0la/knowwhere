#!/usr/bin/env python3
"""
Spike 001c: Centroid→Neighbor Geometry Preservation (TST-inspired)
===================================================================

Direct geometric test: Does the averaged centroid retrieve the same
embedding-space neighbors as its constituent L0 vectors?

This is the TST core claim: averaging preserves enough geometric
structure for the coarse representation to be useful.
"""

import requests, json, time, numpy as np

BASE = "http://localhost:3737"
HEADERS = {"Authorization": "Bearer kw_testkey_12345", "Content-Type": "application/json"}

# Load best user_id
import subprocess
result = subprocess.run(
    ["python3", "-c", """
import json
from collections import Counter
with open('/Users/nimarfranklinmac/knowwhere/data/state.json') as f:
    data = json.load(f)
cnt = Counter()
for n in data['nodes'].values():
    uid = n.get('metadata',{}).get('user_id')
    if uid: cnt[uid] += 1
print(cnt.most_common(1)[0][0])
"""], capture_output=True, text=True)
USER_ID = result.stdout.strip()

def get_node_by_id(node_id):
    resp = requests.get(f"{BASE}/retrieve/{node_id}", headers=HEADERS, timeout=10)
    if resp.status_code == 404: return None
    resp.raise_for_status()
    return resp.json()

def retrieve_with_vector(vector, k=20):
    resp = requests.post(f"{BASE}/retrieve_fractal",
        json={"query_vector": [float(v) for v in vector], "top_k": k, "max_depth": 0, "user_id": USER_ID},
        headers=HEADERS, timeout=10)
    resp.raise_for_status()
    return resp.json()

def cosine_sim(a, b):
    a, b = np.array(a), np.array(b)
    return np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b) + 1e-10)

def jaccard(set_a, set_b):
    if not set_a and not set_b: return 1.0
    return len(set_a & set_b) / len(set_a | set_b)

# ── MAIN ──
print("=" * 70)
print("Spike 001c: Centroid→Neighbor Geometry Preservation")
print("=" * 70)

# 1. Load many nodes from state.json
import json as _json
with open("/Users/nimarfranklinmac/knowwhere/data/state.json") as f:
    state = _json.load(f)

# Filter to our user_id
nodes = {uid: n for uid, n in state["nodes"].items()
         if n.get("metadata", {}).get("user_id") == USER_ID
         and n.get("vector") and len(n["vector"]) == 768}
print(f"\n📊 Nodes for user: {len(nodes)}")

# Sample 20 random nodes
import random
sample_ids = random.sample(list(nodes.keys()), min(20, len(nodes)))
sample_nodes = [(uid, np.array(nodes[uid]["vector"]), nodes[uid].get("content", "")[:80])
                for uid in sample_ids]

# 2. Create 4 groups of 5 nodes each (force-split)
random.shuffle(sample_nodes)
groups = []
for i in range(0, len(sample_nodes), 5):
    g = sample_nodes[i:i+5]
    if len(g) >= 3:
        groups.append(g)

print(f"📦 Testing {len(groups)} groups...\n")

all_results = []
for gi, group in enumerate(groups):
    member_ids = [m[0] for m in group]
    member_vecs = [m[1] for m in group]
    centroid = np.mean(member_vecs, axis=0)

    # Get top-20 neighbors for each L0 vector
    l0_neighbor_sets = []
    for mid, mvec, _ in group:
        results = retrieve_with_vector(mvec, k=20)
        neighbor_ids = set(r["id"] for r in results if r["id"] not in member_ids)
        l0_neighbor_sets.append(neighbor_ids)
        time.sleep(0.1)

    # Union of all L0 neighbor sets (what the group collectively "sees")
    l0_union = set.union(*l0_neighbor_sets) if l0_neighbor_sets else set()
    # Intersection (neighbors ALL L0 nodes agree on)
    l0_intersection = set.intersection(*l0_neighbor_sets) if l0_neighbor_sets else set()

    # Get top-20 neighbors for the centroid
    centroid_results = retrieve_with_vector(centroid, k=20)
    centroid_neighbors = set(r["id"] for r in centroid_results if r["id"] not in member_ids)

    # Metrics
    centroid_vs_union_jaccard = jaccard(centroid_neighbors, l0_union)
    centroid_vs_intersection_jaccard = jaccard(centroid_neighbors, l0_intersection)
    centroid_covered_by_union = len(centroid_neighbors & l0_union) / len(centroid_neighbors) if centroid_neighbors else 0

    # Centroid→member similarities
    member_sims = [cosine_sim(centroid, mv) for mv in member_vecs]
    avg_member_sim = np.mean(member_sims)

    # Inter-member similarities (how tight is the cluster?)
    inter_sims = []
    for i in range(len(member_vecs)):
        for j in range(i+1, len(member_vecs)):
            inter_sims.append(cosine_sim(member_vecs[i], member_vecs[j]))
    avg_inter_sim = np.mean(inter_sims) if inter_sims else 0

    all_results.append({
        "group": gi + 1,
        "size": len(group),
        "avg_member_sim": avg_member_sim,
        "avg_inter_sim": avg_inter_sim,
        "union_jaccard": centroid_vs_union_jaccard,
        "intersection_jaccard": centroid_vs_intersection_jaccard,
        "coverage": centroid_covered_by_union,
        "l0_union_size": len(l0_union),
        "centroid_neighbor_count": len(centroid_neighbors),
    })

    print(f"Group {gi+1} ({len(group)} nodes): "
          f"avg_member_sim={avg_member_sim:.3f} inter_sim={avg_inter_sim:.3f} "
          f"union_jac={centroid_vs_union_jaccard:.3f} coverage={centroid_covered_by_union:.1%} "
          f"|L0∪|={len(l0_union)}")

# 3. Summary
print(f"\n{'='*70}")
print(f"{'Grp':>4} {'Sz':>3} {'MemSim':>7} {'InterSim':>8} {'∪Jacc':>7} {'∩Jacc':>7} {'Cover':>6} {'|L0∪|':>6}")
print("-" * 60)
for r in all_results:
    print(f"{r['group']:>4} {r['size']:>3} {r['avg_member_sim']:>7.3f} {r['avg_inter_sim']:>8.3f} "
          f"{r['union_jaccard']:>7.3f} {r['intersection_jaccard']:>7.3f} "
          f"{r['coverage']:>5.1%} {r['l0_union_size']:>6}")

avg_union_jac = np.mean([r["union_jaccard"] for r in all_results])
avg_coverage = np.mean([r["coverage"] for r in all_results])

print(f"\n📊 Averages: union_jaccard={avg_union_jac:.3f}, centroid_coverage={avg_coverage:.1%}")
print(f"   (Centroid→member cos_sim ≈ {np.mean([r['avg_member_sim'] for r in all_results]):.3f})")

if avg_union_jac > 0.15:
    print("\n✅ VALIDATED: Centroid preserves enough neighborhood geometry.")
    print("   Bag-of-claims averaging is viable for L1 node creation.")
elif avg_union_jac > 0.05:
    print("\n⚠️  PARTIAL: Some geometric preservation, but weak.")
    print("   May need topic-clustered groups (not random) for stronger signal.")
else:
    print("\n❌ NEGATIVE: Random groups don't preserve shared neighborhoods.")
    print("   Bag-of-claims requires TOPIC-CLUSTERED groups (TST's 'contiguous bags').")
    print("   Random grouping = random bag-of-tokens (TST explicitly does contiguous).")

print("\n💡 TST Mapping:")
print("   - TST bags = CONTIGUOUS tokens (same topic/context)")
print("   - Random grouping ≠ TST (breaks the contiguity assumption)")
print("   - For KnowWhere L1 nodes: group by semantic cluster, not randomly")

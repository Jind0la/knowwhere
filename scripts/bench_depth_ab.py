#!/usr/bin/env python3
"""D4-2: A/B Test — Depth-1 only vs Depth-2 Fractal Zoom.

Compares retrieve_fractal results with max_depth=1 (no zoom) vs
max_depth=2 (with zoom into cluster centroids) to quantify the
value of the second fractal zoom layer.

Uses the /retrieve_fractal endpoint with the max_depth parameter.
"""
import json
import sys
import time
import urllib.request

BASE = "http://localhost:3737"

# Test queries — mix of specific and broad questions
QUERIES = [
    "memory architecture fractal",
    "consolidation pipeline design",
    "retrieval scoring algorithm",
    "embedding dimension truncation",
    "governance policy sensitivity",
    "cross encoder reranking",
    "API authentication flow",
    "namespace search implementation",
    "skills management system",
    "turn level storage schema",
]


def retrieve(query: str, max_depth: int, top_k: int = 5) -> dict:
    """Call retrieve_fractal and return parsed response."""
    body = json.dumps({
        "query_text": query,
        "top_k": top_k,
        "max_depth": max_depth,
        "governance_enabled": False,
        "include_debug": False,
    }).encode()
    req = urllib.request.Request(
        f"{BASE}/retrieve_fractal",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    start = time.monotonic()
    with urllib.request.urlopen(req, timeout=60) as resp:
        elapsed = time.monotonic() - start
        data = json.loads(resp.read())
    return {"results": data, "elapsed_s": elapsed}


def main():
    print(f"Running D4-2 A/B Test on {len(QUERIES)} queries...")
    print()

    depth1_results = {}
    depth2_results = {}

    for i, query in enumerate(QUERIES):
        print(f"[{i+1}/{len(QUERIES)}] Query: '{query}'")

        # Depth 1 (no zoom)
        try:
            r1 = retrieve(query, max_depth=1)
            d1_ids = [n["id"] for n in r1["results"]]
            d1_scores = [n["score"] for n in r1["results"]]
            depth1_results[query] = {
                "ids": d1_ids,
                "scores": d1_scores,
                "count": len(r1["results"]),
                "elapsed_s": round(r1["elapsed_s"], 1),
            }
            print(f"  depth=1: {len(r1['results'])} results in {r1['elapsed_s']:.1f}s")
        except Exception as e:
            print(f"  depth=1: ERROR — {e}")
            depth1_results[query] = {"ids": [], "scores": [], "count": 0, "elapsed_s": 0}

        # Depth 2 (with zoom)
        try:
            r2 = retrieve(query, max_depth=2)
            d2_ids = [n["id"] for n in r2["results"]]
            d2_scores = [n["score"] for n in r2["results"]]
            depth2_results[query] = {
                "ids": d2_ids,
                "scores": d2_scores,
                "count": len(r2["results"]),
                "elapsed_s": round(r2["elapsed_s"], 1),
            }
            print(f"  depth=2: {len(r2['results'])} results in {r2['elapsed_s']:.1f}s")
        except Exception as e:
            print(f"  depth=2: ERROR — {e}")
            depth2_results[query] = {"ids": [], "scores": [], "count": 0, "elapsed_s": 0}

        # Overlap analysis
        d1_set = set(depth1_results[query]["ids"])
        d2_set = set(depth2_results[query]["ids"])
        overlap = d1_set & d2_set
        only_d1 = d1_set - d2_set
        only_d2 = d2_set - d1_set
        print(f"  overlap={len(overlap)} only_d1={len(only_d1)} only_d2={len(only_d2)}")
        print()

    # Summary
    print("=" * 72)
    print("  D4-2: Depth-1 vs Depth-2 Comparison")
    print("=" * 72)
    print(f"  Queries run: {len(QUERIES)}")
    print()

    # Aggregate stats
    avg_d1_results = sum(r["count"] for r in depth1_results.values()) / len(QUERIES)
    avg_d2_results = sum(r["count"] for r in depth2_results.values()) / len(QUERIES)
    avg_d1_time = sum(r["elapsed_s"] for r in depth1_results.values()) / len(QUERIES)
    avg_d2_time = sum(r["elapsed_s"] for r in depth2_results.values()) / len(QUERIES)

    total_overlap = 0
    total_unique_d1 = 0
    total_unique_d2 = 0
    for q in QUERIES:
        d1_set = set(depth1_results[q]["ids"])
        d2_set = set(depth2_results[q]["ids"])
        total_overlap += len(d1_set & d2_set)
        total_unique_d1 += len(d1_set - d2_set)
        total_unique_d2 += len(d2_set - d1_set)

    print(f"  {'Metric':<30} | {'Depth=1':>10} | {'Depth=2':>10}")
    print(f"  {'-'*30}-+-{'-'*10}-+-{'-'*10}")
    print(f"  {'Avg results per query':<30} | {avg_d1_results:>10.1f} | {avg_d2_results:>10.1f}")
    print(f"  {'Avg latency (s)':<30} | {avg_d1_time:>10.1f} | {avg_d2_time:>10.1f}")
    print(f"  {'Total overlapping IDs':<30} | {'':>10} | {total_overlap:>10}")
    print(f"  {'Total unique to depth':<30} | {total_unique_d1:>10} | {total_unique_d2:>10}")
    print()

    if total_unique_d2 > 0:
        pct_new = total_unique_d2 / (total_overlap + total_unique_d2) * 100
        print(f"  → Depth-2 brings {total_unique_d2} unique results ({pct_new:.0f}% new)")
        if pct_new > 10:
            print("  ✅ Depth-2 adds significant diversity to results")
        else:
            print("  ⚠️  Depth-2 adds marginal diversity — evaluate cost/benefit")
    else:
        print("  → Depth-2 brings no unique results — zoom may not add value")


if __name__ == "__main__":
    main()

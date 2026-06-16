#!/usr/bin/env python3
"""Matryoshka Dimensions Benchmark — D4-1.

Measures geometric continuity across dimension truncations for
KnowWhere's Matryoshka embedding strategy.

Uses the /nodes/recent API to fetch node vectors, then computes:
- Mean Cosine-Drift (|full_sim - trunc_sim|) per dimension
- Pearson correlation between full and truncated similarity
- Percentage of pairs with truncated similarity > 0.7

Dimensions tested: 64, 128, 256, 512 (full = 768)
"""
import json
import math
import sys
import urllib.request
from collections import defaultdict

BASE = "http://localhost:3737"
SAMPLE_SIZE = 200


def fetch_nodes(limit: int) -> list[dict]:
    """Fetch recent nodes with vectors from the API."""
    url = f"{BASE}/nodes/recent?limit={limit}"
    with urllib.request.urlopen(url, timeout=30) as resp:
        data = json.loads(resp.read())
    # Filter to nodes that actually have vectors
    return [n for n in data if n.get("vector") and len(n["vector"]) >= 512]


def cosine(a: list[float], b: list[float]) -> float:
    """Cosine similarity between two equal-length vectors."""
    dot = sum(x * y for x, y in zip(a, b))
    norm_a = math.sqrt(sum(x * x for x in a))
    norm_b = math.sqrt(sum(x * x for x in b))
    if norm_a == 0 or norm_b == 0:
        return 0.0
    return dot / (norm_a * norm_b)


def pearson(xs: list[float], ys: list[float]) -> float:
    """Pearson correlation coefficient."""
    n = len(xs)
    if n < 2:
        return 0.0
    mean_x = sum(xs) / n
    mean_y = sum(ys) / n
    cov = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys))
    std_x = math.sqrt(sum((x - mean_x) ** 2 for x in xs))
    std_y = math.sqrt(sum((y - mean_y) ** 2 for y in ys))
    if std_x == 0 or std_y == 0:
        return 0.0
    return cov / (std_x * std_y)


def benchmark(nodes: list[dict]) -> dict:
    """Run Matryoshka continuity benchmark."""
    dims = [64, 128, 256, 512]

    # Pair nodes: sequential pairs (0-1, 2-3, ...)
    pairs = []
    for i in range(0, len(nodes) - 1, 2):
        a_vec = nodes[i]["vector"]
        b_vec = nodes[i + 1]["vector"]
        # Ensure both have at least 512 dims
        min_len = min(len(a_vec), len(b_vec))
        if min_len >= 512:
            pairs.append((a_vec[:768], b_vec[:768]))
        if len(pairs) >= 100:
            break

    print(f"Running benchmark on {len(pairs)} node pairs...")
    results = {}
    for dim in dims:
        drifts = []
        full_sims = []
        trunc_sims = []
        above_07 = 0

        for a, b in pairs:
            # Full 768d cosine
            full_sim = cosine(a, b)
            full_sims.append(full_sim)

            # Truncated cosine (first `dim` components)
            trunc_sim = cosine(a[:dim], b[:dim])
            trunc_sims.append(trunc_sim)

            drift = abs(full_sim - trunc_sim)
            drifts.append(drift)

            if trunc_sim > 0.7:
                above_07 += 1

        mean_drift = sum(drifts) / len(drifts)
        corr = pearson(full_sims, trunc_sims)
        pct_above = above_07 / len(pairs) * 100

        results[dim] = {
            "mean_drift": round(mean_drift, 6),
            "pearson_r": round(corr, 4),
            "pairs_above_0.7": f"{pct_above:.0f}%",
            "pair_count": len(pairs),
        }

    return results


def main():
    print(f"Fetching {SAMPLE_SIZE} nodes from KnowWhere...")
    nodes = fetch_nodes(SAMPLE_SIZE)
    print(f"Got {len(nodes)} nodes with vectors (need ≥512 dims)")

    if len(nodes) < 20:
        print("ERROR: Not enough nodes with vectors. Need at least 20 for 10 pairs.")
        sys.exit(1)

    results = benchmark(nodes)

    print()
    print("=" * 72)
    print("  Matryoshka Geometric Continuity Benchmark (D4-1)")
    print("=" * 72)
    print(f"  Node pairs: {results[64]['pair_count']}")
    print(f"  Full dimension: 768d")
    print()
    print(f"  {'dim':>5} | {'mean_drift':>12} | {'pearson_r':>10} | {'pairs>0.7':>10}")
    print(f"  {'-'*5}-+-{'-'*12}-+-{'-'*10}-+-{'-'*10}")

    for dim in [64, 128, 256, 512]:
        r = results[dim]
        print(
            f"  {dim:>5} | {r['mean_drift']:>12.6f} | {r['pearson_r']:>10.4f} | {r['pairs_above_0.7']:>10}"
        )

    print()
    print("  Interpretation:")
    best = min(results.items(), key=lambda x: x[1]["mean_drift"])
    print(f"  → Lowest drift at {best[0]}d: {best[1]['mean_drift']:.6f}")
    print(f"  → Pearson r at 64d: {results[64]['pearson_r']:.4f} "
          f"({'strong' if abs(results[64]['pearson_r']) > 0.8 else 'moderate' if abs(results[64]['pearson_r']) > 0.5 else 'weak'} correlation)")
    drift_64 = results[64]["mean_drift"]
    if drift_64 < 0.05:
        print("  ✅ 64d drift < 0.05 — Matryoshka truncation is well-behaved")
    elif drift_64 < 0.15:
        print("  ⚠️  64d drift 0.05-0.15 — acceptable but monitor")
    else:
        print("  ❌ 64d drift > 0.15 — truncation loses significant precision")


if __name__ == "__main__":
    main()

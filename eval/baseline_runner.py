#!/usr/bin/env python3
"""
KnowWhere Baseline Runner — Phase 1 (Temporal Boost Evaluation)
Reads golden queries from queries/temporal_golden.json and runs them
with and without recency_boost to measure temporal improvement.

Usage:
    python3 eval/baseline_runner.py --boost 0.20 --top_k 5
    python3 eval/baseline_runner.py --no-boost --top_k 5
    python3 eval/baseline_runner.py --compare              # run both + delta
"""

import argparse
import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import URLError

KNOWWHERE_URL = os.environ.get("KNOWWHERE_URL", "http://localhost:3737")
API_KEY = os.environ.get("KNOWWHERE_API_KEY", "kw_testkey_12345")


def retrieve(query: str, top_k: int = 5, boost: float | None = None) -> tuple[list, float]:
    """Call retrieve_fractal. Returns (results, latency_ms)."""
    payload = json.dumps({
        "query_text": query,
        "top_k": top_k,
        **({"recency_boost": boost} if boost is not None else {}),
    }).encode("utf-8")

    req = Request(
        f"{KNOWWHERE_URL}/retrieve_fractal",
        data=payload,
        headers={
            "Authorization": f"Bearer {API_KEY}",
            "Content-Type": "application/json",
        },
    )

    start = time.monotonic()
    try:
        with urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read())
            latency_ms = (time.monotonic() - start) * 1000.0
            return data, latency_ms
    except URLError as e:
        raise RuntimeError(f"HTTP error: {e}") from e


def load_golden_queries(path: str) -> list[dict]:
    """Load the 15 golden temporal queries from JSON file."""
    with open(path) as f:
        data = json.load(f)
    return data["queries"]


def run_eval(queries: list[dict], top_k: int, boost: float | None, label: str) -> dict:
    """Run all queries with given boost setting."""
    results = []
    total_latency = 0.0
    errors = 0

    for i, q in enumerate(queries, 1):
        try:
            res, latency = retrieve(q["query"], top_k=top_k, boost=boost)
            top_score = res[0]["score"] if res else 0.0
            results.append({
                "id": q["id"],
                "query": q["query"],
                "type": q["type"],
                "top_score": top_score,
                "num_results": len(res),
                "latency_ms": round(latency, 1),
            })
            total_latency += latency
            print(f"[{i:02d}] {q['id']} {q['query'][:65]}... → score={top_score:.4f}  {latency:.0f}ms")
        except Exception as e:
            errors += 1
            print(f"[{i:02d}] {q['id']} ERROR: {e}")

    n = len(results)
    avg_score = sum(r["top_score"] for r in results) / n if n else 0
    avg_latency = total_latency / n if n else 0

    return {
        "label": label,
        "boost": boost,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "queries_run": n,
        "errors": errors,
        "avg_top_score": round(avg_score, 4),
        "avg_latency_ms": round(avg_latency, 1),
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--queries", default="queries/temporal_golden.json")
    parser.add_argument("--top_k", type=int, default=5)
    parser.add_argument("--boost", type=float, default=None,
                        help="Recency boost factor (e.g. 0.20)")
    parser.add_argument("--no-boost", action="store_true",
                        help="Run with boost disabled (None)")
    parser.add_argument("--compare", action="store_true",
                        help="Run both with and without boost, print delta")
    parser.add_argument("--output", default=None,
                        help="Save results to JSON file")
    args = parser.parse_args()

    queries = load_golden_queries(args.queries)
    print(f"Loaded {len(queries)} golden queries from {args.queries}")
    print(f"Server: {KNOWWHERE_URL}\n")

    if args.compare:
        print("=== BASELINE (no boost) ===\n")
        baseline = run_eval(queries, args.top_k, None, "baseline_no_boost")
        print(f"\n=== BOOSTED (0.20) ===\n")
        boosted = run_eval(queries, args.top_k, 0.20, "boosted_0.20")
        delta_score = round(boosted["avg_top_score"] - baseline["avg_top_score"], 4)
        delta_lat = round(boosted["avg_latency_ms"] - baseline["avg_latency_ms"], 1)
        print(f"\n=== DELTA ===")
        print(f"  Baseline: avg_score={baseline['avg_top_score']:.4f}  avg_latency={baseline['avg_latency_ms']:.1f}ms")
        print(f"  Boosted:  avg_score={boosted['avg_top_score']:.4f}  avg_latency={boosted['avg_latency_ms']:.1f}ms")
        print(f"  Δ score:  {delta_score:+.4f}")
        print(f"  Δ latency: {delta_lat:+.1f}ms")

        combined = {"baseline": baseline, "boosted": boosted, "delta": {
            "score": delta_score,
            "latency_ms": delta_lat,
        }}
    else:
        boost = None if args.no_boost else (args.boost if args.boost is not None else 0.20)
        label = f"boost_{boost}" if boost is not None else "no_boost"
        combined = run_eval(queries, args.top_k, boost, label)

    if args.output:
        Path(args.output).parent.mkdir(parents=True, exist_ok=True)
        with open(args.output, "w") as f:
            json.dump(combined, f, indent=2)
        print(f"\nSaved to {args.output}")


if __name__ == "__main__":
    main()

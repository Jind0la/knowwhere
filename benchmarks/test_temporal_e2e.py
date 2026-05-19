#!/usr/bin/env python3
"""
End-to-end Temporal Test Runner for KnowWhere.

Runs the temporal-reasoning and knowledge-update test fixtures against
the KnowWhere API with multiple temporal_weight configurations.

Reports NDCG@5 and Top-1 for each weight variant.

Usage:
    python test_temporal_e2e.py [--base-url URL] [--api-key KEY]
    python test_temporal_e2e.py --fixture temporal_reasoning --weights 0.15,0.25,0.35
"""

import argparse
import asyncio
import json
import math
import os
import sys
import time
from pathlib import Path
from typing import Optional

import aiohttp

# Reuse constants from existing eval
K_VALUES = [1, 3, 5, 10, 30, 50]
QUESTION_TYPES = [
    "single-session-user", "single-session-assistant",
    "single-session-preference", "multi-session",
    "temporal-reasoning", "knowledge-update",
]

FIXTURE_DIR = Path(__file__).parent / "hf" / "fixtures"
FIXTURES = {
    "temporal_reasoning": FIXTURE_DIR / "longmemeval_temporal_reasoning.json",
    "knowledge_update": FIXTURE_DIR / "longmemeval_knowledge_update.json",
    "halflife": FIXTURE_DIR / "longmemeval_halflife.json",
    "tiny": FIXTURE_DIR / "longmemeval_retrieval_tiny.json",
}


class EvalCase:
    def __init__(self, raw: dict):
        self.question_id = raw["question_id"]
        self.question = raw["question"]
        self.question_type = raw.get("question_type", "")
        self.question_date = raw.get("question_date", "")
        self.answer = raw.get("answer", "")
        self.answer_session_ids = [str(x) for x in raw.get("answer_session_ids", [])]
        self.haystack_session_ids = [str(x) for x in raw.get("haystack_session_ids", [])]
        self.haystack_dates = [str(x) for x in raw.get("haystack_dates", [])]
        self.haystack_sessions = raw.get("haystack_sessions", [])

    @property
    def is_abstention(self):
        return self.question_id.endswith("_abs") or not self.answer_session_ids

    def session_text(self, idx: int) -> str:
        session = self.haystack_sessions[idx] if idx < len(self.haystack_sessions) else []
        lines = []
        for turn in session:
            role = turn.get("role", "unknown")
            content = turn.get("content", "")
            lines.append(f"{role}: {content}")
        return "\n".join(lines) if lines else str(session)


class KnowWhereClient:
    def __init__(self, base_url: str, api_key: str):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key

    def _headers(self) -> dict:
        return {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
        }

    async def store_session_batch(self, session: aiohttp.ClientSession, run_id: str, case: EvalCase) -> list[str]:
        sessions_payload = []
        for idx, sid in enumerate(case.haystack_session_ids):
            session_date = case.haystack_dates[idx] if idx < len(case.haystack_dates) else ""
            content = case.session_text(idx)
            sessions_payload.append({
                "content": content,
                "metadata": {
                    "benchmark": "temporal_e2e",
                    "run_id": run_id,
                    "question_id": case.question_id,
                    "question_type": case.question_type,
                    "question_date": case.question_date,
                    "session_id": sid,
                    "benchmark_session_date": session_date,
                    "source_timestamp": session_date,
                },
                "memory_type": "episodic",
                "source": "conversation",
            })

        url = f"{self.base_url}/store_session_batch"
        payload = {"sessions": sessions_payload}
        async with session.post(url, headers=self._headers(), json=payload) as resp:
            if resp.status not in (200, 201):
                body = await resp.text()
                raise RuntimeError(f"store_session_batch failed {resp.status}: {body[:300]}")
            data = await resp.json()
            results = data.get("results", [])
            ids = []
            for entry in results:
                primary = entry.get("id")
                if primary:
                    ids.append(primary)
                for cid in entry.get("chunk_ids", []):
                    if cid != primary:
                        ids.append(cid)
            return ids

    async def retrieve(self, session: aiohttp.ClientSession, query: str, top_k: int = 80, temporal_weight: float = 0.0) -> list[dict]:
        url = f"{self.base_url}/retrieve_fractal"
        payload = {
            "query_text": query,
            "top_k": top_k,
            "max_depth": 3,
            "governance_enabled": True,
            "retrieval_profile": "full-fidelity",
            "include_debug": False,
        }
        if temporal_weight > 0.0:
            payload["temporal_weight"] = temporal_weight
        async with session.post(url, headers=self._headers(), json=payload) as resp:
            if resp.status != 200:
                body = await resp.text()
                raise RuntimeError(f"retrieve failed {resp.status}: {body[:300]}")
            return await resp.json()

    async def batch_delete(self, session: aiohttp.ClientSession, ids: list[str]) -> None:
        if not ids:
            return
        url = f"{self.base_url}/nodes/batch_delete"
        async with session.post(url, headers=self._headers(), json={"ids": ids}) as resp:
            if resp.status not in (200, 204):
                pass


def extract_session_ids(hits: list[dict]) -> list[str]:
    seen = set()
    result = []
    for hit in hits:
        sid = hit.get("metadata", {}).get("session_id", "")
        if sid and sid not in seen:
            seen.add(sid)
            result.append(sid)
    return result


def dcg(relevances: list[float], k: int) -> float:
    rels = relevances[:k]
    if not rels:
        return 0.0
    return rels[0] + sum(rels[i] / math.log2(i + 2) for i in range(1, len(rels)))


def ndcg_at_k(rankings: list[int], correct_ids: set[str], all_ids: list[str], k: int) -> float:
    relevances = [1.0 if all_ids[i] in correct_ids else 0.0 for i in range(len(all_ids))]
    sorted_rels = sorted(relevances, reverse=True)
    ideal = dcg(sorted_rels, k)
    if ideal == 0:
        return 0.0
    actual = dcg([relevances[i] for i in rankings[:k]], k)
    return actual / ideal


def compute_metrics(rankings, all_ids, correct_ids, k_values):
    """Compute NDCG@k and recall at each k."""
    results = {}
    for k in k_values:
        recalled = {all_ids[i] for i in rankings[:k]}
        recall_any = 1.0 if recalled & correct_ids else 0.0
        results[k] = {
            "recall_any": recall_any,
            "ndcg_any": ndcg_at_k(rankings, correct_ids, all_ids, k),
        }
    return results


async def run_single_weight(
    client: KnowWhereClient,
    cases: list[EvalCase],
    temporal_weight: float,
) -> dict:
    """Run evaluation with a specific temporal_weight. Returns per-case results."""
    results = []
    total = len(cases)

    async with aiohttp.ClientSession() as http:
        for idx, case in enumerate(cases):
            run_id = f"temp-e2e-{temporal_weight}-{idx}-{case.question_id}"
            stored_ids = []
            try:
                stored_ids = await client.store_session_batch(http, run_id, case)
                hits = await client.retrieve(http, case.question, top_k=80, temporal_weight=temporal_weight)

                # Filter to owned hits
                owned = [h for h in hits if h.get("metadata", {}).get("run_id") == run_id]
                hit_sids = extract_session_ids(owned)

                answer_sids = set(case.answer_session_ids)
                rankings = list(range(len(hit_sids)))

                # Top-1
                rank = None
                for i, sid in enumerate(hit_sids):
                    if sid in answer_sids:
                        rank = i + 1
                        break
                top1 = 1.0 if rank == 1 else 0.0

                # NDCG@k
                metrics = compute_metrics(rankings, hit_sids, answer_sids, K_VALUES)

                results.append({
                    "question_id": case.question_id,
                    "question_type": case.question_type,
                    "rank": rank,
                    "top1": top1,
                    "temporal_weight": temporal_weight,
                    "hit_count": len(owned),
                    "metrics": metrics,
                    "error": None,
                })

                if stored_ids:
                    await client.batch_delete(http, stored_ids)

            except Exception as e:
                results.append({
                    "question_id": case.question_id,
                    "question_type": case.question_type,
                    "rank": None,
                    "top1": 0.0,
                    "temporal_weight": temporal_weight,
                    "hit_count": 0,
                    "metrics": {},
                    "error": str(e),
                })

            status = "\u2713" if not results[-1].get("error") else "\u2717"
            rank_str = f"rank={rank}" if rank else "no hit"
            print(f"  [{idx+1:>3}/{total}] {status} w={temporal_weight:.2f} {case.question_id} {rank_str}",
                  flush=True)

    return results


def aggregate_weight_results(all_results: dict[float, list[dict]]) -> dict:
    """Aggregate results across all temporal weights into a comparison table."""
    summary = {}
    for w, results in all_results.items():
        non_err = [r for r in results if not r.get("error")]
        n = max(len(non_err), 1)

        top1 = sum(r["top1"] for r in non_err) / n
        ndcg5 = sum(r.get("metrics", {}).get(5, {}).get("ndcg_any", 0.0) for r in non_err) / n
        recall5 = sum(r.get("metrics", {}).get(5, {}).get("recall_any", 0.0) for r in non_err) / n

        # Per-type breakdown
        per_type = {}
        for qtype in QUESTION_TYPES:
            type_cases = [r for r in non_err if r.get("question_type") == qtype]
            tn = max(len(type_cases), 1)
            if type_cases:
                per_type[qtype] = {
                    "count": len(type_cases),
                    "top1": sum(r["top1"] for r in type_cases) / tn,
                    "ndcg@5": sum(r.get("metrics", {}).get(5, {}).get("ndcg_any", 0.0) for r in type_cases) / tn,
                }

        summary[w] = {
            "cases": n,
            "top1": top1,
            "ndcg@5": ndcg5,
            "recall@5": recall5,
            "per_type": per_type,
            "errors": len(results) - n,
        }

    return summary


def print_comparison(summary: dict):
    """Print a comparison table across temporal weights."""
    print("\n" + "=" * 90)
    print("  Temporal Weight Comparison — NDCG@5 & Top-1")
    print("=" * 90)

    weights = sorted(summary.keys())
    print(f"\n  {'Weight':>8}  {'Cases':>6}  {'Top-1':>8}  {'NDCG@5':>10}  {'Recall@5':>10}  {'Errors':>6}")
    print("  " + "-" * 70)
    for w in weights:
        s = summary[w]
        print(f"  {w:>8.2f}  {s['cases']:>6}  {s['top1']:>8.4f}  {s['ndcg@5']:>10.4f}  {s['recall@5']:>10.4f}  {s['errors']:>6}")

    # Per-type breakdown
    print("\n  ── Per-Type Breakdown ──")
    for qtype in QUESTION_TYPES:
        print(f"\n  {qtype}:")
        header = f"    {'Weight':>8}  {'Count':>6}  {'Top-1':>8}  {'NDCG@5':>10}"
        print(header)
        print("    " + "-" * 45)
        for w in weights:
            pt = summary[w].get("per_type", {}).get(qtype)
            if pt and pt["count"] > 0:
                print(f"    {w:>8.2f}  {pt['count']:>6}  {pt['top1']:>8.4f}  {pt['ndcg@5']:>10.4f}")

    # Best weight recommendation
    best_w = max(weights, key=lambda w: summary[w]["ndcg@5"])
    print(f"\n  ★ Best NDCG@5: w={best_w:.2f} (NDCG@5={summary[best_w]['ndcg@5']:.4f}, Top-1={summary[best_w]['top1']:.4f})")
    print("=" * 90)


async def main():
    parser = argparse.ArgumentParser(description="Temporal E2E Test Runner")
    parser.add_argument("--base-url", default=os.environ.get("KNOWWHERE_BASE_URL", "http://127.0.0.1:3737"))
    parser.add_argument("--api-key", default=os.environ.get("KNOWWHERE_API_KEY", ""))
    parser.add_argument("--fixture", choices=list(FIXTURES.keys()), default="temporal_reasoning",
                        help="Which fixture to run")
    parser.add_argument("--weights", default="0.0,0.15,0.25,0.35,0.5",
                        help="Comma-separated temporal weights to test")
    parser.add_argument("--output", default="", help="Output JSON report path (default: auto)")
    args = parser.parse_args()

    if not args.api_key:
        print("ERROR: KNOWWHERE_API_KEY is required (set env or pass --api-key)")
        sys.exit(1)

    # Load fixture
    fixture_path = FIXTURES[args.fixture]
    if not fixture_path.exists():
        print(f"ERROR: fixture not found: {fixture_path}")
        sys.exit(1)

    with open(fixture_path) as f:
        raw_cases = json.load(f)
    cases = [EvalCase(c) for c in raw_cases]
    non_abs = [c for c in cases if not c.is_abstention]

    print(f"Fixture: {args.fixture} ({fixture_path})")
    print(f"Cases: {len(cases)} total, {len(non_abs)} non-abstention")
    print(f"API: {args.base_url}")

    weights = [float(w.strip()) for w in args.weights.split(",")]
    print(f"Weights: {weights}\n")

    client = KnowWhereClient(args.base_url, args.api_key)

    # Run each weight
    t0 = time.monotonic()
    all_results = {}
    for w in weights:
        print(f"\n── Temporal weight = {w:.2f} ──")
        results = await run_single_weight(client, non_abs, w)
        all_results[w] = results

    elapsed = time.monotonic() - t0

    # Aggregate and print
    summary = aggregate_weight_results(all_results)
    print_comparison(summary)
    print(f"\n  Total time: {elapsed:.1f}s")

    # Save report
    output = args.output
    if not output:
        ts = time.strftime("%Y%m%d_%H%M%S")
        output = f"temporal_e2e_{args.fixture}_{ts}.json"
    report = {
        "fixture": args.fixture,
        "weights": weights,
        "summary": {str(w): v for w, v in summary.items()},
        "elapsed_s": elapsed,
    }
    os.makedirs(os.path.dirname(output) or ".", exist_ok=True)
    with open(output, "w") as f:
        json.dump(report, f, indent=2, default=str)
    print(f"  Report saved to: {output}")


if __name__ == "__main__":
    asyncio.run(main())

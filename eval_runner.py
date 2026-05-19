#!/usr/bin/env python3
"""
KnowWhere Turn-Level vs Session-Level Comparative Evaluation

Runs a TDD query suite targeting semantic drift and fact burial failures,
comparing session-level retrieval (retrieve_fractal) against turn-level
retrieval (retrieve/turns). Computes NDCG@5, Recall@1, Top-1 Rate,
and per-pattern breakdowns.

Usage:
    # Full comparative run (requires both endpoints)
    python3 eval_runner.py --queries queries/semantic_drift_queries.json

    # Session-level only (works without turn-level backend)
    python3 eval_runner.py --queries queries/semantic_drift_queries.json --session-only

    # Simulated mode (uses gold documents for offline validation)
    python3 eval_runner.py --queries queries/semantic_drift_queries.json --simulate

    # Custom parameters
    python3 eval_runner.py --top-k 5 --url http://localhost:3737
"""

import argparse
import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import URLError, HTTPError


# ── Configuration ──────────────────────────────────────────────────────────
KNOWWHERE_URL = os.environ.get("KNOWWHERE_URL", "http://localhost:3737")
API_KEY = os.environ.get("KNOWWHERE_API_KEY", "kw_testkey_12345")
K_VALUES = [1, 3, 5]


# ── Metrics (pure functions) ────────────────────────────────────────────────

def dcg(relevances, k):
    """Discounted Cumulative Gain at k."""
    rels = relevances[:k]
    if not rels:
        return 0.0
    return rels[0] + sum(rels[i] / (i + 2.0) for i in range(1, len(rels)))


def ndcg_at_k(ranked_doc_ids, gold_doc_ids, k):
    """Normalized DCG at k with binary relevance."""
    if not gold_doc_ids:
        return 0.0
    gold_set = set(gold_doc_ids)
    relevances = [1.0 if did in gold_set else 0.0 for did in ranked_doc_ids]
    ideal_rel = sorted([1.0] * min(len(gold_doc_ids), k) + [0.0] * max(0, k - len(gold_doc_ids)), reverse=True)
    ideal = dcg(ideal_rel, k)
    if ideal == 0:
        return 0.0
    return dcg(relevances, k) / ideal


def recall_at_k(ranked_doc_ids, gold_doc_ids, k):
    """Recall@k: fraction of gold docs found in top-k."""
    if not gold_doc_ids:
        return 1.0
    gold_set = set(gold_doc_ids)
    found = sum(1 for did in ranked_doc_ids[:k] if did in gold_set)
    return found / len(gold_doc_ids)


def recall_at_1(ranked_doc_ids, gold_doc_ids):
    """Recall@1: was the first result relevant?"""
    if not gold_doc_ids:
        return 1.0
    if not ranked_doc_ids:
        return 0.0
    return 1.0 if ranked_doc_ids[0] in set(gold_doc_ids) else 0.0


def top1_accuracy(ranked_doc_ids, gold_doc_ids):
    """Top-1 Accuracy: identical to Recall@1."""
    return recall_at_1(ranked_doc_ids, gold_doc_ids)


def mrr(ranked_doc_ids, gold_doc_ids):
    """Mean Reciprocal Rank."""
    if not gold_doc_ids:
        return 1.0
    gold_set = set(gold_doc_ids)
    for i, did in enumerate(ranked_doc_ids):
        if did in gold_set:
            return 1.0 / (i + 1)
    return 0.0


def compute_all_metrics(ranked_doc_ids, gold_doc_ids, k=5):
    """Compute all metrics for a single query."""
    return {
        f"ndcg@{k}": ndcg_at_k(ranked_doc_ids, gold_doc_ids, k),
        "ndcg@1": ndcg_at_k(ranked_doc_ids, gold_doc_ids, 1),
        "recall@1": recall_at_1(ranked_doc_ids, gold_doc_ids),
        f"recall@{k}": recall_at_k(ranked_doc_ids, gold_doc_ids, k),
        "top1_accuracy": top1_accuracy(ranked_doc_ids, gold_doc_ids),
        "mrr": mrr(ranked_doc_ids, gold_doc_ids),
        "gold_count": len(gold_doc_ids),
        "retrieved_count": len(ranked_doc_ids),
    }


# ── KnowWhere API Client ────────────────────────────────────────────────────

def api_post(endpoint, data, url=KNOWWHERE_URL, api_key=API_KEY, timeout=30):
    """POST to KnowWhere API. Returns (response_json, latency_ms)."""
    payload = json.dumps(data).encode("utf-8")
    req = Request(
        f"{url}{endpoint}",
        data=payload,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    start = time.monotonic()
    with urlopen(req, timeout=timeout) as resp:
        result = json.loads(resp.read())
        latency_ms = (time.monotonic() - start) * 1000.0
        return result, latency_ms


def retrieve_session_level(query_text, top_k=5, url=KNOWWHERE_URL, api_key=API_KEY):
    """Session-level retrieval via /retrieve_fractal."""
    data = {"query_text": query_text, "top_k": top_k}
    results, latency = api_post("/retrieve_fractal", data, url, api_key)
    return results, latency


def retrieve_turn_level(query_text, limit=5, url=KNOWWHERE_URL, api_key=API_KEY):
    """Turn-level retrieval via /retrieve/turns."""
    data = {"query_text": query_text, "limit": limit}
    results, latency = api_post("/retrieve/turns", data, url, api_key)
    return results, latency


def store_document(content, metadata=None, url=KNOWWHERE_URL, api_key=API_KEY):
    """Store a document in KnowWhere via /store_session."""
    data = {
        "content": content,
        "source": "eval_gold",
        "memory_type": "episodic",
        "metadata": metadata or {},
    }
    result, _ = api_post("/store_session", data, url, api_key)
    return result.get("id") if isinstance(result, dict) else None


def extract_doc_ids_from_fractal(results):
    """Extract document IDs from retrieve_fractal response."""
    if isinstance(results, list):
        return [str(item.get("id", "")) for item in results if item.get("id")]
    if isinstance(results, dict):
        items = results.get("results", [])
        return [str(item.get("id", "")) for item in items if item.get("id")]
    return []


def extract_doc_ids_from_turns(results):
    """Extract turn session_ids from retrieve/turns response."""
    if isinstance(results, list):
        ids = []
        for item in results:
            if isinstance(item, dict):
                sid = item.get("session_id") or item.get("external_session_id")
                if sid:
                    ids.append(str(sid))
        return ids
    if isinstance(results, dict):
        items = results.get("results", results.get("turns", []))
        ids = []
        for item in items:
            if isinstance(item, dict):
                sid = item.get("session_id") or item.get("external_session_id")
                if sid:
                    ids.append(str(sid))
        return ids
    return []


# ── Simulated Retrieval ────────────────────────────────────────────────────

def simulate_session_retrieval(query, all_docs, top_k=5):
    """Simulate session-level retrieval by ranking docs by content similarity
    to the query. A placeholder simulation — in reality this would be the
    embedding model's actual ranking."""
    query_lower = query.lower()
    scored = []
    for doc in all_docs:
        content_lower = doc["content"].lower()
        # Simple BM25-like: count word overlap
        score = sum(1 for word in query_lower.split() if word in content_lower)
        scored.append((doc["doc_id"], score))
    scored.sort(key=lambda x: -x[1])
    return [did for did, _ in scored[:top_k]]


def simulate_turn_retrieval(query, all_docs, top_k=5):
    """Simulate turn-level retrieval — identical to session for the
    purpose of testing the evaluation harness. Turn-level would perform
    better in reality due to finer embedding granularity."""
    return simulate_session_retrieval(query, all_docs, top_k)


# ── Evaluation Runner ───────────────────────────────────────────────────────

def run_evaluation(queries, gold_docs, top_k=5, simulate=False,
                   session_only=False, url=KNOWWHERE_URL, api_key=API_KEY,
                   verbose=True):
    """Run evaluation over all queries, returning per-query and aggregate results."""
    # Build doc lookup
    doc_by_id = {d["doc_id"]: d for d in gold_docs}

    per_query = []
    api_calls = 0
    total_latency_session = 0.0
    total_latency_turn = 0.0
    turn_errors = 0

    for i, q in enumerate(queries, 1):
        qid = q["id"]
        query_text = q["query"]
        qtype = q["type"]
        pattern = q.get("failure_pattern", "unknown")
        gold_ids = q.get("gold_documents", [])
        hypothesis = q.get("hypothesis", "")

        result = {
            "id": qid,
            "query": query_text[:100],
            "type": qtype,
            "failure_pattern": pattern,
            "gold_documents": gold_ids,
            "hypothesis": hypothesis,
            "session": None,
            "turn": None,
        }

        # ── Session-level retrieval ──
        try:
            if simulate:
                session_docs = simulate_session_retrieval(query_text, gold_docs, top_k)
                session_latency = 0.0
            else:
                raw_results, session_latency = retrieve_session_level(
                    query_text, top_k, url, api_key
                )
                session_docs = extract_doc_ids_from_fractal(raw_results)
                api_calls += 1
                total_latency_session += session_latency

            session_metrics = compute_all_metrics(session_docs, gold_ids, top_k)
            session_metrics["retrieved_docs"] = session_docs[:top_k]
            session_metrics["latency_ms"] = round(session_latency, 1)
            result["session"] = session_metrics

            if verbose:
                ndcg = session_metrics[f"ndcg@{top_k}"]
                r1 = session_metrics["recall@1"]
                t1 = session_metrics["top1_accuracy"]
                marker = "✓" if ndcg > 0 else "✗"
                print(f"  [{i:02d}] {qid:14s} SESSION {marker} "
                      f"NDCG@{top_k}={ndcg:.3f}  R@1={r1:.0f}  Top1={t1:.0f}  "
                      f"({len(session_docs[:top_k])} docs)")

        except Exception as e:
            result["session"] = {
                "error": str(e),
                f"ndcg@{top_k}": 0.0,
                "ndcg@1": 0.0,
                "recall@1": 0.0,
                f"recall@{top_k}": 0.0,
                "top1_accuracy": 0.0,
                "mrr": 0.0,
            }
            if verbose:
                print(f"  [{i:02d}] {qid:14s} SESSION ✗ ERROR: {e}")

        # ── Turn-level retrieval ──
        if not session_only:
            try:
                if simulate:
                    turn_docs_ids = simulate_turn_retrieval(query_text, gold_docs, top_k)
                    turn_latency = 0.0
                else:
                    raw_results, turn_latency = retrieve_turn_level(
                        query_text, top_k, url, api_key
                    )
                    turn_docs_ids = extract_doc_ids_from_turns(raw_results)
                    api_calls += 1
                    total_latency_turn += turn_latency

                turn_metrics = compute_all_metrics(turn_docs_ids, gold_ids, top_k)
                turn_metrics["retrieved_ids"] = turn_docs_ids[:top_k]
                turn_metrics["latency_ms"] = round(turn_latency, 1)
                result["turn"] = turn_metrics

                if verbose:
                    ndcg = turn_metrics[f"ndcg@{top_k}"]
                    r1 = turn_metrics["recall@1"]
                    t1 = turn_metrics["top1_accuracy"]
                    delta = ndcg - result["session"].get(f"ndcg@{top_k}", 0)
                    marker = "✓" if delta > 0 else ("=" if delta == 0 else "✗")
                    print(f"  [{i:02d}] {qid:14s} TURN    {marker} "
                          f"NDCG@{top_k}={ndcg:.3f}  R@1={r1:.0f}  Top1={t1:.0f}  "
                          f"(Δ={delta:+.3f})")

            except Exception as e:
                turn_errors += 1
                result["turn"] = {
                    "error": str(e),
                    f"ndcg@{top_k}": 0.0,
                    "ndcg@1": 0.0,
                    "recall@1": 0.0,
                    f"recall@{top_k}": 0.0,
                    "top1_accuracy": 0.0,
                    "mrr": 0.0,
                }
                if verbose:
                    print(f"  [{i:02d}] {qid:14s} TURN    ✗ UNAVAILABLE: {e}")

        per_query.append(result)

    # ── Aggregate ──
    session_valid = [r for r in per_query if r["session"] and "error" not in r["session"]]
    turn_valid = [r for r in per_query if r["turn"] and "error" not in r["turn"]]

    def aggregate(valid_results, key, top_k):
        if not valid_results:
            return {}
        n = len(valid_results)
        return {
            f"avg_ndcg@{top_k}": sum(r[key][f"ndcg@{top_k}"] for r in valid_results) / n,
            "avg_ndcg@1": sum(r[key]["ndcg@1"] for r in valid_results) / n,
            "avg_recall@1": sum(r[key]["recall@1"] for r in valid_results) / n,
            f"avg_recall@{top_k}": sum(r[key][f"recall@{top_k}"] for r in valid_results) / n,
            "avg_top1_accuracy": sum(r[key]["top1_accuracy"] for r in valid_results) / n,
            "avg_mrr": sum(r[key]["mrr"] for r in valid_results) / n,
            "num_queries": n,
        }

    # Per-pattern breakdown
    def pattern_breakdown(valid_results, key, top_k):
        patterns = {}
        for r in valid_results:
            pat = r.get("failure_pattern", "unknown")
            if pat not in patterns:
                patterns[pat] = []
            patterns[pat].append(r)
        result = {}
        for pat, cases in patterns.items():
            n = len(cases)
            result[pat] = {
                "count": n,
                f"avg_ndcg@{top_k}": sum(c[key][f"ndcg@{top_k}"] for c in cases) / n,
                "avg_recall@1": sum(c[key]["recall@1"] for c in cases) / n,
                "avg_top1_accuracy": sum(c[key]["top1_accuracy"] for c in cases) / n,
            }
        return result

    session_agg = aggregate(session_valid, "session", top_k)
    turn_agg = aggregate(turn_valid, "turn", top_k)
    session_by_pattern = pattern_breakdown(session_valid, "session", top_k)
    turn_by_pattern = pattern_breakdown(turn_valid, "turn", top_k)

    # Compute deltas
    deltas = {}
    if session_agg and turn_agg:
        for metric in [f"avg_ndcg@{top_k}", "avg_ndcg@1", "avg_recall@1",
                        f"avg_recall@{top_k}", "avg_top1_accuracy", "avg_mrr"]:
            s = session_agg.get(metric, 0)
            t = turn_agg.get(metric, 0)
            deltas[metric.replace("avg_", "delta_")] = t - s
            deltas[metric.replace("avg_", "delta_pct_")] = (
                ((t - s) / s * 100) if s > 0 else float('inf')
            )

    return {
        "config": {
            "mode": "simulated" if simulate else "live",
            "session_only": session_only,
            "top_k": top_k,
            "server_url": url if not simulate else None,
            "timestamp": datetime.now(timezone.utc).isoformat(),
        },
        "summary": {
            "total_queries": len(queries),
            "session_evaluated": len(session_valid),
            "turn_evaluated": len(turn_valid),
            "turn_errors": turn_errors,
            "session": session_agg,
            "turn": turn_agg,
            "deltas": deltas,
            "session_by_pattern": session_by_pattern,
            "turn_by_pattern": turn_by_pattern,
        },
        "per_query": per_query,
    }


# ── Report Formatting ──────────────────────────────────────────────────────

def format_report(results, top_k=5):
    """Format a human-readable report."""
    s = results["summary"]
    lines = []
    sep = "=" * 72

    lines.append("")
    lines.append(sep)
    lines.append("  KnowWhere Turn-Level vs Session-Level Evaluation")
    lines.append(sep)
    lines.append(f"  Mode:       {results['config']['mode'].upper()}")
    lines.append(f"  Queries:    {s['total_queries']} total")
    lines.append(f"  Session:    {s['session_evaluated']} evaluated")
    lines.append(f"  Turn:       {s['turn_evaluated']} evaluated ({s['turn_errors']} unavailable)")
    lines.append("")

    # ── Overall comparison ──
    if s["session"] and s["turn"]:
        lines.append("  ── OVERALL COMPARISON ──")
        lines.append(f"  {'Metric':<20} {'Session':>10} {'Turn':>10} {'Delta':>10} {'Change':>10}")
        lines.append(f"  {'─'*20} {'─'*10} {'─'*10} {'─'*10} {'─'*10}")
        for label, s_key, t_key in [
            (f"NDCG@{top_k}", f"avg_ndcg@{top_k}", f"avg_ndcg@{top_k}"),
            ("NDCG@1", "avg_ndcg@1", "avg_ndcg@1"),
            ("Recall@1", "avg_recall@1", "avg_recall@1"),
            (f"Recall@{top_k}", f"avg_recall@{top_k}", f"avg_recall@{top_k}"),
            ("Top-1 Acc", "avg_top1_accuracy", "avg_top1_accuracy"),
            ("MRR", "avg_mrr", "avg_mrr"),
        ]:
            sv = s["session"].get(s_key, 0)
            tv = s["turn"].get(t_key, 0)
            delta = tv - sv
            pct = (delta / sv * 100) if sv > 0 else float('inf')
            pct_str = f"{pct:+.1f}%" if pct != float('inf') else "N/A"
            lines.append(f"  {label:<20} {sv:>10.4f} {tv:>10.4f} {delta:>+10.4f} {pct_str:>10}")
        lines.append("")

    elif s["session"]:
        lines.append("  ── SESSION-LEVEL RESULTS ──")
        lines.append(f"  NDCG@{top_k}:        {s['session'].get(f'avg_ndcg@{top_k}', 0):.4f}")
        lines.append(f"  NDCG@1:             {s['session'].get('avg_ndcg@1', 0):.4f}")
        lines.append(f"  Recall@1:           {s['session'].get('avg_recall@1', 0):.4f}")
        lines.append(f"  Recall@{top_k}:         {s['session'].get(f'avg_recall@{top_k}', 0):.4f}")
        lines.append(f"  Top-1 Accuracy:     {s['session'].get('avg_top1_accuracy', 0):.4f}")
        lines.append(f"  MRR:                {s['session'].get('avg_mrr', 0):.4f}")
        lines.append("")

    # ── Per-pattern breakdown ──
    if s.get("session_by_pattern"):
        lines.append("  ── PER-PATTERN BREAKDOWN (Session) ──")
        lines.append(f"  {'Pattern':<25} {'Count':>5} {f'NDCG@{top_k}':>10} {'R@1':>8} {'Top1':>8}")
        lines.append(f"  {'─'*25} {'─'*5} {'─'*10} {'─'*8} {'─'*8}")
        for pat in sorted(s["session_by_pattern"]):
            p = s["session_by_pattern"][pat]
            lines.append(f"  {pat:<25} {p['count']:>5} "
                         f"{p[f'avg_ndcg@{top_k}']:>10.4f} "
                         f"{p['avg_recall@1']:>8.4f} "
                         f"{p['avg_top1_accuracy']:>8.4f}")
        lines.append("")

    if s.get("turn_by_pattern") and s["turn_evaluated"] > 0:
        lines.append("  ── PER-PATTERN BREAKDOWN (Turn) ──")
        lines.append(f"  {'Pattern':<25} {'Count':>5} {f'NDCG@{top_k}':>10} {'R@1':>8} {'Top1':>8}")
        lines.append(f"  {'─'*25} {'─'*5} {'─'*10} {'─'*8} {'─'*8}")
        for pat in sorted(s["turn_by_pattern"]):
            p = s["turn_by_pattern"][pat]
            pat_delta = ""
            if pat in s.get("session_by_pattern", {}):
                s_ndcg = s["session_by_pattern"][pat].get(f"avg_ndcg@{top_k}", 0)
                t_ndcg = p.get(f"avg_ndcg@{top_k}", 0)
                pat_delta = f" Δ={t_ndcg - s_ndcg:+.4f}"
            lines.append(f"  {pat:<25} {p['count']:>5} "
                         f"{p[f'avg_ndcg@{top_k}']:>10.4f} "
                         f"{p['avg_recall@1']:>8.4f} "
                         f"{p['avg_top1_accuracy']:>8.4f}{pat_delta}")
        lines.append("")

    # ── Verdict ──
    if s["deltas"]:
        ndcg_delta = s["deltas"].get(f"delta_ndcg@{top_k}", 0)
        r1_delta = s["deltas"].get("delta_recall@1", 0)
        t1_delta = s["deltas"].get("delta_top1_accuracy", 0)

        lines.append("  ── VERDICT ──")
        if ndcg_delta > 0.05:
            lines.append(f"  ✓ SIGNIFICANT improvement: NDCG@{top_k} +{ndcg_delta:.4f}")
        elif ndcg_delta > 0:
            lines.append(f"  ~ MODEST improvement: NDCG@{top_k} +{ndcg_delta:.4f}")
        elif ndcg_delta == 0:
            lines.append(f"  = NO change in NDCG@{top_k}")
        else:
            lines.append(f"  ✗ REGRESSION: NDCG@{top_k} {ndcg_delta:.4f}")

        if r1_delta > 0:
            lines.append(f"  Recall@1 improvement: {r1_delta:+.4f} — turn-level surfaces facts earlier")
        if t1_delta > 0:
            lines.append(f"  Top-1 improvement: {t1_delta:+.4f} — first result more often correct")
        lines.append("")

    # ── Failing queries ──
    failing_session = [r for r in results["per_query"]
                       if r["session"] and "error" not in r["session"]
                       and r["session"].get(f"ndcg@{top_k}", 0) == 0]
    if failing_session:
        lines.append(f"  ── FAILING QUERIES (Session, NDCG@{top_k}=0) ──")
        for r in failing_session:
            lines.append(f"    {r['id']}: {r['query'][:70]}")
        lines.append("")

    lines.append(sep)
    return "\n".join(lines)


# ── Main ───────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="KnowWhere Turn-Level vs Session-Level Comparative Evaluation"
    )
    parser.add_argument("--queries", required=True, help="Path to queries JSON file")
    parser.add_argument("--top-k", type=int, default=5, help="Top-k for retrieval (default: 5)")
    parser.add_argument("--url", default=KNOWWHERE_URL, help=f"KnowWhere URL (default: {KNOWWHERE_URL})")
    parser.add_argument("--api-key", default=API_KEY, help="API key")
    parser.add_argument("--simulate", action="store_true", help="Run in simulated mode")
    parser.add_argument("--session-only", action="store_true", help="Only run session-level evaluation")
    parser.add_argument("--output", default=None, help="Save results JSON to file")
    parser.add_argument("--quiet", action="store_true", help="Suppress per-query output")

    args = parser.parse_args()

    # Load queries
    queries_path = Path(args.queries)
    if not queries_path.exists():
        print(f"ERROR: Queries file not found: {args.queries}")
        sys.exit(1)

    with open(queries_path) as f:
        data = json.load(f)

    gold_docs = data.get("gold_documents", [])
    queries = data.get("queries", [])

    if not queries:
        print("ERROR: No queries found")
        sys.exit(1)

    print(f"KnowWhere Turn-Level vs Session-Level Evaluation")
    print(f"  Mode:       {'SIMULATED' if args.simulate else 'LIVE'}")
    if not args.simulate:
        print(f"  Server:     {args.url}")
    print(f"  Session-level only: {args.session_only}")
    print(f"  Top-K:      {args.top_k}")
    print(f"  Queries:    {len(queries)} ({data.get('description', '')})")
    print(f"  Gold Docs:  {len(gold_docs)}")
    print()

    if not args.quiet:
        print(f"{'─' * 72}")

    results = run_evaluation(
        queries=queries,
        gold_docs=gold_docs,
        top_k=args.top_k,
        simulate=args.simulate,
        session_only=args.session_only,
        url=args.url,
        api_key=args.api_key,
        verbose=not args.quiet,
    )

    if not args.quiet:
        print(f"{'─' * 72}")

    # Format and print report
    report = format_report(results, args.top_k)
    print(report)

    # Save results
    if args.output:
        out_path = Path(args.output)
    else:
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        mode = "sim" if args.simulate else "live"
        suffix = "_session_only" if args.session_only else ""
        out_path = Path("results") / f"turn_vs_session_{mode}{suffix}_{ts}.json"

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2, default=str)
    print(f"Results saved to: {out_path}")


if __name__ == "__main__":
    main()

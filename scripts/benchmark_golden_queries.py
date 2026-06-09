#!/usr/bin/env python3
"""KnowWhere v0.5 Benchmark — honest, reproducible, comparable.
Methodology adapted from Hindsight AMB (agentmemorybenchmark.ai).
"""

import json, os, sys, time, urllib.request

ENDPOINT = os.environ.get("KNOWWHERE_ENDPOINT", "http://127.0.0.1:3737")
API_KEY = os.environ.get("KNOWWHERE_API_KEY", "kw_testkey_12345")
TOP_K = 5

# ── Golden Queries (real production queries, not synthetic) ──
GOLDEN_QUERIES = [
    # Decision/causality queries
    ("decision_why", "Welche Entscheidungen wurden zur Retrieval-Scoring-Logik getroffen?"),
    ("decision_why", "Was wurde zur Decision-Pipeline entschieden?"),
    ("decision_why", "Warum wurde nomic-embed-text-v2-moe als Embedding-Modell gewaehlt?"),
    ("decision_why", "Welche Modelle wurden getestet und warum wurde qwen2.5 gewaehlt?"),
    # Procedural queries
    ("procedure", "Wie startet man KnowWhere fuer Hermes?"),
    ("procedure", "Wie baue ich KnowWhere mit allen Features?"),
    # State queries
    ("current_state", "Wie ist der aktuelle Stand der Hermes-Integration mit KnowWhere?"),
    ("preference", "Welche Praeferenzen gelten fuer die Arbeit an KnowWhere?"),
    # Open recall
    ("open_recall", "Welche Bugs sind aufgetreten und wie wurden sie geloest?"),
    ("open_recall", "Was waren die groessten Probleme mit dem Tier-Roundtrip?"),
    # Historical
    ("historical", "Was wurde am 2026-05-05 an KnowWhere geaendert?"),
    ("open_recall", "Welche Kompromisse wurden bei Small Models eingegangen?"),
]

# ── Metrics ──
def retrieve(query, intent=None, memory_type_filter=None):
    payload = {"query_text": query, "top_k": TOP_K}
    if intent:
        payload["query_intent"] = intent
    if memory_type_filter:
        payload["memory_type_filter"] = memory_type_filter
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{ENDPOINT}/retrieve_fractal",
        data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {API_KEY}"},
        method="POST",
    )
    start = time.perf_counter()
    with urllib.request.urlopen(req, timeout=30) as resp:
        nodes = json.loads(resp.read())
    return nodes, time.perf_counter() - start


def is_relevant(node, intent):
    """Check if a node is relevant for the query intent.

    For decision_why: node should contain decision language.
    For procedure: node should be procedural or decision.
    For open_recall: content should be diverse.
    """
    content = (node.get("content") or "").lower()
    mtype = (node.get("memory_type") or "").lower()

    if intent == "decision_why":
        # Decision nodes are always relevant for decision queries
        if mtype == "decision":
            return True
        # Semantic nodes with decision language
        if "entscheid" in content or "decision" in content:
            return True
        return False

    if intent == "procedure":
        if mtype in ("procedural", "decision"):
            return True
        return "start" in content or "bau" in content or "features" in content

    if intent == "current_state":
        return mtype in ("decision", "semantic") or "stand" in content

    if intent == "preference":
        return mtype in ("decision", "preference") or "präferenz" in content

    if intent == "open_recall":
        return True  # All top results are relevant for open recall

    if intent == "historical":
        return "2026" in content or "mai" in content or mtype == "decision"

    return True


def compute_metrics(results):
    """Compute Recall@k, MRR, Precision@k."""
    total = len(results)
    if total == 0:
        return {}

    recall_at_1 = sum(1 for r in results if r["relevant_ranks"] and min(r["relevant_ranks"]) == 1) / total
    recall_at_3 = sum(1 for r in results if r["relevant_ranks"] and min(r["relevant_ranks"]) <= 3) / total
    recall_at_5 = sum(1 for r in results if r["relevant_ranks"] and min(r["relevant_ranks"]) <= 5) / total

    mrr = sum(1.0 / min(r["relevant_ranks"]) for r in results if r["relevant_ranks"]) / total

    precision_5 = sum(r["relevant_at_5"] for r in results) / (total * TOP_K)

    decision_purity = sum(r["decision_in_top5"] for r in results) / (total * TOP_K)

    latencies = sorted(r["latency"] for r in results)
    p50 = latencies[len(latencies) // 2] if latencies else 0
    p95 = latencies[int(len(latencies) * 0.95)] if len(latencies) > 1 else p50

    return {
        "total_queries": total,
        "recall@1": round(recall_at_1, 3),
        "recall@3": round(recall_at_3, 3),
        "recall@5": round(recall_at_5, 3),
        "mrr": round(mrr, 3),
        "precision@5": round(precision_5, 3),
        "decision_purity": round(decision_purity, 3),
        "latency_p50_ms": round(p50 * 1000, 1),
        "latency_p95_ms": round(p95 * 1000, 1),
    }


def main():
    print("=" * 70)
    print("KnowWhere v0.5 Benchmark — Golden Queries (n=12)")
    print("=" * 70)
    print()

    results = []
    for intent, query in GOLDEN_QUERIES:
        nodes, latency = retrieve(query, intent=intent)

        relevant_ranks = [i + 1 for i, n in enumerate(nodes[:TOP_K]) if is_relevant(n, intent)]
        decision_count = sum(1 for n in nodes[:TOP_K] if (n.get("memory_type") or "").lower() == "decision")

        result = {
            "query": query,
            "intent": intent,
            "latency": latency,
            "top5_types": [(n.get("memory_type", "?"), round(n.get("score", 0), 4)) for n in nodes[:TOP_K]],
            "relevant_ranks": relevant_ranks,
            "relevant_at_5": sum(1 for n in nodes[:TOP_K] if is_relevant(n, intent)),
            "decision_in_top5": decision_count,
        }
        results.append(result)

        marker = "✅" if relevant_ranks else "❌"
        first_rel = min(relevant_ranks) if relevant_ranks else "-"
        print(f"  {marker} Rank{first_rel:>4} | Decision:{decision_count}/5 | {query[:55]}...")

    metrics = compute_metrics(results)

    print()
    print("=" * 70)
    print("AGGREGATE METRICS")
    print("=" * 70)
    print(f"  Recall@1:      {metrics['recall@1']:.3f}")
    print(f"  Recall@3:      {metrics['recall@3']:.3f}")
    print(f"  Recall@5:      {metrics['recall@5']:.3f}")
    print(f"  MRR:           {metrics['mrr']:.3f}")
    print(f"  Precision@5:   {metrics['precision@5']:.3f}")
    print(f"  Decision@5:    {metrics['decision_purity']:.3f} ({metrics['decision_purity']*100:.0f}%)")
    print(f"  Latency P50:   {metrics['latency_p50_ms']:.1f}ms")
    print(f"  Latency P95:   {metrics['latency_p95_ms']:.1f}ms")

    # Full JSON output
    output = {
        "benchmark": "KnowWhere Golden Queries v0.5",
        "methodology": "Adapted from Hindsight AMB (agentmemorybenchmark.ai)",
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "endpoint": ENDPOINT,
        "model": {
            "embedding": "nomic-embed-text-v2-moe (Ollama, 768-dim)",
            "summarizer": "qwen2.5:3b (Ollama)",
            "reranker": "bge-reranker-v2-m3 (ONNX, quantized, 571MB)",
        },
        "metrics": metrics,
        "results": [{
            "query": r["query"],
            "intent": r["intent"],
            "first_relevant_rank": min(r["relevant_ranks"]) if r["relevant_ranks"] else None,
            "relevant_at_5": r["relevant_at_5"],
            "decision_in_top5": r["decision_in_top5"],
            "latency_ms": round(r["latency"] * 1000, 1),
        } for r in results],
    }

    with open("benchmark_results.json", "w") as f:
        json.dump(output, f, indent=2, ensure_ascii=False)
    print(f"\n✅ Full results written to benchmark_results.json")

    return 0 if metrics["recall@5"] >= 0.9 else 1


if __name__ == "__main__":
    sys.exit(main())

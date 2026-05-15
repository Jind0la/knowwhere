#!/usr/bin/env python3
"""Retrieval evaluation harness for KnowWhere.

Tests retrieval quality with and without multi-query expansion.
Stores diverse test nodes covering tools, patterns, and domains,
then runs queries with known expected results.

Usage:
    python3 scripts/eval_retrieval.py
    python3 scripts/eval_retrieval.py --multi-query  # compare with multi-query
"""
import requests
import json
import sys
import time

BASE = "http://localhost:3737"
TOKEN = "kw_testkey_12345"
HEADERS = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}

# Test data: nodes spanning different semantic domains
TEST_NODES = [
    # Tools
    {"pointer": "redis-cache", "content": "Redis wird als In-Memory-Cache für User-Sessions verwendet. TTL-basierte Eviction mit 30-Minuten-Timeout.", "metadata": {"domain": "tool", "topic": "caching"}},
    {"pointer": "redis-queue", "content": "Redis PubSub wird als Message-Queue für asynchrone Job-Verarbeitung eingesetzt. Jobs werden über Channels verteilt.", "metadata": {"domain": "tool", "topic": "messaging"}},
    {"pointer": "postgres-queue", "content": "PostgreSQL LISTEN/NOTIFY dient als Message-Queue für interne System-Events. ACID-garantierte Zustellung.", "metadata": {"domain": "tool", "topic": "messaging"}},
    {"pointer": "docker-deploy", "content": "Docker Compose wird für lokale Entwicklungsumgebungen genutzt. Services: Postgres, Redis, KnowWhere.", "metadata": {"domain": "tool", "topic": "deployment"}},
    # Patterns
    {"pointer": "ttl-pattern", "content": "TTL-basierte Eviction wird in Redis-Cache und KnowWhere L0-Tier verwendet. Pattern: time-bounded storage.", "metadata": {"domain": "pattern", "topic": "storage"}},
    {"pointer": "pubsub-pattern", "content": "Publish-Subscribe Pattern wird für Event-getriebene Systeme eingesetzt. Implementiert mit Redis und PostgreSQL.", "metadata": {"domain": "pattern", "topic": "messaging"}},
    {"pointer": "acid-guarantee", "content": "ACID-Garantien durch PostgreSQL. Wichtig für Message-Queues wo Nachrichten nicht verloren gehen dürfen.", "metadata": {"domain": "pattern", "topic": "reliability"}},
    # Decisions
    {"pointer": "decision-embedding", "content": "DECISION: nomic-embed-text v1.5 als Embedding-Modell gewählt. 8K Kontext, 768d, Matryoshka-Support.", "metadata": {"domain": "decision", "topic": "ml"}},
    {"pointer": "decision-fractal", "content": "DECISION: Fractal-Hierarchie wird durch Matryoshka-Embedding-Truncation realisiert, nicht durch LLM-Summarization.", "metadata": {"domain": "decision", "topic": "architecture"}},
    {"pointer": "decision-multi-query", "content": "DECISION: Multi-Query-Retrieval als Alternative zu PCA-basiertem Disentangled Clustering gewählt.", "metadata": {"domain": "decision", "topic": "architecture"}},
]

# Queries with expected relevant pointer IDs (partial match on pointer prefix)
TEST_QUERIES = [
    {
        "query": "Message-Queue Implementierungen",
        "expect_pointers": ["redis-queue", "postgres-queue"],
        "min_recall": 2,  # at least 2 of expected in top-5
    },
    {
        "query": "Caching Strategien",
        "expect_pointers": ["redis-cache", "ttl-pattern"],
        "min_recall": 1,
    },
    {
        "query": "Architektur-Entscheidungen Embedding",
        "expect_pointers": ["decision-embedding", "decision-fractal"],
        "min_recall": 1,
    },
    {
        "query": "Event-getriebene Systeme",
        "expect_pointers": ["redis-queue", "pubsub-pattern"],
        "min_recall": 1,
    },
    {
        "query": "Deployment Werkzeuge",
        "expect_pointers": ["docker-deploy"],
        "min_recall": 1,
    },
]


def store_nodes():
    """Store all test nodes."""
    for node in TEST_NODES:
        resp = requests.post(f"{BASE}/store_external", headers=HEADERS, json=node, timeout=10)
        if resp.status_code != 201:
            print(f"  FAIL store {node['pointer']}: {resp.status_code} {resp.text}")
            return False
    print(f"  Stored {len(TEST_NODES)} nodes")
    return True


def run_queries(multi_query=False):
    """Run all test queries and measure recall."""
    results = []
    for tc in TEST_QUERIES:
        body = {
            "query_text": tc["query"],
            "top_k": 5,
            "multi_query": multi_query,
        }
        t0 = time.time()
        resp = requests.post(f"{BASE}/retrieve_fractal", headers=HEADERS, json=body, timeout=30)
        elapsed = time.time() - t0

        if resp.status_code != 200:
            print(f"  FAIL query '{tc['query']}': {resp.status_code}")
            results.append({"query": tc["query"], "recall": 0, "precision": 0, "elapsed": elapsed, "error": True})
            continue

        nodes = resp.json()
        found_pointers = []
        for n in nodes[:5]:
            ptr = n.get("original_pointer", "")
            if ptr:
                found_pointers.append(ptr)

        # Count how many expected pointers were found in top-5
        hits = sum(1 for ep in tc["expect_pointers"] if any(ep in fp for fp in found_pointers))
        recall = hits / len(tc["expect_pointers"]) if tc["expect_pointers"] else 0
        precision = hits / min(len(nodes), 5) if nodes else 0

        results.append({
            "query": tc["query"],
            "recall": recall,
            "precision": precision,
            "elapsed": elapsed,
            "found": found_pointers,
            "expected": tc["expect_pointers"],
            "pass": recall >= (tc["min_recall"] / len(tc["expect_pointers"])),
        })

        status = "✅" if results[-1]["pass"] else "❌"
        print(f"  {status} '{tc['query'][:50]}' recall={recall:.1%} precision={precision:.1%} in {elapsed:.0f}ms")

    return results


def print_summary(results, label):
    """Print summary statistics."""
    if not results:
        return
    avg_recall = sum(r["recall"] for r in results) / len(results)
    avg_precision = sum(r["precision"] for r in results) / len(results)
    avg_time = sum(r["elapsed"] for r in results) / len(results)
    passed = sum(1 for r in results if r.get("pass", False))
    print(f"\n--- {label} ---")
    print(f"  Queries: {len(results)}")
    print(f"  Passed: {passed}/{len(results)}")
    print(f"  Avg Recall: {avg_recall:.1%}")
    print(f"  Avg Precision: {avg_precision:.1%}")
    print(f"  Avg Latency: {avg_time:.0f}ms")


if __name__ == "__main__":
    multi = "--multi-query" in sys.argv

    print("=== KnowWhere Retrieval Eval ===")
    print(f"Multi-Query: {multi}")
    print()

    print("Storing nodes...")
    if not store_nodes():
        sys.exit(1)

    print(f"\nRunning {len(TEST_QUERIES)} queries...")
    results = run_queries(multi_query=multi)
    print_summary(results, "Multi-Query" if multi else "Single-Query")

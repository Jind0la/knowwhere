#!/usr/bin/env python3
"""
Qualitative Retrieval Test Script for KnowWhere

Compares Baseline vs Optimized retrieval on the benchmark server.
"""

import requests
from typing import List, Dict, Any

SERVER = "http://localhost:3738"
API_KEY = "kw_bench_key_12345"
HEADERS = {
    "Content-Type": "application/json",
    "Authorization": f"Bearer {API_KEY}"
}

TEST_QUERIES = [
    "What was the latest decision about the KnowWhere architecture?",
    "How did we solve the session leakage problem?",
    "What are the current open tasks in the retrieval quality project?",
    "Tell me about the early experiments with fractal memory.",
    "What was the outcome of the half-life discussion?",
    "Which embedding model are we currently using?",
    "What were the first benchmark results like?",
    "How does the temporal scoring work exactly?",
]

def retrieve(query: str, temporal_weight: float, use_session: bool) -> List[Dict[str, Any]]:
    payload = {
        "query_text": query,
        "top_k": 5,
        "temporal_weight": temporal_weight,
        "use_session_boost": use_session
    }
    try:
        r = requests.post(f"{SERVER}/retrieve_fractal", json=payload, headers=HEADERS, timeout=30)
        if r.status_code == 200:
            data = r.json()
            if isinstance(data, list):
                return data
            return data.get("results", []) if isinstance(data, dict) else []
        print(f"Error {r.status_code}")
        return []
    except Exception as e:
        print(f"Exception: {e}")
        return []

def print_comparison(query: str, baseline: List[Dict], optimized: List[Dict]):
    print("=" * 90)
    print(f"QUERY: {query}")
    print("-" * 90)
    
    print("BASELINE (temporal_weight=0.0)")
    for i, r in enumerate(baseline, 1):
        sess = r.get("metadata", {}).get("session_id", "?")[:8]
        score = r.get("score", 0)
        content = r.get("content", "")[:100].replace("\n", " ")
        print(f"  {i}. [S:{sess}] {score:.3f} | {content}...")
    
    print()
    print("OPTIMIZED (temporal_weight=0.5 + session)")
    for i, r in enumerate(optimized, 1):
        sess = r.get("metadata", {}).get("session_id", "?")[:8]
        score = r.get("score", 0)
        content = r.get("content", "")[:100].replace("\n", " ")
        print(f"  {i}. [S:{sess}] {score:.3f} | {content}...")
    print("=" * 90)
    print()

def main():
    print("Qualitative Retrieval Test - Benchmark Server\n")
    for q in TEST_QUERIES:
        bl = retrieve(q, 0.0, False)
        op = retrieve(q, 0.5, True)
        print_comparison(q, bl, op)

if __name__ == "__main__":
    main()
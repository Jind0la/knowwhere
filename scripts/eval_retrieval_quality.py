#!/usr/bin/env python3
"""
Retrieval Quality Evaluation - Session-based Recency Measurement

Since created_at is currently ignored by store_external, we measure recency
via explicit session order instead (which we control in the benchmark).

Session order (oldest → newest):
    bench_sess_01 < bench_sess_02 < bench_sess_03 < bench_sess_04 < bench_sess_05

Metrics:
- Avg Recency Score (higher = more recent sessions in top results)
- % Results from newest sessions (sess_04 + sess_05)
- Session concentration
"""

import json
import requests
from pathlib import Path
from typing import List, Dict, Any
from collections import defaultdict

SERVER_URL = "http://localhost:3738"

API_KEY = "kw_bench_key_12345"
HEADERS = {
    "Content-Type": "application/json",
    "Authorization": f"Bearer {API_KEY}"
}

BENCHMARK_FILE = Path("/Users/nimarfranklinmac/knowwhere/benchmarks/data/longmemeval_s_cleaned.json")
TOP_K = 8

# Session recency mapping (higher = more recent)
SESSION_RECENCY = {
    "bench_sess_01": 1,
    "bench_sess_02": 2,
    "bench_sess_03": 3,
    "bench_sess_04": 4,
    "bench_sess_05": 5,
}

NEWEST_SESSIONS = {"bench_sess_04", "bench_sess_05"}

CONFIGS = [
    {"name": "baseline",               "temporal_weight": 0.0,  "use_session": False},
    {"name": "temporal_0.35",          "temporal_weight": 0.35, "use_session": False},
    {"name": "temporal_0.50",          "temporal_weight": 0.50, "use_session": False},
    {"name": "temporal_0.65",          "temporal_weight": 0.65, "use_session": False},
    {"name": "temporal_0.50+session",  "temporal_weight": 0.50, "use_session": True},
]


def is_benchmark_result(item: Dict) -> bool:
    meta = item.get("metadata", {})
    return meta.get("benchmark") is True


def load_and_adapt_questions(limit: int = 15) -> List[Dict]:
    if not BENCHMARK_FILE.exists():
        print(f"Benchmark file not found: {BENCHMARK_FILE}")
        return []

    with open(BENCHMARK_FILE) as f:
        data = json.load(f)

    questions = [q for q in data if q.get("question_type") == "single-session-user" and q.get("answer")]

    adapted = []
    for q in questions[:limit]:
        adapted.append({
            "original_question": q["question"],
            "query": f"Q: {q['question']}",
            "answer": q.get("answer", ""),
        })
    return adapted


def retrieve(query_text: str, temporal_weight: float = None, session_id: str = None) -> List[Dict]:
    payload = {
        "query_text": query_text,
        "top_k": TOP_K,
    }
    if temporal_weight is not None:
        payload["temporal_weight"] = temporal_weight
    if session_id:
        payload["session_id"] = session_id

    try:
        resp = requests.post(f"{SERVER_URL}/retrieve_fractal", json=payload, headers=HEADERS, timeout=30)
        resp.raise_for_status()
        return resp.json()
    except Exception as e:
        print(f"Retrieval failed: {e}")
        return []


def evaluate_config(questions: List[Dict], config: Dict) -> Dict[str, Any]:
    recency_scores = []
    newest_session_hits = 0
    session_counts = defaultdict(int)
    total_results = 0

    for q in questions:
        query = q["query"]
        session_id = "bench_sess_05" if config.get("use_session") else None

        retrieved = retrieve(
            query_text=query,
            temporal_weight=config.get("temporal_weight"),
            session_id=session_id
        )

        for item in retrieved[:5]:
            if not is_benchmark_result(item):
                continue

            total_results += 1
            sess = item.get("metadata", {}).get("session_id")

            if sess and sess in SESSION_RECENCY:
                recency_scores.append(SESSION_RECENCY[sess])
                session_counts[sess] += 1

                if sess in NEWEST_SESSIONS:
                    newest_session_hits += 1

    avg_recency = sum(recency_scores) / len(recency_scores) if recency_scores else None
    newest_pct = (newest_session_hits / total_results * 100) if total_results > 0 else None

    concentration = None
    if session_counts and total_results > 0:
        most_common = max(session_counts.items(), key=lambda x: x[1])[0]
        concentration = (session_counts[most_common] / total_results * 100)

    return {
        "config": config["name"],
        "avg_recency_score": round(avg_recency, 2) if avg_recency else None,
        "newest_sessions_pct": round(newest_pct, 1) if newest_pct else None,
        "session_concentration_pct": round(concentration, 1) if concentration else None,
        "questions_tested": len(questions),
    }


def main():
    print("=== KnowWhere Retrieval Quality Evaluation ===")
    print("Method: Session-order recency (because created_at is currently ignored)")
    print("Session order: 01 (oldest) < 02 < 03 < 04 < 05 (newest)\n")

    questions = load_and_adapt_questions(limit=12)
    if not questions:
        print("No questions loaded.")
        return

    print(f"Running {len(questions)} questions across configurations...\n")

    all_results = []
    for config in CONFIGS:
        print(f"Evaluating: {config['name']}")
        res = evaluate_config(questions, config)
        all_results.append(res)
        print(f"  Avg Recency Score: {res.get('avg_recency_score', '-')}")
        print(f"  Newest Sessions (04+05): {res.get('newest_sessions_pct', '-')} %")
        print(f"  Session Concentration: {res.get('session_concentration_pct', '-')} %\n")

    print("=" * 75)
    print("SUMMARY - Temporal + Session Scoring Effects (Session Recency)")
    print("=" * 75)
    print(f"{'Config':<25} | {'Avg Recency':>10} | {'Newest %':>9} | {'Concentration':>12}")
    print("-" * 75)
    for r in all_results:
        rec = r.get('avg_recency_score', '-')
        new = r.get('newest_sessions_pct', '-')
        conc = r.get('session_concentration_pct', '-')
        rec_str = f"{rec}" if rec is not None else "-"
        new_str = f"{new}%" if new is not None else "-"
        conc_str = f"{conc}%" if conc is not None else "-"
        print(f"{r['config']:<25} | {rec_str:>10} | {new_str:>9} | {conc_str:>12}")
    print("=" * 75)
    print("\nNote: Higher Avg Recency + higher 'Newest %' = stronger temporal effect")


if __name__ == "__main__":
    main()

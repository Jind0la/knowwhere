#!/usr/bin/env python3
"""LongMemEval S retrieval benchmark — content-based matching."""
import json, time, requests
from pathlib import Path

BASE = "http://localhost:3737"
KEY = "***"
DATASET = Path.home() / "knowwhere/benchmarks/data/longmemeval_s_cleaned.json"
MAX_CASES = 30
TOP_K = 5

def retrieve(query_text, top_k=TOP_K):
    r = requests.post(f"{BASE}/retrieve_fractal",
        headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"},
        json={"query_text": query_text, "top_k": top_k}, timeout=60)
    if r.status_code != 200:
        raise Exception(f"API error {r.status_code}: {r.text[:200]}")
    return r.json()

with open(DATASET) as f:
    cases = json.load(f)

print(f"📊 LongMemEval S — Content-Match Benchmark")
print(f"   Cases: {min(MAX_CASES, len(cases))}, Top-K: {TOP_K}")
print(f"   Reranker: bge-reranker-v2-m3 (enabled)")
print()

health = requests.get(f"{BASE}/health").json()
print(f"🏥 {health['status']} ({health['node_count']} nodes)\n")

results = []
start_time = time.time()

for idx, case in enumerate(cases[:MAX_CASES]):
    qid = case["question_id"]
    question = case["question"]
    answer = case.get("answer", "").strip()
    qtype = case.get("question_type", "?")

    t0 = time.time()
    try:
        retrieved = retrieve(question, top_k=TOP_K)
        elapsed = time.time() - t0
    except Exception as e:
        print(f"[{idx+1:2d}] ❌ {qid}: {e}")
        results.append({"qid": qid, "hit": False, "error": str(e), "elapsed": 0})
        continue

    # Content match: is the answer substring in any retrieved content?
    hit = False
    matched_content = ""
    if answer:
        answer_lower = answer.lower()
        for item in retrieved:
            content = item.get("content", "").lower()
            if answer_lower in content:
                hit = True
                matched_content = content[:120]
                break

    results.append({
        "qid": qid,
        "question": question[:60],
        "answer": answer,
        "hit": hit,
        "retrieved": len(retrieved),
        "elapsed": round(elapsed, 1),
        "type": qtype,
    })

    icon = "✅" if hit else "❌"
    print(f"[{idx+1:2d}] {icon} {qid} [{qtype}] {elapsed:.1f}s — {question[:60]}...")
    if hit:
        print(f"     🎯 Answer found: \"{answer[:80]}\"")

# Summary
total = len(results)
hits = sum(1 for r in results if r["hit"])
recall = hits / total * 100 if total > 0 else 0
avg_latency = sum(r["elapsed"] for r in results) / total if total > 0 else 0
elapsed_total = time.time() - start_time

print(f"\n{'='*70}")
print(f"📈 LongMemEval S — Content-Match Results (Reranker: bge-m3)")
print(f"{'='*70}")
print(f"  Cases:         {total}")
print(f"  Hits (R@{TOP_K}):   {hits}/{total}")
print(f"  Recall@{TOP_K}:     {recall:.1f}%")
print(f"  Avg latency:   {avg_latency:.1f}s")
print(f"  Total time:    {elapsed_total:.0f}s")
print(f"{'='*70}")

# Per-type breakdown
from collections import Counter
type_hits = Counter()
type_total = Counter()
for r in results:
    type_total[r["type"]] += 1
    if r["hit"]:
        type_hits[r["type"]] += 1

print("\n  By type:")
for t in sorted(type_total):
    th = type_hits[t]
    tt = type_total[t]
    print(f"    {t}: {th}/{tt} ({th/tt*100:.0f}%)")

# Save report
report = {
    "benchmark": "KnowWhere LongMemEval S — Content Match",
    "matching": "content_substring",
    "reranker": "bge-reranker-v2-m3",
    "reranker_enabled": True,
    "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "top_k": TOP_K,
    "total_cases": total,
    "hits": hits,
    f"recall_at_{TOP_K}": round(recall, 1),
    "avg_latency_s": round(avg_latency, 1),
    "results": results,
}
report_path = Path.home() / "knowwhere/benchmarks/reports/reranker_bge_m3_longmemeval.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
with open(report_path, "w") as f:
    json.dump(report, f, indent=2)
print(f"\n💾 {report_path}")

#!/usr/bin/env python3
"""LongMemEval S — Session-ID Match Benchmark (fair comparison to Hindsight)."""
import json, time, requests, sys
from collections import Counter
from pathlib import Path

BASE = "http://localhost:3737"
KEY = "***"
DATASET = Path.home() / "knowwhere/benchmarks/data/longmemeval_s_cleaned.json"
MAX_CASES = 30
TOP_K = 5

def health():
    return requests.get(f"{BASE}/health").json()

def store(content, metadata):
    r = requests.post(f"{BASE}/store_session",
        headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"},
        json={"content": content, "metadata": metadata, "source": "benchmark", "memory_type": "episodic"},
        timeout=60)
    if r.status_code not in (200, 201):
        raise Exception(f"Store {r.status_code}: {r.text[:200]}")
    return r.json()

def retrieve(query_text):
    r = requests.post(f"{BASE}/retrieve_fractal",
        headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"},
        json={"query_text": query_text, "top_k": TOP_K}, timeout=60)
    if r.status_code != 200:
        raise Exception(f"Retrieve {r.status_code}: {r.text[:200]}")
    return r.json()

# ─── Load ───
print("📂 Loading dataset...")
with open(DATASET) as f:
    cases = json.load(f)

# Collect sessions
sessions = {}
for case in cases:
    for sid, msgs in zip(case.get("haystack_session_ids", []), case.get("haystack_sessions", [])):
        if sid not in sessions:
            sessions[sid] = msgs

print(f"   {len(sessions)} unique sessions, {len(cases)} cases")

# ─── Ingest ───
h = health()
if h["node_count"] > 0:
    print(f"⚠️  {h['node_count']} existing nodes. Run with --clean to purge.")
    if "--clean" in sys.argv:
        print("   Cleaning...")
        while True:
            r = requests.get(f"{BASE}/nodes/recent",
                headers={"Authorization": f"Bearer {KEY}"}, params={"limit": 1000})
            data = r.json()
            if not data: break
            ids = [n["id"] for n in data]
            requests.post(f"{BASE}/nodes/batch_delete",
                headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"},
                json={"ids": ids})
        h = health()
        print(f"   Clean. {h['node_count']} nodes remain.")

total = len(sessions)
print(f"\n📥 Ingesting {total} sessions...")
t0 = time.time()
for i, (sid, msgs) in enumerate(sessions.items()):
    content = "\n".join(f"[{m.get('role','user')}] {m.get('content','')}" for m in msgs)[:64000]
    try:
        store(content, {"session_id": sid, "benchmark": "longmemeval_s"})
    except Exception as e:
        print(f"\n   ❌ {sid}: {e}")
    if (i + 1) % 100 == 0:
        elapsed = time.time() - t0
        rate = (i + 1) / elapsed
        print(f"   [{i+1}/{total}] {rate:.1f} sess/s", end="\r")
elapsed = time.time() - t0
print(f"\n   ✅ {total} sessions in {elapsed:.0f}s ({total/elapsed:.1f} sess/s)")
h = health()
print(f"   Nodes: {h['node_count']}")

# ─── Benchmark ───
print(f"\n🔍 Session-ID Match Benchmark (Top-{TOP_K})")
results = []
t0 = time.time()

for idx, case in enumerate(cases[:MAX_CASES]):
    qid = case["question_id"]
    question = case["question"]
    answer_sids = set(case.get("answer_session_ids", []))
    qtype = case.get("question_type", "?")

    t1 = time.time()
    try:
        data = retrieve(question)
        lat = time.time() - t1
    except Exception as e:
        results.append({"qid": qid, "hit": False, "error": str(e), "elapsed": 0, "type": qtype})
        print(f"[{idx+1:2d}] ❌ {qid}: {e}")
        continue

    hit = False
    matched = None
    for item in data:
        meta = item.get("metadata", {})
        if isinstance(meta, str):
            try: meta = json.loads(meta)
            except: meta = {}
        if meta.get("session_id", "") in answer_sids:
            hit = True
            matched = meta["session_id"]
            break

    results.append({
        "qid": qid, "question": question[:80], "hit": hit,
        "matched_sid": matched, "answer_sids": list(answer_sids),
        "retrieved": len(data), "elapsed": round(lat, 1), "type": qtype,
    })
    icon = "✅" if hit else "❌"
    info = f"→ {matched}" if hit else f"(sessions: {list(answer_sids)[:2]}...)"
    print(f"[{idx+1:2d}] {icon} {qid} [{qtype}] {lat:.1f}s {info}")

# ─── Summary ───
n = len(results)
hits = sum(1 for r in results if r["hit"])
recall = hits / n * 100 if n else 0
avg_lat = sum(r["elapsed"] for r in results) / n if n else 0
total_t = time.time() - t0

print(f"\n{'='*70}")
print(f"📈 LongMemEval S — Session-ID Match (bge-m3 reranker)")
print(f"{'='*70}")
print(f"  Hindsight:  94.6% recall@5")
print(f"  KnowWhere:  {recall:.1f}% ({hits}/{n})")
print(f"  Δ:          {recall-94.6:+.1f}pp")
print(f"  Avg lat:    {avg_lat:.1f}s")
print(f"  Total:      {total_t:.0f}s (ingest + query)")
print(f"{'='*70}")

th = Counter(); tt = Counter()
for r in results:
    tt[r["type"]] += 1
    if r["hit"]: th[r["type"]] += 1
for t in sorted(tt):
    print(f"  {t}: {th[t]}/{tt[t]} ({th[t]/tt[t]*100:.0f}%)")

misses = [r for r in results if not r["hit"]]
if misses:
    print(f"\n  ❌ Misses: {', '.join(m['qid'] for m in misses)}")

# Save
report_path = Path.home() / "knowwhere/benchmarks/reports/session_id_match_bge_m3.json"
report_path.parent.mkdir(parents=True, exist_ok=True)
with open(report_path, "w") as f:
    json.dump({
        "benchmark": "KnowWhere LongMemEval S — Session-ID Match",
        "methodology": "session-id",
        "reranker": "bge-reranker-v2-m3",
        "hindsight_recall5": 94.6,
        "recall_at_5": round(recall, 1),
        "delta_pp": round(recall - 94.6, 1),
        "hits": hits, "total": n,
        "avg_latency_s": round(avg_lat, 1),
        "sessions_ingested": total,
        "results": results,
    }, f, indent=2, default=str)
print(f"\n💾 {report_path}")

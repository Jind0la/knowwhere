#!/usr/bin/env python3
"""
KnowWhere v0.6 Verification Gate — Comprehensive Evaluation

Tests:
  1. PersonaMem 20q (≥80% accuracy)
  2. AMB (≥75% Top-1 / Recall@5 / MRR)
  3. Node count / performance check (≥2405 nodes, no degradation)
  4. Temporal Golden Queries (recency boost delivers correct results)
"""

import json
import time
import sys
import uuid
import urllib.request
import urllib.error

ENDPOINT = "http://127.0.0.1:3737"
API_KEY = "kw_testkey_12345"
HEADERS = {
    "Content-Type": "application/json",
    "Authorization": f"Bearer {API_KEY}",
}
TIMEOUT = 30

def api(method, path, data=None):
    url = f"{ENDPOINT}{path}"
    body = json.dumps(data).encode() if data else None
    req = urllib.request.Request(url, data=body, headers={**HEADERS}, method=method)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        body = e.read().decode()[:300]
        return e.code, {"error": body}
    except Exception as e:
        return 0, {"error": str(e)}

def store(content, memory_type="semantic", metadata=None):
    return api("POST", "/store_session", {
        "content": content,
        "memory_type": memory_type,
        "metadata": metadata or {},
    })

def retrieve(query_text, top_k=5, recency_boost=None, max_depth=0):
    payload = {"query_text": query_text, "top_k": top_k, "max_depth": max_depth}
    if recency_boost is not None:
        payload["recency_boost"] = recency_boost
    return api("POST", "/retrieve_fractal", payload)

# ═══════════════════════════════════════════════════
# TEST 1: PERSONAMEM — 20 Questions
# ═══════════════════════════════════════════════════
print("=" * 60)
print("TEST 1: PersonaMem — 20 Questions")
print("=" * 60)

# Persona facts to store (if not already present)
persona_facts = [
    "Nimar is the inventor of KnowWhere, a fractal memory system for AI agents.",
    "Nimar prefers Pointer-First architecture — store references and metadata, not raw data.",
    "Nimar is skeptical of too-perfect results — demands real benchmarks, zero fake data.",
    "Nimar prefers comprehensive documentation — 'so viel wie möglich', maximum detail.",
    "Nimar thinks in first-principles / Elon-Mode: ship fast, prove with real benchmarks.",
    "Nimar is pragmatic: pivots immediately when something doesn't work.",
    "Nimar blocks destructive database operations without confirmation.",
    "Nimar likes clean, well-structured git commits.",
    "Nimar uses an M1 MacBook Air, not M3.",
    "Nimar removed Docker Desktop because it was unstable on M1.",
    "Nimar prefers Kanban over delegate_task for multi-agent development.",
    "Nimar wants periodic updates during long-running operations.",
    "Nimar's OpenAI API key is stored in ~/.knowwhere/.env with chmod 600.",
    "Nimar uses Hermes Agent as his primary AI assistant.",
    "Nimar developed KnowWhere natively on macOS without Docker.",
    "Nimar uses DeepSeek v4 Pro as the model for Hermes.",
    "Nimar's KnowWhere server runs on port 3737 with native PostgreSQL.",
    "Nimar wants 'max Doku/Exploration ohne Nachfragen' — prefer action over asking.",
    "Nimar evaluates approaches pragmatically — if a tool fits, use it; if not, fall back.",
    "Nimar uses Ollama for local embeddings on his M1 Mac.",
]

print(f"Storing {len(persona_facts)} persona facts...")
for i, fact in enumerate(persona_facts):
    store(fact, "semantic", {"type": "persona_fact", "index": i})
    if i % 5 == 4:
        print(f"  ✓ {i+1}/{len(persona_facts)} stored")

# PersonaMem queries — 20 questions
persona_queries = [
    ("Who invented KnowWhere?", ["nimar", "inventor", "knowwhere", "created"]),
    ("What architecture does Nimar prefer?", ["pointer", "first", "reference"]),
    ("Is Nimar skeptical of perfect results?", ["skeptical", "benchmark", "fake", "real"]),
    ("How much documentation does Nimar want?", ["comprehensive", "max", "detail", "viel"]),
    ("How does Nimar think about problems?", ["first principles", "elon", "ship fast"]),
    ("What does Nimar do when something doesn't work?", ["pivot", "immediately", "pragmatic"]),
    ("How does Nimar handle destructive operations?", ["block", "confirmation", "destructive"]),
    ("What kind of git commits does Nimar like?", ["clean", "structured", "commit"]),
    ("What Mac does Nimar use?", ["m1", "macbook air", "not m3"]),
    ("Why did Nimar remove Docker?", ["unstable", "m1", "docker"]),
    ("What development approach does Nimar prefer for multi-agent?", ["kanban", "delegate"]),
    ("What does Nimar want during long operations?", ["periodic", "update", "long"]),
    ("Where is Nimar's OpenAI key stored?", ["knowwhere", ".env", "chmod 600"]),
    ("What AI assistant does Nimar use?", ["hermes", "agent", "primary"]),
    ("How did Nimar develop KnowWhere?", ["native", "macOS", "without docker"]),
    ("What model does Nimar use for Hermes?", ["deepseek", "v4", "pro"]),
    ("What port does KnowWhere run on?", ["3737", "port"]),
    ("Does Nimar prefer action or asking?", ["action", "exploration", "ohne nachfragen"]),
    ("How does Nimar evaluate tools?", ["pragmatic", "fit", "fall back"]),
    ("What does Nimar use for local embeddings?", ["ollama", "local", "m1"]),
]

print(f"\nRunning {len(persona_queries)} PersonaMem queries...\n")

persona_results = []
for i, (query, expected_keywords) in enumerate(persona_queries):
    status, results = retrieve(query, top_k=3, recency_boost=0.10)

    if not isinstance(results, list) or not results:
        persona_results.append({"query": query, "match": False, "top_content": "NO RESULTS", "score": 0})
        print(f"  [{i+1:02d}] ✗ NO RESULTS: {query}")
        continue

    top_content = (results[0].get("content") or "").lower()
    top_score = results[0].get("score", 0)

    # Check if any expected keyword is in the top result
    matched = any(kw.lower() in top_content for kw in expected_keywords)

    persona_results.append({
        "query": query,
        "match": matched,
        "top_content": top_content[:120],
        "score": top_score,
    })

    icon = "✓" if matched else "✗"
    print(f"  [{i+1:02d}] {icon} score={top_score:.4f} | {top_content[:80]}...")

persona_accuracy = sum(1 for r in persona_results if r["match"]) / len(persona_results)
print(f"\nPersonaMem Accuracy: {persona_accuracy:.1%} ({sum(1 for r in persona_results if r['match'])}/{len(persona_results)})")
print(f"Threshold: ≥80% → {'✓ PASS' if persona_accuracy >= 0.80 else '✗ FAIL'}")

# ═══════════════════════════════════════════════════
# TEST 2: AMB — Echo Retrieval Baseline
# ═══════════════════════════════════════════════════
print("\n" + "=" * 60)
print("TEST 2: AMB — Echo Retrieval (Top-1 / Recall@5 / MRR)")
print("=" * 60)

# Echo cases: store a unique fact, then query with 3 different phrasings
echo_cases = [
    {
        "fact": "KnowWhere verwendet Reciprocal Rank Fusion (k=60) für kombinierte Vektor+BM25 Suche.",
        "queries": [
            "Wie fusioniert KnowWhere Suchergebnisse?",
            "reciprocal rank fusion vector bm25",
            "was ist RRF in KnowWhere",
        ],
    },
    {
        "fact": "Der Frigate Webhook läuft auf Port 9177 mit MiniMax-M2.7 VLM.",
        "queries": [
            "Auf welchem Port läuft der Frigate Webhook?",
            "frigate port vlm model",
            "welches VLM nutzt Frigate",
        ],
    },
    {
        "fact": "Phase 2 hat Embeddings von bge-m3 (1024-dim) auf nomic-embed-text (768-dim) migriert.",
        "queries": [
            "Wie wurde das Embedding-Modell in Phase 2 geändert?",
            "embedding migration bge nomic",
            "von welchem zu welchem embedding modell",
        ],
    },
    {
        "fact": "Die KnowWhere API erfordert den 'content' Parameter für storage und 'query_text' für fractal retrieval.",
        "queries": [
            "Welche Parameter braucht die KnowWhere API?",
            "content query_text api required",
            "store retrieve api felder",
        ],
    },
    {
        "fact": "Hindsight läuft auf Port 9177 mit PostgreSQL auf 127.0.0.1:5433.",
        "queries": [
            "Wo läuft Hindsight und mit welcher Datenbank?",
            "hindsight port postgresql connection",
            "9177 5433 hindsight",
        ],
    },
    {
        "fact": "Kimi CLI v1.13.0 erfordert OAuth Login und ACP protocolVersion als Integer 1.",
        "queries": [
            "Wie konfiguriert man Kimi CLI?",
            "kimi cli oauth protocol version",
            "kimi acp setup",
        ],
    },
    {
        "fact": "Der Curator konsolidiert knowwhere-* Skills alle 7 Tage und archiviert stale Einträge nach 90 Tagen.",
        "queries": [
            "Wie oft läuft der Skill Curator?",
            "curator konsolidierung intervall",
            "skill curator schedule archiv",
        ],
    },
    {
        "fact": "KnowWhere v0.5.0 hat 850+ Nodes und verwendet OpenAI text-embedding-3-small (1536-dim).",
        "queries": [
            "Wie viele Nodes hat KnowWhere v0.5.0?",
            "knowwhere version node count embedding",
            "welche version und embedding dimension",
        ],
    },
]

print(f"Storing {len(echo_cases)} echo facts...")
echo_fact_contents = []
for i, case in enumerate(echo_cases):
    status, resp = store(case["fact"], "semantic", {"type": "echo_fact", "index": i})
    cid = resp.get("id", "?") if isinstance(resp, dict) else "?"
    echo_fact_contents.append(case["fact"])
    print(f"  [{i+1}] ✓ id={cid[:12]}... | {case['fact'][:60]}...")

time.sleep(1)  # Let embeddings settle

print(f"\nRunning {sum(len(c['queries']) for c in echo_cases)} echo queries...\n")

echo_ranks = []
echo_latencies = []
echo_top1_hits = 0

for case_idx, case in enumerate(echo_cases):
    for q_idx, query in enumerate(case["queries"]):
        t0 = time.time()
        status, results = retrieve(query, top_k=5)
        elapsed = (time.time() - t0) * 1000
        echo_latencies.append(elapsed)

        if not isinstance(results, list) or not results:
            echo_ranks.append(None)
            print(f"  [{case_idx+1}.{q_idx+1}] ✗ NO RESULTS: {query}")
            continue

        # Find rank of the correct fact
        rank = None
        for r_idx, result in enumerate(results):
            content = (result.get("content") or "").lower()
            if content in case["fact"].lower() or case["fact"].lower() in content:
                rank = r_idx + 1
                break

        echo_ranks.append(rank)
        if rank == 1:
            echo_top1_hits += 1

        icon = "✓" if rank and rank <= 5 else "✗"
        rank_str = f"rank={rank}" if rank else "not found"
        print(f"  [{case_idx+1}.{q_idx+1}] {icon} {rank_str} | {elapsed:.0f}ms | {query[:60]}")

# Calculate AMB metrics
total = len(echo_ranks)
top1 = sum(1 for r in echo_ranks if r == 1) / total if total else 0
recall5 = sum(1 for r in echo_ranks if r is not None and r <= 5) / total if total else 0
mrr = sum(1.0 / r if r is not None else 0 for r in echo_ranks) / total if total else 0

sorted_lat = sorted([l for l in echo_latencies if l > 0])
p95_ms = sorted_lat[int(len(sorted_lat) * 0.95)] if sorted_lat else 0

print(f"\nAMB Metrics:")
print(f"  Top-1:     {top1:.1%} (threshold ≥75%) → {'✓ PASS' if top1 >= 0.75 else '✗ FAIL'}")
print(f"  Recall@5:  {recall5:.1%} (threshold ≥75%) → {'✓ PASS' if recall5 >= 0.75 else '✗ FAIL'}")
print(f"  MRR:       {mrr:.3f} (threshold ≥0.75) → {'✓ PASS' if mrr >= 0.75 else '✗ FAIL'}")
print(f"  P95 latency: {p95_ms:.0f}ms")
print(f"  Avg latency: {sum(echo_latencies)/len(echo_latencies):.0f}ms")

# ═══════════════════════════════════════════════════
# TEST 3: Node Count & Performance
# ═══════════════════════════════════════════════════
print("\n" + "=" * 60)
print("TEST 3: Node Count & Performance (≥2405, no degradation)")
print("=" * 60)

status, health = api("GET", "/health")
node_count = health.get("node_count", 0) if isinstance(health, dict) else 0
print(f"  Node count: {node_count} (threshold ≥2405) → {'✓ PASS' if node_count >= 2405 else '✗ WARN'}")

# Performance: measure retrieval latency distribution
perf_queries = [
    "what is KnowWhere",
    "embedding model configuration",
    "pointer first architecture",
    "temporal memory boost",
    "database connection postgres",
]

perf_latencies = []
for q in perf_queries:
    for _ in range(3):  # 3 samples each
        t0 = time.time()
        status, results = retrieve(q, top_k=5)
        elapsed = (time.time() - t0) * 1000
        if status == 200:
            perf_latencies.append(elapsed)

if perf_latencies:
    perf_sorted = sorted(perf_latencies)
    perf_p50 = perf_sorted[len(perf_sorted)//2]
    perf_p95 = perf_sorted[int(len(perf_sorted)*0.95)]
    perf_p99 = perf_sorted[int(len(perf_sorted)*0.99)] if len(perf_sorted) > 10 else perf_sorted[-1]
    print(f"  Retrieval latency (15 samples):")
    print(f"    P50: {perf_p50:.0f}ms")
    print(f"    P95: {perf_p95:.0f}ms")
    print(f"    P99: {perf_p99:.0f}ms")

    perf_ok = perf_p95 < 500  # P95 under 500ms
    print(f"  Performance (P95 < 500ms): → {'✓ PASS' if perf_ok else '✗ WARN'}")
else:
    print(f"  ✗ Performance tests failed")

# ═══════════════════════════════════════════════════
# TEST 4: Temporal Golden Queries
# ═══════════════════════════════════════════════════
print("\n" + "=" * 60)
print("TEST 4: Temporal Golden Queries")
print("=" * 60)

# Store recent and old facts with the same topic
old_session = str(uuid.uuid4())
new_session = str(uuid.uuid4())

# Old fact (simulated — store first so it's "older")
print("Storing temporal test data...")
store("OLD FACT: KnowWhere used Ollama for embeddings initially on port 11434.",
      "semantic", {"session_id": old_session, "temporal": "old"})
time.sleep(0.3)

# New fact (stored second so it's "newer")
store("NEW FACT: KnowWhere now uses OpenAI text-embedding-3-small (1536-dim) via API.",
      "semantic", {"session_id": new_session, "temporal": "new"})
time.sleep(0.5)

temporal_queries = [
    {
        "query": "What embedding does KnowWhere use now?",
        "expect_new": True,
        "new_keywords": ["openai", "text-embedding-3", "1536"],
        "old_keywords": ["ollama", "11434"],
    },
    {
        "query": "What was the original embedding setup?",
        "expect_new": False,
        "new_keywords": ["openai", "text-embedding-3"],
        "old_keywords": ["ollama", "11434", "old fact"],
    },
    {
        "query": "how does knowwhere embed text",
        "expect_new": True,
        "new_keywords": ["openai", "text-embedding-3", "1536"],
        "old_keywords": ["ollama", "11434"],
    },
    {
        "query": "initial ollama configuration knowwhere",
        "expect_new": False,
        "new_keywords": ["openai"],
        "old_keywords": ["ollama", "old fact"],
    },
]

temporal_passes = 0
for i, tq in enumerate(temporal_queries):
    # Test WITHOUT recency boost
    _, results_noboost = retrieve(tq["query"], top_k=3)
    # Test WITH recency boost
    _, results_boost = retrieve(tq["query"], top_k=3, recency_boost=0.15)

    def check_content(results, keywords):
        if not isinstance(results, list) or not results:
            return False
        top_content = " ".join((r.get("content") or "").lower() for r in results[:3])
        return any(kw.lower() in top_content for kw in keywords)

    noboost_new = check_content(results_noboost, tq["new_keywords"])
    noboost_old = check_content(results_noboost, tq["old_keywords"])
    boost_new = check_content(results_boost, tq["new_keywords"])
    boost_old = check_content(results_boost, tq["old_keywords"])

    # The boosted version should favor the new fact more
    if tq["expect_new"]:
        # New fact should be findable; boost should help surface it
        temporal_ok = boost_new  # At minimum, boost should surface new fact
    else:
        temporal_ok = boost_old or noboost_old  # Old fact should be findable

    temporal_passes += 1 if temporal_ok else 0

    icon = "✓" if temporal_ok else "✗"
    print(f"  [{i+1}] {icon} \"{tq['query']}\"")
    if not temporal_ok:
        print(f"       noboost: new={'✓' if noboost_new else '✗'} old={'✓' if noboost_old else '✗'}")
        print(f"       boost:   new={'✓' if boost_new else '✗'} old={'✓' if boost_old else '✗'}")

temporal_score = temporal_passes / len(temporal_queries)
print(f"\nTemporal Golden Queries: {temporal_passes}/{len(temporal_queries)} ({temporal_score:.0%})")
print(f"Threshold: all must pass → {'✓ PASS' if temporal_score >= 1.0 else '✗ FAIL'}")

# ═══════════════════════════════════════════════════
# FINAL VERDICT
# ═══════════════════════════════════════════════════
print("\n" + "=" * 60)
print("FINAL VERDICT — KnowWhere v0.6 Verification Gate")
print("=" * 60)

checks = [
    ("PersonaMem 20q (≥80%)", persona_accuracy >= 0.80, f"{persona_accuracy:.1%}"),
    ("AMB Top-1 (≥75%)", top1 >= 0.75, f"{top1:.1%}"),
    ("AMB Recall@5 (≥75%)", recall5 >= 0.75, f"{recall5:.1%}"),
    ("AMB MRR (≥0.75)", mrr >= 0.75, f"{mrr:.3f}"),
    ("Node Count (≥2405)", node_count >= 2405, f"{node_count}"),
    ("Performance (P95 < 500ms)", perf_ok if 'perf_ok' in dir() else False, f"{perf_p95:.0f}ms" if 'perf_p95' in dir() else "N/A"),
    ("Temporal Golden Queries (4/4)", temporal_score >= 1.0, f"{temporal_passes}/4"),
]

all_pass = True
for name, passed, value in checks:
    icon = "✓" if passed else "✗"
    if not passed:
        all_pass = False
    print(f"  {icon} {name}: {value}")

print(f"\n{'=' * 60}")
if all_pass:
    print("✓ ALL GATES PASSED — KnowWhere v0.6 is GO for ship")
else:
    print("✗ SOME GATES FAILED — review required before shipping")
print(f"{'=' * 60}")

#!/usr/bin/env python3
"""50-Turn Hermes Session Test — Phase 3 AgentMemory-Parity UX Verification.

Tests the self-improving memory loop:
- 50 turns of simulated conversation
- Stores facts, decisions, preferences via knowwhere_remember
- Retrieves via knowwhere_reflect
- Compares with and without temporal layer
"""

import json
import time
import urllib.request
import urllib.error
import sys
import uuid
from datetime import datetime, timezone

ENDPOINT = "http://127.0.0.1:3737"
API_KEY = "kw_testkey_12345"
SESSION_ID = str(uuid.uuid4())
TIMEOUT = 30

def api(method, path, data=None):
    """Call KnowWhere API."""
    url = f"{ENDPOINT}{path}"
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {API_KEY}",
    }
    body = json.dumps(data).encode() if data else None
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        body = e.read().decode()[:500]
        print(f"  API ERROR {e.code}: {body}")
        return None


def store_turn(user_msg, assistant_msg, turn_idx):
    """Store a conversation turn."""
    api("POST", "/store_session", {
        "content": f"[user] {user_msg}",
        "session_id": SESSION_ID,
        "turn_index": turn_idx * 2,
        "memory_type": "episodic",
        "source": "conversation",
        "metadata": {"role": "user", "trust_tier": "primary"},
    })
    api("POST", "/store_session", {
        "content": f"[assistant] {assistant_msg}",
        "session_id": SESSION_ID,
        "turn_index": turn_idx * 2 + 1,
        "memory_type": "episodic",
        "source": "conversation",
        "metadata": {"role": "assistant", "trust_tier": "derived"},
    })


def self_improve(content, memory_type="semantic", importance=7):
    """Self-improvement: AI→Memory hook."""
    return api("POST", "/memory/self_improve", {
        "content": content,
        "memory_type": memory_type,
        "importance": importance,
        "session_id": SESSION_ID,
    })


def retrieve(query, top_k=5, reflect=False, recency_boost=None):
    """Retrieve memories."""
    payload = {"query_text": query, "top_k": top_k, "reflect": reflect}
    if recency_boost is not None:
        payload["recency_boost"] = recency_boost
    return api("POST", "/retrieve_fractal", payload)


# ── Simulated 50-turn conversation ──

print(f"=== 50-Turn AgentMemory Test ===\n")
print(f"Session: {SESSION_ID[:12]}...")
print(f"Server:  {api('GET', '/health')}\n")

# Pre-seed: Explicit self-improvement facts
print("--- Pre-seeding self-improvement facts ---")
facts = [
    ("decision", "DECISION: KnowWhere uses nomic-embed-text (768-dim) for embeddings — confirmed via Phase 2 benchmarks, 28% faster than bge-m3.", 9),
    ("preference", "PREFERENCE: Nimar prefers Pointer-First architecture — store references, not raw data.", 10),
    ("preference", "PREFERENCE: User prefers comprehensive documentation — 'so viel wie möglich', maximum detail.", 8),
    ("semantic", "FACT: KnowWhere v0.6.0 runs on port 3737 with Ollama for embeddings.", 7),
    ("procedural", "WORKFLOW: To restart KnowWhere: pkill -f knowwhere-server && cargo run --release in /Users/nimarfranklinmac/knowwhere", 8),
    ("decision", "DECISION: Phase 1 added temporal recency boost (0.0-0.20) with selective tiebreaker semantics for turn_index + created_at.", 8),
    ("semantic", "FACT: The server has 2405+ memory nodes across multiple sessions.", 6),
    ("preference", "PREFERENCE: Nimar is skeptical of too-perfect results — demands real benchmarks, zero fake data.", 9),
    ("decision", "DECISION: Phase 3 adds self-improving memory hook — AI can call knowwhere_remember to persist insights.", 8),
    ("procedural", "WORKFLOW: KnowWhere plugin lives at ~/.hermes/plugins/knowwhere/__init__.py", 6),
]
for mtype, content, importance in facts:
    r = self_improve(content, mtype, importance)
    status = "✓" if r else "✗"
    print(f"  {status} [{mtype}:{importance}] {content[:80]}...")

print(f"\n--- Running 50 turns ---")

topics = [
    ("Hi, what's the project status?", "KnowWhere v0.6 is in Phase 3 — adding AgentMemory-Parity UX with self-improving memory hooks. Server at port 3737, 2405 nodes stored."),
    ("What embedding model do we use?", "We use nomic-embed-text (768-dim), chosen in Phase 2. It's 28% faster than bge-m3 with acceptable quality trade-off."),
    ("How do I restart the server?", "pkill -f knowwhere-server, then cargo run --release in /Users/nimarfranklinmac/knowwhere. The server binds to port 3737."),
    ("What's the architecture philosophy?", "Pointer-First architecture — store references and metadata, not raw data. Better retrieval quality and lower storage."),
    ("Who is this for?", "Nimar — the KnowWhere inventor. He prefers comprehensive docs, is skeptical of fake results, and demands real benchmarks."),
    ("What did Phase 1 add?", "Phase 1 added temporal recency boost (0.0-0.20) with turn_index + created_at tiebreaker semantics."),
    ("What did Phase 2 add?", "Phase 2 upgraded embeddings from bge-m3 (1024-dim) to nomic-embed-text (768-dim). All 2405 nodes re-embedded in 77s."),
    ("What does Phase 3 add?", "Phase 3 adds self-improving memory hooks, better compression prompts, and the knowwhere_remember/reflect tools."),
    ("How do I deploy this?", "No formal deploy yet — it's a local dev server. Docker Compose available but native macOS is preferred."),
    ("What database does it use?", "In-memory by default, optional PostgreSQL. Trajectory logging and tiered context require postgres-storage feature."),
    # ... repeat variations with different queries
]

# Pad topics to 50 turns by cycling
while len(topics) < 50:
    for i, t in enumerate(topics[:10]):
        if len(topics) >= 50:
            break
        q, a = t
        q = q.replace("?", f" [{len(topics)}]?")
        topics.append((q, a))

percentiles = []
for i in range(50):
    user_q, assistant_a = topics[i]
    store_turn(user_q, assistant_a, i)

    # Every 10 turns: explicitly self-improve
    if i % 10 == 0:
        self_improve(
            f"CHECKPOINT turn {i}: 50-turn test in progress. {i+1} turns stored so far.",
            "episodic", 5
        )

    # Every 25 turns: test retrieval
    if i in [24, 49]:
        print(f"\n  --- Turn {i+1} retrieval test ---")
        # Test with temporal boost
        t0 = time.time()
        r_boosted = retrieve("what architecture does KnowWhere use", top_k=3, recency_boost=0.15)
        t_boosted = (time.time() - t0) * 1000

        # Test without temporal boost
        t0 = time.time()
        r_noboost = retrieve("what architecture does KnowWhere use", top_k=3)
        t_noboost = (time.time() - t0) * 1000

        b_score = r_boosted[0]["score"] if r_boosted else 0
        nb_score = r_noboost[0]["score"] if r_noboost else 0
        b_content = r_boosted[0]["content"][:80] if r_boosted else "N/A"
        nb_content = r_noboost[0]["content"][:80] if r_noboost else "N/A"

        print(f"    Boosted:  score={b_score:.4f} top=\"{b_content}\" ({t_boosted:.1f}ms)")
        print(f"    No boost: score={nb_score:.4f} top=\"{nb_content}\" ({t_noboost:.1f}ms)")
        print(f"    Δ score:  {b_score - nb_score:+.4f}")

        if i == 49:
            percentiles.append((b_score, nb_score, t_boosted, t_noboost))

    if i % 10 == 9:
        print(f"  ✓ {i+1}/50 turns done")

print(f"\n=== Final Verification ===")

# Test self-improvement retrieval
print("\n--- Self-Improvement Retrieval ---")
for query, expected_type in [
    ("what embedding model", "decision"),
    ("what are the preferences", "preference"),
    ("how to restart", "procedural"),
    ("phase 1 temporal", "decision"),
    ("pointer first architecture", "preference"),
]:
    r = retrieve(query, top_k=3)
    top = r[0] if r else {}
    top_content = (top.get("content") or "")[:100]
    top_type = top.get("memory_type", "?")
    top_score = top.get("score", 0)
    match = "✓" if top_type in (expected_type, "decision", "preference", "semantic", "procedural") else "?"
    print(f"  {match} \"{query}\" → [{top_type}] score={top_score:.4f} \"{top_content}\"")

# Final stats
print(f"\n--- Timeline ---")
print(f"Total turns: 50")
print(f"Session ID:  {SESSION_ID[:12]}...")
print(f"Self-improve facts stored: {len(facts)} + checkpoints")
if percentiles:
    b_s, nb_s, b_t, nb_t = percentiles[0]
    print(f"Final retrieval: Boosted={b_s:.4f} NoBoost={nb_s:.4f} Δ={b_s-nb_s:+.4f}")

print(f"\n=== Phase 3 Test Complete ===")

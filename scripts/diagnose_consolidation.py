#!/usr/bin/env python3
"""Diagnostic: Test consolidation pipeline end-to-end without the server."""

import requests
import json
import sys

BASE = "http://localhost:3737"
HEADERS = {"Authorization": "Bearer kw_testkey_12345"}

# ── Step 1: Health check ──
print("═══ STEP 1: Health Check ═══")
r = requests.get(f"{BASE}/health")
print(f"Health: {r.json()}")

# ── Step 2: Dream status ──
print("\n═══ STEP 2: Dream Status ═══")
r = requests.get(f"{BASE}/dream/status", headers=HEADERS)
print(json.dumps(r.json(), indent=2))

# ── Step 3: Test Ollama directly ──
print("\n═══ STEP 3: Test Ollama Summarization ═══")
ollama_r = requests.post("http://localhost:11434/api/chat", json={
    "model": "llama3.2",
    "messages": [
        {"role": "system", "content": "You are a concise summarizer. Output exactly one sentence."},
        {"role": "user", "content": "Summarize in ONE sentence (≤20 words). If any decisions were made, state the decision AND the reason. No preamble.\n\nWe decided to use Rust instead of Python for the KnowWhere server because we needed type safety and zero-cost abstractions. The benchmark showed Rust was 10x faster for embedding operations."}
    ],
    "stream": False,
    "options": {"temperature": 0.0, "seed": 42, "num_predict": 50}
}, timeout=30)
result = ollama_r.json()
summary = result.get("message", {}).get("content", "NO_RESPONSE")
print(f"Summary: {summary}")
print(f"Model: {result.get('model', 'unknown')}")

# ── Step 4: Find candidate nodes ──
print("\n═══ STEP 4: Identify Candidate Nodes ═══")
r = requests.get(f"{BASE}/nodes/recent?limit=50", headers=HEADERS)
nodes = r.json()
candidates = []
for node in nodes:
    content = node.get("content") or ""
    tier = node.get("context_tier", "unknown")
    pid = node.get("parent_tier_id")
    imp = node.get("importance", 0)

    if (tier == "raw" and pid is None and imp >= 3 and len(content) > 500):
        candidates.append(node)

print(f"Found {len(candidates)} candidates in recent 50 nodes")
for c in candidates[:3]:
    content = c.get("content", "")
    print(f"  ID: {c['id'][:16]}... | len={len(content)} | type={c.get('memory_type','?')} | importance={c.get('importance')}")
    print(f"    First 100 chars: {content[:100]}...")
    print()

# ── Step 5: Check if any raw-tier nodes exist at all ──
print("\n═══ STEP 5: Raw-tier Node Census ═══")
# We can't easily query all 15k nodes, but let's look at recent nodes
raw_count = sum(1 for n in nodes if n.get("context_tier") == "raw")
overview_count = sum(1 for n in nodes if n.get("context_tier") == "overview")
summary_count = sum(1 for n in nodes if n.get("context_tier") == "summary")
print(f"Recent 50 nodes: {raw_count} raw, {overview_count} overview, {summary_count} summary")

# Check how many have content > 500
long_content = sum(1 for n in nodes if len(n.get("content") or "") > 500)
print(f"Nodes with content > 500 chars: {long_content}")

# ── Step 6: Try to retrieve a raw node with long content ──
print("\n═══ STEP 6: Sample raw node retrieval ═══")
raw_nodes = [n for n in nodes if n.get("context_tier") == "raw"]
if raw_nodes:
    sample_id = raw_nodes[0]["id"]
    r = requests.get(f"{BASE}/retrieve/{sample_id}", headers=HEADERS)
    node_data = r.json()
    content_preview = (node_data.get("content") or "")[:200]
    print(f"Sample ID: {sample_id}")
    print(f"Content len: {len(node_data.get('content') or '')}")
    print(f"Tier: {node_data.get('context_tier')}")
    print(f"Type: {node_data.get('memory_type')}")
    print(f"Content preview: {content_preview}...")
else:
    print("NO raw-tier nodes in recent 50!")

print("\n═══ DIAGNOSTIC COMPLETE ═══")

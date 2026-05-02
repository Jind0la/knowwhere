#!/usr/bin/env python3
"""Diagnostic: Single-case benchmark with per-phase timing breakdown."""

import json
import sys
import time
import os
import urllib.request
import urllib.error

# --- Config ---
BASE_URL = os.environ.get("KNOWWHERE_BENCH_BASE_URL", "http://localhost:3737")
API_KEY = os.environ.get("KNOWWHERE_BENCH_API_KEY")
if not API_KEY:
    print("ERROR: KNOWWHERE_BENCH_API_KEY environment variable required", file=sys.stderr)
    sys.exit(1)
DATASET = os.environ.get("KNOWWHERE_LONGMEMEVAL_DATASET",
    os.path.expanduser("~/knowwhere/benchmarks/hf/fixtures/longmemeval_oracle.json"))
CASE_INDEX = int(os.environ.get("DIAG_CASE_INDEX", "0"))  # first case

def api_post(endpoint, payload):
    """POST JSON to KnowWhere, return (status, body, elapsed_ms)."""
    url = f"{BASE_URL}/{endpoint}"
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Authorization", f"Bearer {API_KEY}")
    req.add_header("Content-Type", "application/json")
    t0 = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            elapsed = (time.monotonic() - t0) * 1000
            body = json.loads(resp.read().decode())
            return resp.status, body, elapsed
    except urllib.error.HTTPError as e:
        elapsed = (time.monotonic() - t0) * 1000
        body = e.read().decode()[:500]
        return e.code, body, elapsed
    except Exception as e:
        elapsed = (time.monotonic() - t0) * 1000
        return 0, str(e), elapsed

def session_text(session):
    """Convert session (list of turns) to text."""
    lines = []
    for turn in session:
        role = turn.get("role", "")
        content = turn.get("content", "")
        if role and content:
            lines.append(f"{role}: {content}")
    return "\n".join(lines) if lines else str(session)

def chunk_count(text):
    """Estimate chunk count (same logic as chunk_into_rounds with min_round_chars=80)."""
    role_prefixes = ["user:", "assistant:", "human:", "ai:", "User:", "Assistant:", "Human:", "AI:"]
    lines = text.split("\n")
    rounds = []
    current = []
    for line in lines:
        trimmed = line.strip()
        is_role = any(trimmed.startswith(p) for p in role_prefixes)
        if is_role and current:
            c = " ".join(current).strip()
            if c:
                rounds.append(c)
            current = []
        current.append(line)
    if current:
        c = " ".join(current).strip()
        if c:
            rounds.append(c)
    if len(rounds) <= 1:
        return 1
    # Merge tiny rounds (< 80 chars)
    merged = []
    for r in rounds:
        if merged and len(merged[-1]) < 80:
            merged[-1] += "\n" + r
        else:
            merged.append(r)
    return max(1, len(merged))

def main():
    print("=" * 70)
    print("  KnowWhere Single-Case Diagnostic")
    print("=" * 70)

    # Load dataset
    with open(DATASET) as f:
        cases = json.load(f)
    if CASE_INDEX >= len(cases):
        print(f"ERROR: CASE_INDEX={CASE_INDEX} out of range ({len(cases)} cases)")
        sys.exit(1)
    case = cases[CASE_INDEX]
    print(f"\nCase:  {case['question_id']}")
    print(f"Question: {case['question'][:120]}...")
    print(f"Sessions: {len(case['haystack_sessions'])}")
    print(f"Answer IDs: {case['answer_session_ids']}")

    # --- Phase 0: Analyze sessions ---
    print("\n" + "-" * 50)
    print("Phase 0: Session Analysis")
    print("-" * 50)
    total_chars = 0
    total_est_chunks = 0
    for i, sess in enumerate(case['haystack_sessions']):
        text = session_text(sess)
        chars = len(text)
        chunks = chunk_count(text)
        total_chars += chars
        total_est_chunks += chunks
        sid = case.get('haystack_session_ids', [])[i] if i < len(case.get('haystack_session_ids', [])) else f"session_{i}"
        print(f"  Session {i}: {chars:,} chars, ~{chunks} chunks, id={sid}")
    print(f"  TOTAL: {total_chars:,} chars, ~{total_est_chunks} estimated chunks")

    # --- Phase 1: Build session payloads ---
    print("\n" + "-" * 50)
    print("Phase 1: store_session_batch")
    print("-" * 50)

    sessions_payload = []
    for i, sess in enumerate(case['haystack_sessions']):
        text = session_text(sess)
        sid_val = case.get('haystack_session_ids', [])
        sid = sid_val[i] if i < len(sid_val) else f"session_{i}"
        sessions_payload.append({
            "content": text,
            "metadata": {
                "benchmark": "diagnostic",
                "question_id": case['question_id'],
                "session_id": sid,
            },
            "memory_type": "episodic",
            "source": "conversation",
        })

    batch_payload = {"sessions": sessions_payload}
    status, body, elapsed_ms = api_post("store_session_batch", batch_payload)
    print(f"  Status: {status}")
    print(f"  Elapsed: {elapsed_ms:,.0f} ms ({elapsed_ms/1000:.1f}s)")
    if status == 201:
        total_chunks_reported = body.get("total_chunks", "?")
        total_sessions = body.get("total_sessions", "?")
        print(f"  Server chunks: {total_chunks_reported}")
        print(f"  Server sessions: {total_sessions}")
        # Collect all IDs for later deletion
        all_ids = []
        for result in body.get("results", []):
            node_id = result.get("id")
            if node_id:
                all_ids.append(node_id)
            for cid in result.get("chunk_ids", []):
                if cid != node_id:
                    all_ids.append(cid)
        print(f"  Total nodes created: {len(all_ids)}")
    else:
        print(f"  Error body: {str(body)[:300]}")
        all_ids = []

    store_elapsed = elapsed_ms

    # --- Phase 2: retrieve_fractal ---
    print("\n" + "-" * 50)
    print("Phase 2: retrieve_fractal")
    print("-" * 50)

    retrieve_payload = {
        "query_text": case['question'],
        "top_k": 20,
        "max_depth": 3,
        "governance_enabled": True,
        "retrieval_profile": "full-fidelity",
        "include_debug": False,
    }
    status, body, elapsed_ms = api_post("retrieve_fractal", retrieve_payload)
    print(f"  Status: {status}")
    print(f"  Elapsed: {elapsed_ms:,.0f} ms ({elapsed_ms/1000:.1f}s)")
    if status == 200:
        hit_count = len(body) if isinstance(body, list) else 0
        print(f"  Results returned: {hit_count}")
        if hit_count > 0:
            top_hit = body[0]
            hit_meta = top_hit.get("metadata", {})
            hit_sid = hit_meta.get("session_id", "N/A")
            hit_score = top_hit.get("score", "N/A")
            print(f"  Top hit: sid={hit_sid}, score={hit_score}")
    else:
        print(f"  Error: {str(body)[:300]}")

    retrieve_elapsed = elapsed_ms

    # --- Phase 3: batch_delete ---
    print("\n" + "-" * 50)
    print("Phase 3: batch_delete")
    print("-" * 50)

    if all_ids:
        status, body, elapsed_ms = api_post("nodes/batch_delete", {"ids": all_ids})
        print(f"  Status: {status}")
        print(f"  Elapsed: {elapsed_ms:,.0f} ms ({elapsed_ms/1000:.1f}s)")
        print(f"  Deleted: {body.get('deleted', '?')}")
        print(f"  Not found: {body.get('not_found', '?')}")
    else:
        print("  SKIPPED (no nodes to delete)")
        elapsed_ms = 0

    delete_elapsed = elapsed_ms

    # --- Summary ---
    total_elapsed = store_elapsed + retrieve_elapsed + delete_elapsed
    print("\n" + "=" * 70)
    print("  TIMING SUMMARY")
    print("=" * 70)
    print(f"  store_session_batch:  {store_elapsed:>8,.0f} ms  ({store_elapsed/total_elapsed*100:.0f}%)")
    print(f"  retrieve_fractal:     {retrieve_elapsed:>8,.0f} ms  ({retrieve_elapsed/total_elapsed*100:.0f}%)")
    print(f"  batch_delete:         {delete_elapsed:>8,.0f} ms  ({delete_elapsed/total_elapsed*100:.0f}%)")
    print(f"  ─────────────────────────────────────")
    print(f"  TOTAL:                {total_elapsed:>8,.0f} ms  ({total_elapsed/1000:.1f}s)")
    print(f"\n  Estimated for 500 cases: {total_elapsed*500/1000/60:,.0f} minutes")

    # Also check what the server thinks about chunking
    print(f"\n  Client-estimated chunks: {total_est_chunks}")
    print(f"  Chunks-per-Ollama-call (if MAX_BATCH=8): {total_est_chunks/8:.0f} HTTP calls")


if __name__ == "__main__":
    main()

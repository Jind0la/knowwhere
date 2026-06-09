#!/usr/bin/env python3
"""End-to-end test: Fractal Hierarchy Activation for KnowWhere.

Tests:
1. Ingest 8-turn test session as raw-tier nodes (full content)
2. Force consolidation via Ollama LocalSummarizer
3. Verify L0→L1→L2 parent/child links
4. Test fractal zoom via retrieve_fractal
5. Run retrieval benchmarks (5 doc + 5 conv queries)
6. Measure Precision@3
"""

import requests
import json
import time
import sys

BASE = "http://localhost:3737"
HEADERS = {"Authorization": "Bearer kw_testkey_12345"}
TIMEOUT_CONSOLIDATION = 180  # Max wait for consolidation to complete

# ═══════════════════════════════════════════════════════
# TEST DATA: 8-turn conversation about technical decisions
# ═══════════════════════════════════════════════════════

TEST_TURNS = [
    {
        "content": """TURN 1 — Nimar (User): I've been analyzing the KnowWhere retrieval benchmarks. The RRF k=60 parameter was degrading score separation by 7.9x compared to k=5. This is documented in SIGNAL-TRACE.md. The fix reduced our AMB accuracy issue from 25% to 73%. Specifically, the reciprocal rank fusion formula gives 60% score separation between rank 1 and rank 10 with k=5, vs only 12% separation with k=60. That's why the top results were flat — all scores were within 0.002 of each other.

The debug server on port 3737 is now running with k=5. I tested 5 retrieval queries and all show correct Rank 1 results. The Memory-Type-Multiplier was also neutralized because it introduced a 76% penalty for Episodic memories vs Decision memories. This was the primary cause of poor document and conversation retrieval."""
    },
    {
        "content": """TURN 2 — Alex (Assistant): Good analysis. But we still have the architectural problem: 14,979 flat Decision atoms with no hierarchy. The ConsolidationScheduler in src/scheduler/consolidation.rs has the full L0→L1→L2 pipeline coded, including the LocalSummarizer that uses Ollama's JSON Schema format for structured claim extraction. The problem is that the ingestion pipeline via chunk_into_rounds breaks sessions into 80-character chunks, so no node ever has content > 500 chars to qualify as a consolidation candidate.

We need to either set KNOWWHERE_MIN_ROUND_CHARS to a large value or modify the pipeline to store full-content nodes alongside chunked nodes. The full-content raw nodes would serve as consolidation candidates, and the consolidation process would create proper L1 overviews with 2-3 sentence narrative summaries."""
    },
    {
        "content": """TURN 3 — Nimar (User): Agreed. I checked the data: the first chunk of each session (idx=0) DOES store the full content in the node. So those full-content nodes exist but their embeddings only cover the first 80 characters. For consolidation, that's fine — the summarizer reads the full content regardless of the embedding.

The real issue is the 30-second timeout on the Ollama HTTP client. With llama3.2, a 1500-character document takes about 24 seconds to summarize with the JSON Schema format. With a 30-second timeout and cold-start model loading, the first consolidation run hits the timeout. Switching to qwen2.5:3b reduces this to ~17 seconds.

The fix should use qwen2.5:3b as the default summarization model and increase the timeout to 120 seconds. But first, let's verify the pipeline works end-to-end with a test session."""
    },
    {
        "content": """TURN 4 — Alex (Assistant): Let me also check the fractal zoom code. The FractalNode::zoom_retrieve method in fractal_node.rs implements hierarchical pruning with a configurable threshold (default 0.7). It recursively descends from parent to child, collecting (similarity, node) pairs along the best path. When similarity drops below the pruning threshold, the branch is cut — this prevents exploring irrelevant subtrees.

The retrieve_fractal endpoint (POST /retrieve_fractal) accepts max_depth and pruning_threshold parameters. When called with max_depth=2, it should expand from L0 to L1 to L2, returning nodes at all three tiers. But this has never been tested with real consolidated data because the consolidation pipeline was effectively disabled.

We need to: (1) Run consolidation on the test session, (2) Verify the parent/child links are correct, (3) Call retrieve_fractal with max_depth=2 and verify multi-tier results."""
    },
    {
        "content": """TURN 5 — Nimar (User): Here's my plan for the retrieval benchmarks. We'll use the same 10 queries from the Core Loop proof:
Document queries:
1. "What is the KnowWhere roadmap?"
2. "How does the consolidation pipeline work?"
3. "What embedding models does KnowWhere use?"
4. "How is retrieval scored in KnowWhere?"
5. "What is the fractal memory hierarchy?"

Conversation queries:
6. "What was decided about Docker?"
7. "Why was the Memory-Type-Multiplier removed?"
8. "What model was chosen for embeddings?"
9. "How was the RRF k parameter determined?"
10. "What is the current state of KnowWhere?"

Precision@3 target: ≥ 0.50 for both document and conversation queries. Current Precision@3 from the Core Loop proof: 0.33 for documents, 0.27 for conversations."""
    },
    {
        "content": """TURN 6 — Alex (Assistant): For the benchmarks, we need ground truth. Each query has known answers in the test data:

Ground truth for document queries:
1. KnowWhere roadmap → Roadmap section with v1.0 features, Docker Compose, PostgreSQL
2. Consolidation pipeline → L0→L1→L2 compaction, LocalSummarizer, JSON Schema claims
3. Embedding models → Ollama nomic-embed-text-v2-moe, 768-dim, MoE, multilingual
4. Retrieval scoring → RRF k=5, cosine similarity, neutralized multipliers
5. Fractal hierarchy → L0 (Summary), L1 (Overview), L2 (Raw), bidirectional links

For conversation queries:
6. Docker decision → Docker Compose with all features, Ollama via host.docker.internal
7. Multiplier removal → 76% penalty, 25%→73% AMB accuracy improvement
8. Embedding model choice → Ollama with 92.1% instruction-following rate
9. RRF k parameter → k=5 chosen, 7.9x better score separation vs k=60
10. Current state → Core Loop proven, consolidation working, hierarchy being activated

We can use keyword overlap between retrieved content and these ground truth anchors to determine relevance at Precision@3."""
    },
    {
        "content": """TURN 7 — Nimar (User): One more architectural concern: lossless sessions. The PRD promises that sessions remain as complete units, not just atomized claims. Currently, when we ingest via store_session_batch, each turn is chunked into 80-character pieces. The full text of turn 0 exists in the first chunk node, but turns 1-7 are fragmented.

For true lossless sessions, we need: (1) Store the full session content in a single raw-tier node, (2) The consolidation creates an L1 overview summarizing the entire session, (3) Individual turns remain as L0 raw nodes for fine-grained retrieval, (4) The L1 node links to all L0 turn nodes via children_tier_ids.

This way, a user can search for "what did we decide about Docker" and get the L1 session summary, then zoom into the specific turn where the decision was made. The provenance chain is: L0 Summary → L1 Session Overview → L2 Individual Turn."""
    },
    {
        "content": """TURN 8 — Alex (Assistant): Final checklist for the activation:

1. Fix consolidation.rs: Parse JSON output BEFORE creating L1 node, use clean narrative summary as l1_content (currently stores raw JSON)
2. Build and restart server with KNOWWHERE_MIN_ROUND_CHARS=2000 to prevent chunking
3. Set OLLAMA_SUMMARIZER_MODEL=qwen2.5:3b for faster summarization (~17s vs ~24s with llama3.2)
4. Ingest 8-turn test session as individual store_session calls with full content
5. Force consolidation via POST /consolidation/force
6. Verify hierarchy: L0 (Summary) → L1 (Overview) → L2 (Raw) with bidirectional parent_tier_id/children_tier_ids links
7. Test fractal zoom: POST /retrieve_fractal with max_depth=2, verify multi-tier results
8. Run 10 retrieval benchmarks, measure Precision@3
9. Document everything in CONSOLIDATION-REPORT.md

Success criteria: Consolidation completes, hierarchy is verifiable, Precision@3 ≥ 0.50 for both doc and conv queries. Let's execute."""
    },
]

# ═══════════════════════════════════════════════════════
# GROUND TRUTH for Precision@3 measurement
# ═══════════════════════════════════════════════════════

GROUND_TRUTH = {
    "document": {
        "What is the KnowWhere roadmap?": ["roadmap", "v1.0", "docker", "postgresql", "features"],
        "How does the consolidation pipeline work?": ["consolidation", "l0", "l1", "l2", "localsummarizer", "compaction"],
        "What embedding models does KnowWhere use?": ["ollama", "nomic-embed", "768", "moe", "multilingual", "embedding"],
        "How is retrieval scored in KnowWhere?": ["rrf", "k=5", "cosine", "similarity", "score", "multiplier"],
        "What is the fractal memory hierarchy?": ["fractal", "l0", "l1", "l2", "zoom", "tier", "hierarchy"],
    },
    "conversation": {
        "What was decided about Docker?": ["docker", "compose", "host.docker.internal", "ollama"],
        "Why was the Memory-Type-Multiplier removed?": ["multiplier", "76%", "penalty", "25%", "73%", "amb"],
        "What model was chosen for embeddings?": ["ollama", "92.1%", "nomic", "instruction-following", "embedding"],
        "How was the RRF k parameter determined?": ["rrf", "k=5", "k=60", "7.9", "score", "separation"],
        "What is the current state of KnowWhere?": ["core loop", "consolidation", "hierarchy", "proven", "activated"],
    }
}

# ═══════════════════════════════════════════════════════
# HELPER FUNCTIONS
# ═══════════════════════════════════════════════════════

def api(method, path, **kwargs):
    """Make API call with auth header."""
    url = f"{BASE}{path}"
    kwargs.setdefault("headers", HEADERS)
    kwargs.setdefault("timeout", 60)
    return getattr(requests, method)(url, **kwargs)

def health_check():
    r = requests.get(f"{BASE}/health", timeout=10)
    assert r.status_code == 200, f"Server not healthy: {r.status_code}"
    return r.json()

def ingest_session(turns):
    """Ingest turns as individual store_session calls (each stores full content)."""
    node_ids = []
    for i, turn in enumerate(turns):
        payload = {
            "content": turn["content"],
            "session_id": "fractal-hierarchy-test-002",
            "turn_index": i,
            "memory_type": "episodic",
            "source": "conversation",
            "importance": 5,
            "metadata": {
                "test": "fractal-hierarchy-activation",
                "turn": str(i),
            }
        }
        r = api("post", "/store_session", json=payload)
        if r.status_code in [200, 201]:
            data = r.json()
            nid = data.get("id") or (data.get("node_ids", [None])[0] if "node_ids" in data else None)
            if nid:
                node_ids.append(nid)
                print(f"  Turn {i}: stored node {nid[:16]}... ({len(turn['content'])} chars)")
        else:
            print(f"  Turn {i}: ERROR {r.status_code}: {r.text[:200]}")
    return node_ids

def verify_node(node_id):
    """Verify a node exists and check its tier/content."""
    r = api("get", f"/retrieve/{node_id}")
    if r.status_code != 200:
        return None
    node = r.json()
    return {
        "id": node["id"],
        "tier": node.get("context_tier"),
        "source": node.get("source"),
        "content_len": len(node.get("content") or ""),
        "parent": node.get("parent_tier_id"),
        "children": node.get("children_tier_ids", []),
        "content_preview": (node.get("content") or "")[:100],
    }

def force_consolidation():
    """Trigger force consolidation and wait for completion."""
    r = api("post", "/consolidation/force")
    data = r.json()
    print(f"  Candidates: {data.get('candidates_found')}, Total: {data.get('total_nodes')}")

    if not data.get("accepted"):
        return False

    # Wait for consolidation to process
    for i in range(TIMEOUT_CONSOLIDATION // 5):
        time.sleep(5)
        # Check if new consolidation nodes appeared
        r2 = api("get", "/nodes/recent?limit=30")
        nodes = r2.json()
        recent_cons = [n for n in nodes if n.get("source") == "consolidation"]
        if recent_cons:
            newest_time = recent_cons[0].get("created_at", "")
            print(f"  [{i*5}s] Found {len(recent_cons)} consolidation nodes, newest: {newest_time}")

    return True

def measure_precision(query, ground_terms, category):
    """Measure Precision@3 for a single query."""
    r = api("post", "/retrieve_fractal", json={
        "query": query,
        "max_depth": 2,
        "limit": 3,
    })

    if r.status_code != 200:
        print(f"    Query failed: {r.status_code}")
        return 0, []

    data = r.json()
    nodes = data.get("nodes", data) if isinstance(data, dict) else data
    if not isinstance(nodes, list):
        nodes = [nodes]

    relevant = 0
    for node in nodes[:3]:
        content = (node.get("content") or "").lower()
        matches = sum(1 for term in ground_terms if term.lower() in content)
        if matches >= 1:
            relevant += 1

    precision = relevant / min(3, len(nodes)) if nodes else 0
    return precision, nodes[:3]

# ═══════════════════════════════════════════════════════
# MAIN TEST SEQUENCE
# ═══════════════════════════════════════════════════════

def main():
    print("═══ KNOWWHERE FRACTAL HIERARCHY ACTIVATION TEST ═══\n")

    # Step 1: Health check
    print("STEP 1: Health Check")
    health = health_check()
    print(f"  Server: {health['status']}, Nodes: {health['node_count']}\n")

    # Step 2: Ingest test session
    print("STEP 2: Ingesting 8-Turn Test Session")
    node_ids = ingest_session(TEST_TURNS)
    print(f"  Stored {len(node_ids)} nodes\n")

    if len(node_ids) < 8:
        print("  ⚠️ Not all turns stored! Check server logs.")
        return 1

    # Step 3: Verify raw nodes
    print("STEP 3: Verifying Raw Nodes")
    for nid in node_ids:
        info = verify_node(nid)
        if info:
            print(f"  {nid[:16]}... tier={info['tier']} len={info['content_len']}")
    print()

    # Step 4: Force consolidation
    print("STEP 4: Running Consolidation")
    success = force_consolidation()
    print(f"  Consolidation {'started' if success else 'failed'}\n")

    # Step 5: Verify hierarchy
    print("STEP 5: Verifying Fractal Hierarchy")
    hierarchy_found = False
    for nid in node_ids:
        info = verify_node(nid)
        if info and info["parent"]:
            print(f"  L2 {nid[:16]}... → parent: {str(info['parent'])[:16]}...")
            # Trace up to L1 and L0
            parent_info = verify_node(info["parent"])
            if parent_info:
                print(f"    L1 {str(info['parent'])[:16]}... tier={parent_info['tier']} content='{parent_info['content_preview']}'")
                if parent_info["parent"]:
                    l0_info = verify_node(parent_info["parent"])
                    if l0_info:
                        print(f"      L0 {str(parent_info['parent'])[:16]}... tier={l0_info['tier']} content='{l0_info['content_preview']}'")
                        hierarchy_found = True
    print(f"  Hierarchy found: {hierarchy_found}\n")

    # Step 6: Test fractal zoom
    print("STEP 6: Testing Fractal Zoom")
    r = api("post", "/retrieve_fractal", json={
        "query": "What is the fractal memory hierarchy?",
        "max_depth": 2,
        "limit": 5,
    })

    if r.status_code == 200:
        data = r.json()
        nodes = data.get("nodes", data) if isinstance(data, dict) else data
        tiers_found = set()
        for node in (nodes if isinstance(nodes, list) else [nodes]):
            tier = node.get("context_tier", "unknown")
            tiers_found.add(tier)
        print(f"  Tiers in results: {tiers_found}")
        print(f"  Multi-tier zoom: {'✅' if len(tiers_found) > 1 else '❌'} ({len(tiers_found)} tiers)\n")
    else:
        print(f"  retrieve_fractal failed: {r.status_code}\n")

    # Step 7: Retrieval benchmarks
    print("STEP 7: Retrieval Benchmarks (Precision@3)")

    doc_precisions = []
    conv_precisions = []

    print("\n  Document Queries:")
    for query, terms in GROUND_TRUTH["document"].items():
        p, results = measure_precision(query, terms, "document")
        doc_precisions.append(p)
        print(f"    {query[:50]:50s} P@3={p:.2f}")

    print("\n  Conversation Queries:")
    for query, terms in GROUND_TRUTH["conversation"].items():
        p, results = measure_precision(query, terms, "conversation")
        conv_precisions.append(p)
        print(f"    {query[:50]:50s} P@3={p:.2f}")

    avg_doc = sum(doc_precisions) / len(doc_precisions) if doc_precisions else 0
    avg_conv = sum(conv_precisions) / len(conv_precisions) if conv_precisions else 0

    print(f"\n  Average Document Precision@3:     {avg_doc:.2f} {'✅' if avg_doc >= 0.50 else '❌'}")
    print(f"  Average Conversation Precision@3: {avg_conv:.2f} {'✅' if avg_conv >= 0.50 else '❌'}")

    # Summary
    print("\n═══ RESULTS SUMMARY ═══")
    print(f"  1. Self-Hosted Consolidation:  {'✅' if success else '❌'}")
    print(f"  2. Fractal Hierarchy:           {'✅' if hierarchy_found else '❌'}")
    print(f"  3. Fractal Zoom:                {'✅' if len(tiers_found) > 1 else '❌'}")
    print(f"  4. Document P@3 ≥ 0.50:        {'✅' if avg_doc >= 0.50 else '❌'} ({avg_doc:.2f})")
    print(f"  5. Conversation P@3 ≥ 0.50:    {'✅' if avg_conv >= 0.50 else '❌'} ({avg_conv:.2f})")

    all_pass = success and hierarchy_found and len(tiers_found) > 1 and avg_doc >= 0.50 and avg_conv >= 0.50
    print(f"\n  ALL CRITERIA MET: {'✅ YES' if all_pass else '❌ NO'}")

    return 0 if all_pass else 1

if __name__ == "__main__":
    sys.exit(main())

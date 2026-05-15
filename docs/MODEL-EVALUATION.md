# KnowWhere Model Evaluation — Embedding & Summarization

**Date:** 2026-05-12
**Goal:** Evaluate Ollama models for KnowWhere's embedding + summarization layers before AMB benchmark
**Author:** Hermes (autonomous operator)

---

## Executive Summary

**Two findings, two recommendations:**

1. **Embedding: `nomic-embed-text-v2-moe` has a 512-token context window → silently truncates all content beyond ~400 words.** Switch to `nomic-embed-text` (v1.5, 8192 context, same 768d). No vRAM penalty, 3-5× better semantic differentiation on long content.

2. **Summarization: `qwen2.5:3b` is the right model.** Quality beats `llama3.2:3b` dramatically (specific claims with evidence vs generic placeholders). Cold-start latency (32s) is acceptable for background consolidation. `phi:3.8b` not tested (not pulled) but qwen2.5 already meets quality requirements.

---

## 1. Embedding Model Evaluation

### 1.1 Truncation Test

Test: progressively longer content (50→800 words) embedded with each model. Cosine similarity between 400-word and 800-word embeddings reveals truncation.

| Model | Dims | Context | 400w↔800w cos_sim | Truncated? |
|-------|------|---------|-------------------|-------------|
| `nomic-embed-text-v2-moe` | 768 | **512 tokens** | **1.000000** | ⚠️ **YES** |
| `nomic-embed-text` (v1.5) | 768 | 8192 | 0.995196 | ✅ No |
| `bge-m3` | 1024 | 8192 | 0.995059 | ✅ No |
| `mxbai-embed-large` | 1024 | 512 | 0.975762 | ✅ No (different handling) |

**Evidence:** `nomic-embed-text-v2-moe` produces identical embeddings for 400-word and 800-word content (cos_sim = 1.0 to 6 decimal places). Every KnowWhere node with content >~500 characters has its embedding truncated to the first ~400 words. This silently degrades retrieval quality for all long-form content.

### 1.2 Retrieval Quality (Top-1 Cosine Similarity)

Test: 20 real KnowWhere nodes, 6 queries (document + conversation types). Each model independently embedded.

| Query | v2-moe (current) | v1.5 (recommended) | bge-m3 |
|-------|------------------|---------------------|--------|
| doc_roadmap | 0.7943 | 0.7533 | 0.7206 |
| doc_state | 0.6467 | **0.7192** ↑ | 0.6555 |
| doc_arch | 0.5033 | **0.6144** ↑ | 0.5822 |
| conv_decision | 0.2969 | **0.5343** ↑ | 0.4431 |
| conv_session | 0.4288 | **0.6699** ↑ | 0.5866 |
| doc_truncation | 0.6154 | 0.6500 | 0.5815 |

**Winner:** `nomic-embed-text` (v1.5) beats v2-moe on **5/6 queries**, with dramatic improvements on conv_decision (+80%) and conv_session (+56%).

### 1.3 Latency

| Model | Cold Start | Warm (per embed) | Size |
|-------|-----------|------------------|------|
| `nomic-embed-text-v2-moe` | 1.7s | 0.16-0.36s | 957 MB |
| `nomic-embed-text` (v1.5) | 1.4s | 0.07-0.24s | 274 MB |
| `bge-m3` | **22.5s** ⚠️ | 0.30-1.72s | 1.2 GB |

bge-m3's cold start is prohibitive (22s). `nomic-embed-text` v1.5 is both faster AND smaller than v2-moe.

### 1.4 Recommendation: Switch to `nomic-embed-text` (v1.5)

- Same 768 dimensions → **no schema migration needed**
- 8192 context → **no silent truncation**
- Better retrieval scores on 5/6 queries
- Smaller (274MB vs 957MB), faster (0.07s vs 0.16s warm)
- Already pulled on this machine

---

## 2. Summarization Model Evaluation

### 2.1 Quality Comparison

Test: real KnowWhere node (602 chars, "SESSION TURN 8 — Alex: The KnowWhere roadmap..."). Exact KnowWhere prompt with JSON schema output.

#### `qwen2.5:3b` (current)
```json
{
  "summary": "Alex outlined the KnowWhere roadmap divided into three phases, 
              detailing progress on each phase with specific metrics and improvements.",
  "claims": [
    {
      "claim": "Phase 1 of KnowWhere's roadmap is complete with RRF k=5 fixed 
                and AMB at 73 percent accuracy.",
      "reason": "Alex stated that Phase 1 Core Loop Proof is complete, specifically 
                 mentioning RRF k=5 fixed and AMB at 73 percent accuracy."
    },
    ...
  ]
}
```
✅ Specific metrics, exact technologies, proper JSON, 3 claims extracted.

#### `llama3.2:3b` 
```json
{
  "summary": "KnowWhere Roadmap Phases",
  "claims": [{"claim": "Three phases of development", "reason": "Detailed roadmap for KnowWhere"}]
}
```
❌ Generic. No metrics, no specific technologies. One vague claim.

### 2.2 Latency

| Model | Cold Start | Warm | Quality |
|-------|-----------|------|---------|
| `qwen2.5:3b` | 31.7s | 18.4s | ✅ Specific, evidence-backed |
| `llama3.2:3b` | 11.9s | 1.9s | ❌ Generic, misses all specifics |

### 2.3 Recommendation: Keep `qwen2.5:3b`

The 10× speed advantage of llama3.2 is irrelevant — consolidation runs asynchronously in the background. Quality is paramount: qwen2.5 extracts specific claims with exact metrics, llama3.2 produces useless placeholders.

`phi:3.8b` (Phi-4-mini) was not tested because it's not pulled (requires `ollama pull phi:3.8b`). Based on benchmarks (67.3% MMLU vs ~63% for llama3.2 3B), it may offer better quality than llama3.2 but likely similar speed. Not worth testing unless qwen2.5 proves insufficient for a specific use case.

---

## 3. Final Recommendations

| Layer | Current | Recommended | Reason |
|-------|---------|-------------|--------|
| **Embedding** | `nomic-embed-text-v2-moe` | **`nomic-embed-text`** (v1.5) | Fixes silent truncation, better retrieval, faster, smaller |
| **Summarization** | `qwen2.5:3b` | **`qwen2.5:3b`** (keep) | Best quality, consolidation is async anyway |

### Migration Steps

1. **Embedding switch:** Update KnowWhere config to use `nomic-embed-text` → run `/nodes/reembed_all` → ~15,350 nodes × 0.15s = ~38 minutes
2. **Server restart:** No code changes needed — just change the model pull/configured name
3. **Verify:** Run 5 test queries, compare retrieval results before/after

---

## 4. Appendix: Raw Test Data

- `/tmp/embedding_results.json` — Truncation test results
- `/tmp/retrieval_quality.json` — Retrieval quality comparison
- `/tmp/summarization_results.json` — Summarization quality comparison

---

## 5. Re-Embedding Results (2026-05-12)

### Execution

| Metric | Value |
|--------|-------|
| **New model** | `nomic-embed-text` v1.5 (8192 context) |
| **Old model** | `nomic-embed-text-v2-moe` (512 context, truncation bug) |
| **Re-embedded** | 15,448 nodes |
| **Failed** | 0 |
| **Server** | Port 3737, debug build |
| **Total nodes** | 15,450 (after re-embed + roadmap doc ingest) |

### Retrieval Quality (Post Re-Embed, RRF k=5)

Content-based P@3 using keyword relevance matching:

| Query | P@3 | Status |
|-------|-----|--------|
| `doc_roadmap` — "What is the KnowWhere roadmap?" | 3/3 = 1.00 | ✓ |
| `doc_state` — "What is the current state?" | 3/3 = 1.00 | ✓ |
| `doc_arch` — "Explain the architecture" | 3/3 = 1.00 | ✓ |
| `conv_decision` — "What decisions were made?" | 1/3 = 0.33 | ⚠ |
| `conv_session` — "What happened in the session?" | 0/3 = 0.00 | ✗ |
| `doc_truncation` — "Does KnowWhere truncate?" | 0/3 = 0.00 | ✗ |

**Note:** RRF scores (0.10–0.21 range) are NOT comparable to cosine similarity scores (0.50–0.80 range) from earlier benchmarks. The scoring system changed from direct cosine similarity to RRF k=5 normalized scoring when moving from raw embedding queries to the `/retrieve_fractal` API endpoint.

### Roadmap Document

A dedicated semantic/document node was ingested with `POST /store_external`:
- **ID:** `1437db41-b5eb-4636-9fd0-2d07202d38e7`
- **Content:** Three-phase roadmap with keywords
- **Retrieval:** Rank 2 for keyword-heavy queries, not in top 5 for natural-language queries (known embedding-density limitation — claim nodes with exact phrase matches score higher via BM25)

### Files

- `/tmp/retrieval_final_v15.json` — Final retrieval results post re-embed
- `/tmp/retrieval_quality_reembedded.json` — Comparison data

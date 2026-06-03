<div align="center">

# KnowWhere

### Lossless fractal memory for AI agents — every fact has an address.

**Pointer-first. Fractal Zoom. 0% information loss.**

[![CI](https://github.com/Jind0la/knowwhere/actions/workflows/ci.yml/badge.svg)](https://github.com/Jind0la/knowwhere/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

## What KnowWhere is

Most AI memory systems extract "facts" from conversations and discard the rest. KnowWhere doesn't. It stores every piece of information in a **fractal hierarchy** — atomic facts at the bottom (L0), summaries in the middle (L1), overviews at the top (L2). You can search at any resolution and zoom down to the original data. Nothing is ever lost.

> Hindsight extracts facts. LangChain stores vectors. KnowWhere stores *knowledge* — with provenance, trust tiers, and a pointer back to every original source.

### Why this matters

When an agent asks "why did we decide X three months ago?", other memory systems return isolated facts. KnowWhere returns the *entire decision path* — from the original conversation rounds (L0) through the summary (L1) to the strategic overview (L2). Fractal Zoom makes this possible.

---

## 🎯 Core Loop Status (May 2026)

> **Verdict: The Core Loop works — and then some.** After the v0.6.0 transformation (Turn-Level Storage, Hybrid Retrieval, Cross-Encoder Reranking, Source-Type Weighting, Temporal-Aware Scoring), KnowWhere achieves **72.97% Recall@5** on LongMemEval — up from 7.1% pre-migration.

| Component | Status | Details |
|---|---|---|---|
| Ingestion (Conversations) | ✅ Working | **Turn-Level** — per-turn embeddings with EmbeddingInfo (provider, dimension, metadata) |
| Retrieval | ✅ Working | BM25 + Dense Hybrid + Cross-Encoder (gte-modernbert ONNX) + RRF Fusion |
| Scoring | ✅ Working | Source-Type Weighting + Temporal Decay + Trust Tiers |
| Turn-Level Migration | ✅ Complete | 81 tasks, 8 initiatives; Migration 014–017; Session embeddings removed |
| LongMemEval Benchmark | ✅ Complete | 42 stratified cases, all 6 question types functional [→ Report](benchmarks/reports/LONGMEMEVAL_COMPARISON.md) |
| Consolidation | ✅ Active | Self-hosted Ollama (qwen2.5:3b), L0→L1→L2 chains |
| Fact Extraction | ✅ Working | Symbolic facts extracted and weighted separately |
| Embedding Model | ✅ Stable | `nomic-embed-text` (768d, 8192 context) |
| Cross-Encoder | ✅ Working | gte-modernbert via ONNX (599 MB, no Ollama needed) |

📊 **Phase 2 Completion:** [`docs/phase2-retrieval-quality-completion.md`](docs/phase2-retrieval-quality-completion.md)  
📋 **81-Task Summary:** CHANGELOG v0.6.0 section  
🏗️ **Architecture:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)  
🧪 **Evaluation:** [`benchmarks/reports/LONGMEMEVAL_COMPARISON.md`](benchmarks/reports/LONGMEMEVAL_COMPARISON.md)

---

## Start Here

- **API Reference:** [docs/API_REFERENCE.md](docs/API_REFERENCE.md)
- **Architecture:** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Contributing:** [CONTRIBUTING.md](CONTRIBUTING.md)
- **ADR Index:** [docs/ADR_INDEX.md](docs/ADR_INDEX.md)
- **Setup guide:** [docs/archive/QUICKSTART.md](docs/archive/QUICKSTART.md) (v0.5, needs update)
- **Product scope:** [docs/archive/PRD.md](docs/archive/PRD.md) (v0.5, vision still valid)

---

## Current status — v0.6.0 (Post-Migration)

**82 tasks across 8 initiatives** — the largest architectural upgrade in KnowWhere's history.

### Major features shipped

| Initiative | Tasks | What Changed |
|-----------|:-----:|--------------|
| **Turn-Level Storage + Per-Turn Embeddings** | 26 | Session-Level → Turn-Level granularity. Every conversation turn gets its own embedding with EmbeddingInfo metadata. Migration 015 drops the old session embedding column. |
| **Stratified LongMemEval Benchmark** | 12 | 42-case stratified benchmark across all 6 question types. From single-type-only (7.1%) to full coverage (72.97%). |
| **Hybrid BM25 + Dense Retrieval** | 6 | Keyword + semantic fusion catches queries that pure dense misses. |
| **Source-Type Weighting** | 13 | Real conversations weighted higher than synthetic injections. Provenance tracking on every result. |
| **Cross-Encoder Reranking** | 11 | gte-modernbert (ONNX, 599 MB) reranks top-K candidates. No Ollama dependency for reranking. |
| **Fact Extraction Pipeline** | 5 | Symbolic knowledge extraction from conversations, stored and weighted separately. |
| **Temporal-Aware Scoring** | 5 | Recency decay: newer information gets higher weights. |
| **Ollama Slimming** | — | 14 models (18 GB) → 3 models (4.2 GB). Only runtime-required models retained. |

### Benchmark Results (42-case stratified LongMemEval)

| Metric | Pre-Migration | Post-Migration |
|--------|:------------:|:-------------:|
| **Overall Recall@5** | 7.1% | **72.97%** |
| **MRR** | ~0.00 | **0.56** |
| **Turn-Level NDCG@5** | — | **0.42** |

All 6 question types functional (up from 1/6). [→ Full Comparison](benchmarks/reports/LONGMEMEVAL_COMPARISON.md)

---

## Current status — v0.5.0 (Legacy)

| Category | Status |
|----------|--------|
| **Core API** | ✅ store_session, store_external, retrieve_fractal, chat/subconscious |
| **Batch API** | ✅ store_session_batch, batch_delete |
| **Fractal Zoom** | ✅ zoom_retrieve() with hierarchical pruning across L0→L1→L2 |
| **6-Type System** | ✅ Episodic, Semantic, Preference, Procedural, Meta, Decision |
| **Decision Scoring** | ✅ PRIMARY trust tier + 1.5× memory_type_multiplier = 2× boost |
| **Trust Tiers** | ✅ primary, reference, derived, volatile — auto-detected |
| **L2→L1→L0 Compaction** | ✅ LocalSummarizer (Ollama qwen2.5:3b, 92.1% instruction-following) + VLM fallback |
| **Claims Extraction** | ✅ JSON Schema (GBNF-constrained) → 92.6% coverage, ∅4.3/5 specificity, Evidence-First prompt |
| **Reflect Mode** | ✅ Query-time memory synthesis via Ollama |
| **Event Consolidation** | ✅ Write-driven trigger + POST /consolidation/force |
| **Hybrid Retrieval** | ✅ USearch vector + BM25 keyword + RRF fusion |
| **Hermes Retrieval Quality** | ✅ Strict type filters, no default Meta/Reflect leakage, intent + dedupe + MMR, eval script |
| **Energy Decay** | ✅ Ebbinghaus forgetting curve |
| **Governance** | ✅ Retrieval profiles, sensitivity levels |
| **Auth** | ✅ Static admin key + user registration (PostgreSQL) |
| **PostgreSQL** | ✅ Dedup, conflicts, self-healing, namespaces, skills, tier persistence, `expand_fractal` parity |
| **Native macOS** | ✅ Zero-Docker: Ollama native + PostgreSQL Homebrew + KnowWhere binary |
| **Tests** | ✅ 136 unit (0 failed) + 40 integration (0 failed) + 9 ignored |
| **Benchmark** | ✅ 50-case LongMemEval: Top-1 96%, Recall@5 96%, MRR 0.96 |
| **Hermes Plugin** | ✅ MemoryProvider: per-turn crash-safe storage, safe prefetch, provenance metadata |
| **Cross-Modal** | ✅ EmbeddingRouter: CLIP/Whisper/Sensor via Ollama |
| **Cross-Encoder** | ✅ bge-reranker-v2-m3 via ONNX (feature: reranker) |
| **Entity Search** | ✅ GET /entities — entity_edges table with model/tool/project tracking |
| **Decision Extraction** | ✅ Structured claims with decision_what/decision_why metadata |
| **Webhooks** | ✅ Frigate + HomeAssistant webhook endpoints |

---

## Quick start (Docker Compose)

One command:

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
cp .env.example .env
docker compose up -d --build
```

> On first start, KnowWhere connects to Ollama (native macOS). Models `nomic-embed-text-v2-moe` (multilingual, 768-dim, MoE) and `llama3.2` (summarization) must be pre-pulled. Embedding latency: ~0.23s warm.

Verify:

```bash
curl http://localhost:3737/health
# → {"status":"ok","node_count":0}
```

First API call:

```bash
curl -X POST http://localhost:3737/store_session \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer *** \
  -d '{"content": "USER: What is KnowWhere?\nASSISTANT: A fractal memory service."}'

curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer *** \
  -d '{"query_text": "What is KnowWhere", "profile": "user-facing"}'
```

---

## How it works

```text
                    ┌─────────────────────┐
                    │   Agent / SDK / UI   │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                ▼                 ▼
     store_session     store_external     retrieve_fractal
              │                │                 │
              ▼                ▼                 ▼
     ┌────────────────────────────────────────────┐
     │           Fractal Memory Store              │
     │                                             │
     │  L2: Overview ────► L1: Summary ────► L0: Raw │
     │  (zoom out)          (mid-level)       (atomic) │
     │                                             │
     │  USearch(Vector) + BM25(Keyword) + RRF     │
     │  Trust Tiers + Governance + Energy Decay    │
     └────────────────────────────────────────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                ▼                 ▼
        PostgreSQL      Local Ollama      Cloud VLM
       (persistence)   (embeddings +     (GPT-5-nano→
                        summarization)   GPT-4o-mini→
                                          Grok-4-fast)
```

---

## Pointer-first data model

KnowWhere distinguishes two fundamental memory types with full provenance:

| Type | What it stores | Example |
|------|---------------|---------|
| **Session** | Full text + embedding + metadata | Chat rounds, decisions, notes |
| **External** | Pointer + embedding + metadata only | File paths, URLs, sensor IDs |

Every node carries: memory type (5 types), source (5 sources), trust tier (auto-detected), confidence, sensitivity, importance, conflict state.

---

## API overview

### Core (always available)

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/store_session` | Store full-text session memory with auto-chunking |
| `POST` | `/store_external` | Store pointer-only external reference |
| `POST` | `/retrieve_fractal` | Hybrid retrieval with fractal zoom + profile-based scoring |
| `POST` | `/chat/subconscious` | Retrieval-backed response with cited sources |
| `GET` | `/retrieve/{id}` | Fetch single node by ID |
| `GET` | `/nodes/recent` | Recent nodes |
| `POST` | `/consolidation/force` | Trigger full re-consolidation (admin) |
| `GET` | `/dream/status` | Compaction scheduler status |
| `GET` / `POST` | `/governance/policy` | Read / update governance policy |

### PostgreSQL-only (postgres-storage feature)

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/retrieval/runs` | Retrieval analytics |
| `POST` | `/energy/decay` | Apply Ebbinghaus forgetting curve |
| `POST` | `/deduplication/run` | Find and merge duplicate memories |
| `GET` | `/conflicts` | List conflicting memories |
| `GET` | `/self-healing/stats` | Orphaned nodes, broken links, embedding drift |
| `GET` | `/namespaces` | Namespace-organized memory views |
| `POST` | `/memories/{id}/compact` | Trigger tiered compaction for a node |

---

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `KNOWWHERE_API_KEY` | unset | Admin Bearer token (auth off if unset) |
| `DATABASE_URL` | unset | PostgreSQL backend (postgres-storage feature) |
| `OLLAMA_URL` | `http://localhost:11434` | Ollama API base URL |
| `OLLAMA_MODEL` | `nomic-embed-text-v2-moe` | Embedding model (768-dim, MoE, multilingual) |
| `OLLAMA_SUMMARIZER_MODEL` | `qwen2.5:3b` | Summarization model (92.1% instruction-following, best in 3B class) |
| `KNOWWHERE_EMBEDDING_PROVIDER` | `ollama` | Embedding backend: ollama (default), openai, grok |
| `GROK_API_KEY` | unset | Grok/xAI embeddings or VLM fallback |
| `FRIGATE_URL` | unset | Frigate NVR connector |
| `RUST_LOG` | `info` | Tracing verbosity |

---

## SDK and integrations

- **Python SDK:** `sdk/python`
- **Hermes MemoryProvider Plugin:** Per-turn crash-safe storage + dual retrieval (episodic + decision). Auto-discovered on Hermes startup.
- **Swagger UI:** `http://localhost:3737/swagger-ui/`

---

## Development

```bash
# Unit tests (always work)
cargo test --lib                         # 136 tests

# Integration tests (need PostgreSQL + Ollama)
DATABASE_URL="postgresql:///knowwhere_dev?host=localhost" \
OLLAMA_URL=http://127.0.0.1:11434 \
SQLX_OFFLINE=true \
cargo test --features postgres-storage --test integration  # 35 tests

# Native macOS server (recommended)
export KNOWWHERE_API_KEY="kw_testkey_12345"
export DATABASE_URL="postgresql:///knowwhere_dev?host=localhost"
export OLLAMA_URL="http://127.0.0.1:11434"
cargo run --release --features postgres-storage,summarizer
```

---

## License

[MIT](LICENSE) — 2026 KnowWhere contributors

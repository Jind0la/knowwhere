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

## Start Here

- **5-minute setup:** [docs/QUICKSTART.md](docs/QUICKSTART.md)
- **Full walkthrough:** [docs/WALKTHROUGH.md](docs/WALKTHROUGH.md)
- **Current limitations:** [docs/BETA-README.md](docs/BETA-README.md)
- **Product scope:** [docs/PRD.md](docs/PRD.md)
- **Architecture:** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

---

## Current status — v0.5.0

| Category | Status |
|----------|--------|
| **Core API** | ✅ store_session, store_external, retrieve_fractal, chat/subconscious |
| **Batch API** | ✅ store_session_batch, batch_delete |
| **Fractal Zoom** | ✅ zoom_retrieve() with hierarchical pruning across L0→L1→L2 |
| **6-Type System** | ✅ Episodic, Semantic, Preference, Procedural, Meta, Decision |
| **Decision Scoring** | ✅ PRIMARY trust tier + 1.5× memory_type_multiplier = 2× boost |
| **Trust Tiers** | ✅ primary, reference, derived, volatile — auto-detected |
| **L2→L1→L0 Compaction** | ✅ LocalSummarizer (Ollama llama3.2) + VLM fallback, event-driven |
| **Claims Extraction** | ✅ Structured claim parsing from summaries → Decision nodes |
| **Reflect Mode** | ✅ Query-time memory synthesis via Ollama |
| **Event Consolidation** | ✅ Write-driven trigger + POST /consolidation/force |
| **Hybrid Retrieval** | ✅ USearch vector + BM25 keyword + RRF fusion |
| **Energy Decay** | ✅ Ebbinghaus forgetting curve |
| **Governance** | ✅ Retrieval profiles, sensitivity levels |
| **Auth** | ✅ Static admin key + user registration (PostgreSQL) |
| **PostgreSQL** | ✅ Dedup, conflicts, self-healing, namespaces, skills, tier persistence |
| **Native macOS** | ✅ Zero-Docker: Ollama native + PostgreSQL Homebrew + KnowWhere binary |
| **Tests** | ✅ 136 unit (0 failed) + 35 integration (0 failed) + 9 ignored |
| **Benchmark** | ✅ 50-case LongMemEval: Top-1 96%, Recall@5 96%, MRR 0.96 |
| **Hermes Plugin** | ✅ MemoryProvider: per-turn crash-safe storage + dual retrieval |
| **Cross-Modal** | ✅ EmbeddingRouter: CLIP/Whisper/Sensor via Ollama |
| **Cross-Encoder** | ✅ bge-reranker-v2-m3 via ONNX (feature: reranker) |
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
| `OLLAMA_SUMMARIZER_MODEL` | `llama3.2` | Summarization model for L2→L1→L0 |
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

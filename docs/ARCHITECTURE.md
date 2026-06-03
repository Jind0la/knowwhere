# KnowWhere Architecture — v0.6.0

## Vision

KnowWhere is a **lossless fractal memory system for AI agents**. Unlike conventional vector databases that extract "facts" and discard context, KnowWhere stores every piece of information in a **fractal hierarchy** — raw conversation turns at the bottom (L0), summaries in the middle (L1), strategic overviews at the top (L2). You can search at any resolution and zoom down to the original data. Nothing is ever lost.

> Hindsight extracts facts. LangChain stores vectors. KnowWhere stores *knowledge* — with provenance, trust tiers, and a pointer back to every original source.

## Core Principles

1. **Pointer-First** — Every node carries a pointer to its source. You can always trace a claim back to the exact conversation turn that produced it.
2. **Fractal Zoom** — Search at any resolution. An overview node expands into its children, down to raw turns. Like a map that lets you zoom from continent to street.
3. **0% Information Loss** — No summarization without preservation. The original data is always accessible, even after consolidation.
4. **Provenance-Aware** — Every node is classified by its source type (Real/Synthetic/Derived/Unknown) and scored accordingly. Human conversations carry more weight than AI generations.

## System Architecture

```
┌─────────────────────────────────────────────────┐
│                  CLIENT LAYER                     │
│  Hermes Plugin  │  REST API  │  OpenClaw  │  SDK │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────┴──────────────────────────┐
│                 API LAYER (14 modules)            │
│  routes.rs (104 LOC router)                       │
│  ├── health.rs          ├── store.rs              │
│  ├── retrieve.rs        ├── rerank.rs             │
│  ├── maintenance.rs     ├── trajectory.rs         │
│  ├── conflicts.rs       ├── energy.rs             │
│  ├── dedup.rs           ├── healing.rs            │
│  ├── namespaces.rs      ├── skills_routes.rs      │
│  ├── turn_handlers.rs   └── auth.rs               │
│  types.rs — shared request/response types         │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────┴──────────────────────────┐
│                MEMORY ENGINE                      │
│                                                   │
│  types.rs          — MemoryType (6 variants)      │
│  fractal_node.rs   — FractalNode struct           │
│  conversation.rs   — Turn/Session storage         │
│  chunking.rs       — Semantic text chunker        │
│  fact_extraction   — Regex-based fact extraction  │
│  governance.rs     — Policy layer (trust tiers)   │
│  control_room.rs   — Multi-agent memory scoping   │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────┴──────────────────────────┐
│              RETRIEVAL ENGINE                     │
│                                                   │
│  hybrid.rs         — BM25 + Dense RRF fusion      │
│  source_weighting  — Provenance-aware scoring     │
│  temporal.rs       — Recency decay (Ebbinghaus)   │
│  cross_encoder.rs  — ONNX reranker (gte)          │
│  scoring.rs        — Tier × Source × Time chain   │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────┴──────────────────────────┐
│               STORAGE BACKEND                     │
│                                                   │
│  backend.rs   — StorageBackend trait              │
│  ├── in_memory.rs   — Dev/testing (USearch idx)   │
│  └── postgres_store — Production (pgvector+bm25)  │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────┴──────────────────────────┐
│            EMBEDDING & INFERENCE                  │
│                                                   │
│  nomic-embed-text (768d)  — Ollama               │
│  gte-modernbert ONNX      — Cross-encoder        │
│  qwen2.5:3b               — Consolidation LLM    │
│  CLIP + Whisper           — Multimodal routing   │
└─────────────────────────────────────────────────┘
```

## Data Flow

### Ingestion

```
User Message
    │
    ▼
TextChunker (semantic boundary detection)
    │
    ▼
EmbeddingRouter (content-type dispatch)
    ├── text/plain     → Ollama nomic-embed-text
    ├── image/*        → CLIP (768-dim projection)
    ├── audio/*        → Whisper → text → Ollama
    └── application/*  → Metadata extraction
    │
    ▼
FractalNode creation (with EmbeddingInfo metadata)
    │
    ▼
Storage Backend (InMemoryStore or PostgresStore)
    │
    ▼
Fact Extraction (regex rules, inline, no LLM)
    │
    ▼
Consolidation (qwen2.5:3b, L0→L1→L2 chains)
```

### Retrieval

```
Query Text
    │
    ▼
Query Embedding (nomic-embed-text)
    │
    ▼
Hybrid Retrieval
    ├── Dense: USearch/pgvector cosine similarity (top-100)
    ├── BM25: Keyword matching on content + original_pointer
    └── RRF Fusion: Reciprocal Rank Fusion (k=5) → top-20
    │
    ▼
Cross-Encoder Reranking (gte-modernbert ONNX, top-20 → top-K)
    │
    ▼
Source-Type Weighting (Real > Synthetic > Derived > Unknown)
    │
    ▼
Temporal Scoring (recency decay via temporal_weight)
    │
    ▼
Diversity Sampling (temporal diversity across Early/Mid/Late phases)
    │
    ▼
Score Debug (transparent multiplier chain in response)
```

## Key Design Decisions

### Turn-Level over Session-Level

**Problem:** Session-level embeddings averaged all messages into one vector, losing speaker identity, temporal order, and fine-grained retrieval precision.

**Decision:** Every conversation turn gets its own embedding with `EmbeddingInfo` metadata (provider, dimension). Turn-level retrieval achieves 93.3% recall@5 vs 73.3% session-level on LongMemEval.

### Hybrid BM25 + Dense with RRF

**Problem:** Pure dense retrieval misses exact keyword matches (proper names, technical IDs). Pure BM25 misses semantic similarity.

**Decision:** Both run in parallel, fused via Reciprocal Rank Fusion (k=5). The k=5 was discovered after debugging a k=60 score collapse — too many candidates diluted the signal.

### ONNX Cross-Encoder over Ollama Reranker

**Problem:** Running a reranker via Ollama added latency and a hard dependency on the Ollama service.

**Decision:** gte-modernbert runs as an ONNX model (599 MB) with zero external dependencies. Faster inference, no Ollama needed for reranking.

### Source-Type Weighting

**Problem:** All retrieved nodes were scored equally, regardless of whether they came from real conversations or AI-generated consolidations.

**Decision:** Four-tier provenance classification with configurable multipliers. Real (1.0) > Synthetic (0.85) > Derived (0.70) > Unknown (0.95). Configurable via env var, file, or per-query.

### Summarizer Removal

**Problem:** The VLM fallback chain (Ollama → cloud VLM → local llama3.2) added ~4K LOC of complexity with marginal quality gain.

**Decision:** Removed entirely. Consolidation now uses a single ollama qwen2.5:3b model. Simpler, faster, more maintainable.

## Scaling Characteristics

| Component | Current Scale | Scaling Path |
|-----------|--------------|--------------|
| Embeddings | 768-dim, ~50K nodes | Matryoshka truncation (768→256→64) |
| Retrieval | In-memory USearch | PostgreSQL pgvector for >1M nodes |
| Consolidation | qwen2.5:3b (local) | Cloud LLM fallback for large batches |
| Storage | ~10 GB state | PostgreSQL with partitioning |

## Integration Points

- **Hermes Agent Plugin** — Native memory provider for Hermes, using turn-level storage with crash-safe dual retrieval
- **OpenClaw** — Agent framework integration via REST API
- **Frigate NVR** — Webhook-based event ingestion (every 30s poll)
- **Home Assistant** — Webhook-based state change ingestion
- **Google Drive** — Optional document import (feature-gated behind `google-drive` feature)

## Monitoring

- `GET /health` — Basic health check
- `GET /dream/status` — Consolidation scheduler state
- `GET /events` — Recent system events
- Score Debug in every retrieval response — transparent multiplier chain

---

*See [ARCHITECTURE_MAP.md](../ARCHITECTURE_MAP.md) for module-level navigation and [ADR_INDEX.md](ADR_INDEX.md) for architecture decision records.*

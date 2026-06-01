# src/

KnowWhere source code — a Rust-based fractal memory system for AI agents.

## Module Overview

| Directory | Purpose |
|-----------|---------|
| `api/` | REST API routes (store, retrieve, consolidate, governance, webhooks) |
| `bin/` | CLI tools (LongMemEval evaluation, canary tests) |
| `connectors/` | External integrations (Frigate NVR, Google Drive) |
| `embedding/` | Embedding providers (Ollama, OpenAI, Grok) with routing and caching |
| `memory/` | Core fractal memory engine — nodes, conversations, fact extraction, consolidation |
| `retrieval/` | Hybrid retrieval: BM25 keyword + dense vector + cross-encoder reranking + RRF fusion |
| `scheduler/` | Consolidation scheduler (Dream Pipeline: Claims→Dedup→Conflict) |
| `services/` | Service layer orchestrating memory operations |
| `storage/` | Database backends (PostgreSQL, in-memory) with migrations |

## Entry Points

- `main.rs` — Server bootstrap, route wiring, startup sequence
- `lib.rs` — Public library interface for SDK consumers

## Key Architecture

The fractal memory hierarchy (L0→L1→L2) lives in `memory/fractal_node.rs`. Retrieval flows through `retrieval/` → `memory/` → `storage/`. Consolidation is driven by `scheduler/` and implemented in `memory/dream/`.

See [ARCHITECTURE.md](../docs/ARCHITECTURE.md) for full details.

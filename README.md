<div align="center">

# KnowWhere

### Dein KI-Gedaechtnis, das nie vergisst.

**Pointer-first fractal memory service for AI agents.**

[![CI](https://github.com/NimarMoradbakhti/knowwhere/actions/workflows/ci.yml/badge.svg)](https://github.com/NimarMoradbakhti/knowwhere/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

KnowWhere is a long-term memory backend for AI agents. It stores session data (full text + embeddings) and references external data sources via **pointers only** — never raw files. It features **hybrid retrieval** (semantic vector search + BM25 keyword search fused via Reciprocal Rank Fusion), fractal zooming through memory clusters, a "Dream Mode" for organic cluster formation, and pluggable embedding providers.

## How It Works

```
User Message ──→ store_session ──→ [Embedding + BM25 Index]
                                          │
Next Prompt  ──→ retrieve_fractal ──→ [Hybrid Search] ──→ Ranked Context
                                          │
AI Response  ──→ store_session ──→ [Embedding + BM25 Index]
```

Every user message and AI response is embedded and indexed. On the next query, KnowWhere performs hybrid retrieval (vector similarity + keyword matching), returning ranked results with relevance scores. The full conversation loop is preserved.

## Quickstart

### Local (recommended for development)

```bash
git clone https://github.com/NimarMoradbakhti/knowwhere.git
cd knowwhere
cargo run
```

Requires Rust 1.85+ via [rustup](https://rustup.rs) and [Ollama](https://ollama.ai) running locally with the `nomic-embed-text-v2-moe` model:

```bash
ollama pull nomic-embed-text-v2-moe
```

Open [http://localhost:3737/swagger-ui/](http://localhost:3737/swagger-ui/) for the interactive API docs.

### Docker

```bash
docker compose up --build
```

## Core Concepts

### Pointer-First Principle
- **Session nodes** (`store_session`): Full text + embedding stored. Used for conversations, decisions, notes.
- **External nodes** (`store_external`): Only a pointer string + embedding + metadata. Never raw files. Used for cameras, sensors, documents.

### Hybrid Retrieval
KnowWhere combines two search strategies for optimal results:
1. **Semantic search** via USearch (cosine similarity on embeddings)
2. **Keyword search** via BM25 (exact term matching, German-optimized)
3. **Reciprocal Rank Fusion (RRF)** merges both ranked lists into a single result

### Fractal Zooming
Nodes can have children. During retrieval, KnowWhere "zooms" into the best-matching child nodes up to `max_depth` levels, finding increasingly specific context.

### Dream Mode
A background process that periodically clusters related nodes and strengthens connections — making retrieval organically better over time.

## API Endpoints

| Method | Path                | Auth     | Description                                    |
|--------|---------------------|----------|------------------------------------------------|
| GET    | `/health`           | Public   | Server status + node count                     |
| GET    | `/swagger-ui/`      | Public   | Interactive OpenAPI documentation               |
| POST   | `/embed`            | Required | Generate embedding vector for text              |
| POST   | `/store_session`    | Required | Store session node (full content + embedding)   |
| POST   | `/store_external`   | Required | Store external pointer (no raw data)            |
| GET    | `/retrieve/{id}`    | Required | Retrieve single node by UUID                    |
| POST   | `/retrieve_fractal` | Required | Hybrid fractal search (returns `ScoredNode[]`)  |
| GET    | `/nodes/recent`     | Required | Recent nodes (sorted by `created_at`)           |
| DELETE | `/nodes/{id}`       | Required | Delete node by UUID                             |
| POST   | `/nodes/purge_dummy`| Required | Remove nodes with placeholder vectors           |
| POST   | `/nodes/reembed_all`| Required | Re-embed all nodes with current provider        |
| GET    | `/dream/status`     | Required | Dream mode status                               |

### Key Response Types

**ScoredNode** (returned by `/retrieve_fractal`):
```json
{
  "score": 0.032,
  "id": "uuid",
  "node_type": "Session",
  "content": "The app should be anonymous...",
  "original_pointer": null,
  "metadata": { "source": "user:Nimar" },
  "created_at": "2026-03-02T14:20:28Z"
}
```

Note: The `vector` field is intentionally excluded from retrieval responses to save bandwidth.

**NodeType**: `Session` (full text stored) or `External` (pointer only).

## Integration Philosophy

**KnowWhere is additive, never destructive.**

When connecting KnowWhere to an existing agent system, it must:

1. **Discover** — scan the host system for existing memories, identity files, agent knowledge, and session history
2. **Import** — bring all existing knowledge into KnowWhere as Session nodes with full provenance metadata
3. **Preserve** — all original files stay untouched. Nothing gets deleted, overwritten, or reset
4. **Layer** — KnowWhere adds a retrieval layer on top. The host's memory system keeps running
5. **Degrade gracefully** — if KnowWhere goes offline, the host system works normally

This means: no deleting `MEMORY.md`, no resetting conversation history, no overwriting identity files. Import first, then enhance.

### Proven Import Results (OpenClaw)

Our first integration imported 100 nodes from OpenClaw covering personal info, agent identity, 5 sub-agent workspaces (research, business strategy, design, dev, marketing), daily logs, conversation history, and project context. All knowledge is now retrievable via a single hybrid search query. See `docs/IMPORT_GUIDE.md` for the full playbook.

## Agent Integration (OpenClaw)

KnowWhere ships with an OpenClaw plugin (`knowwhere-memory`) that provides a complete memory loop:

| Hook              | Purpose                                      |
|-------------------|----------------------------------------------|
| `message_received`| Stores every incoming user message            |
| `llm_output`      | Stores every AI response (with model info)    |
| `before_prompt_build` | Retrieves and injects relevant context    |

The plugin also handles health checks, self-reference filtering, and score-based relevance gating.

### Plugin Configuration

In `openclaw.json`:
```json
{
  "plugins": {
    "entries": {
      "knowwhere-memory": {
        "enabled": true,
        "config": {
          "url": "http://127.0.0.1:3737",
          "topK": 5,
          "maxDepth": 3
        }
      }
    }
  }
}
```

## Python SDK

### Installation

```bash
pip install -e sdk/python
```

### Basic Usage

```python
from knowwhere import KnowWhereClient

client = KnowWhereClient()
client.store_session("The app should be anonymous, no login needed")
results = client.retrieve_fractal("What was the design decision?")
```

### LangChain Integration

```python
from knowwhere import KnowWhereClient, KnowWhereMemory

client = KnowWhereClient()
memory = KnowWhereMemory(client=client)
memory.add_user_message("Remember: deploy on Friday")
context = memory.get_context_string("When do we deploy?")
```

## Environment Variables

| Variable             | Required | Default                  | Description                                              |
|----------------------|----------|--------------------------|----------------------------------------------------------|
| `KNOWWHERE_PORT`     | No       | `3737`                   | Server listen port                                       |
| `KNOWWHERE_API_KEY`  | No       | *(unset)*                | If set, all routes except `/health` require Bearer token |
| `KNOWWHERE_DATA_DIR` | No       | `./data`                 | Directory for persisted state (`state.json`)             |
| `GROK_API_KEY`       | No       | *(unset)*                | Grok/xAI embedding provider API key                      |
| `OPENAI_API_KEY`     | No       | *(unset)*                | OpenAI embedding provider API key                        |
| `OLLAMA_MODEL`       | No       | `nomic-embed-text-v2-moe`| Local Ollama embedding model name                        |
| `FRIGATE_URL`        | No       | *(unset)*                | Frigate NVR URL (enables camera event connector)         |
| `RUST_LOG`           | No       | `info`                   | Tracing log level                                        |

If neither `GROK_API_KEY` nor `OPENAI_API_KEY` is set, KnowWhere falls back to local Ollama.

## Authentication

```bash
export KNOWWHERE_API_KEY=my-secret-key-123
cargo run
```

```bash
curl -H "Authorization: Bearer my-secret-key-123" http://localhost:3737/embed \
  -d '{"text":"hello"}' -H "Content-Type: application/json"
```

Public endpoints (no token): `/health`, `/swagger-ui/*`

## Architecture

- **Backend:** Rust 1.85+ (Axum 0.8, Tokio, Tower)
- **Embeddings:** Pluggable — Grok (xAI), OpenAI, local Ollama (`nomic-embed-text-v2-moe`)
- **Vector Store:** USearch (cosine similarity, HNSW)
- **Keyword Search:** BM25 with cached scorer (German-optimized)
- **Fusion:** Reciprocal Rank Fusion (RRF, k=60)
- **Graph:** In-memory fractal graph with Dream Mode clustering
- **Persistence:** JSON state file with debounced auto-save + graceful shutdown
- **Connectors:** Frigate NVR (optional, pointer-only)
- **SDK:** Python 3.11+ with LangChain/LlamaIndex compatibility
- **Docs:** OpenAPI 3.0 via utoipa + Swagger UI
- **Principle:** Pointer-First — external data is never stored, only referenced

## Deployment

### Railway

```bash
railway login && railway init && railway up
```

### Fly.io

```bash
fly launch
fly secrets set KNOWWHERE_API_KEY=your-secret
fly deploy
```

### Local

```bash
cargo run
```

## Running Tests

```bash
cargo test
```

## Contributing

Contributions are welcome! Please open an issue or pull request on [GitHub](https://github.com/NimarMoradbakhti/knowwhere).

---

> **Beta Notice**
>
> KnowWhere is currently in **Beta (v0.2.0)**. We are actively looking for early testers and feedback.
> Reach out to **@NimarMoradbakhti** on X or via email to get involved!

## License

[MIT](LICENSE) — 2026 Nimar Moradbakhti & KnowWhere Contributors

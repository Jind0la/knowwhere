# KnowWhere API Reference — v0.6.0

Base URL: `http://localhost:3737` (configurable via `KNOWWHERE_PORT`)

Authentication: `Authorization: Bearer <KNOWWHERE_API_KEY>` header on protected endpoints.

---

## Public Endpoints (no auth)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/login` | Authenticate and receive a JWT token |
| POST | `/refresh` | Refresh an expiring JWT token |
| POST | `/register` | Register a new user (rate-limited: 10/60s) |

---

## Protected Endpoints (auth required)

### Storage

| Method | Path | Description |
|--------|------|-------------|
| POST | `/store_session` | Store a single-turn conversation session |
| POST | `/store_session_batch` | Store multiple turns in batch |
| POST | `/store_external` | Store external content (documents, imports) |
| POST | `/memory/self_improve` | Trigger AI self-improvement cycle |

### Retrieval

| Method | Path | Description |
|--------|------|-------------|
| POST | `/retrieve_fractal` | **Primary retrieval endpoint.** Hybrid BM25+dense with fractal zoom. Supports `temporal_weight` (0.0–0.8), `source_type_weights`, `diversity`, `user_id` filter. |
| GET | `/retrieve/{id}` | Retrieve a specific node by UUID |
| POST | `/rerank` | Cross-encoder reranking (gte-modernbert ONNX) of candidate list |
| GET | `/nodes/recent` | List recently stored nodes |

### Embedding

| Method | Path | Description |
|--------|------|-------------|
| POST | `/embed` | Generate embedding vector for text. Supports multimodal routing (text→Ollama, image→CLIP, audio→Whisper). |

### Maintenance

| Method | Path | Description |
|--------|------|-------------|
| POST | `/nodes/purge_dummy` | Remove nodes with zero-vector embeddings |
| POST | `/nodes/reembed_all` | Recompute embeddings for all nodes |
| POST | `/maintenance/repair_embeddings` | Detect and fix broken embeddings |
| DELETE | `/nodes/{id}` | Delete a single node |
| POST | `/nodes/batch_delete` | Delete multiple nodes by ID list |
| POST | `/nodes/deduplicate` | Find and merge duplicate nodes |

### Chat

| Method | Path | Description |
|--------|------|-------------|
| POST | `/chat/subconscious` | Subconscious Q&A — retrieves relevant memories and generates answers via LLM |

### Configuration

| Method | Path | Description |
|--------|------|-------------|
| GET | `/config/temporal_weight` | Get server-wide temporal weight default |
| POST | `/config/temporal_weight` | Set server-wide temporal weight (0.0–0.8) |

### Governance

| Method | Path | Description |
|--------|------|-------------|
| GET | `/governance/policy` | Get current governance policy |
| POST | `/governance/policy` | Update governance policy rules |

### System

| Method | Path | Description |
|--------|------|-------------|
| GET | `/dream/status` | Get Dream Mode scheduler status |
| GET | `/events` | List recent system events |

### Webhooks

| Method | Path | Description |
|--------|------|-------------|
| POST | `/webhooks/frigate` | Receive Frigate NVR events (requires `FRIGATE_WEBHOOK_SECRET`) |
| POST | `/webhooks/homeassistant` | Receive Home Assistant events (requires `HASS_WEBHOOK_SECRET`) |

### PostgreSQL-only (feature `postgres-storage`)

| Method | Path | Description |
|--------|------|-------------|
| GET | `/entities` | Entity search (knowledge graph) |
| GET | `/retrieval/runs` | List retrieval trajectory runs |
| GET | `/retrieval/runs/{id}` | Get specific retrieval run details |
| GET | `/retrieval/runs/{id}/trajectory` | Get full trajectory for a retrieval run |
| POST | `/memories/{id}/compact` | Compact/consolidate a memory node |

---

## Key Concepts

### Fractal Retrieval (`POST /retrieve_fractal`)

The primary search endpoint. Request body:

```json
{
  "query_text": "What did we decide about the API?",
  "query_vector": [0.1, 0.2, ...],    // optional, auto-embeds if omitted
  "top_k": 10,
  "temporal_weight": 0.5,              // 0.0 = pure semantic, 0.8 = heavy recency
  "diversity": true,                   // enable temporal diversity sampling
  "user_id": "user-123",               // optional, scopes to specific user
  "source_type_weights": {             // optional, overrides server defaults
    "real": 1.0,
    "synthetic": 0.85,
    "derived": 0.70,
    "unknown": 0.95
  }
}
```

### Source-Type Weighting

Nodes are classified by provenance:
- **Real** (1.0×) — Human-authored, conversation-derived
- **Synthetic** (0.85×) — AI-generated, consolidation artifacts
- **Derived** (0.70×) — Summaries, auto-extractions
- **Unknown** (0.95×) — Missing provenance — tiny penalty

Configure defaults via `KNOWWHERE_SOURCE_TYPE_WEIGHTS` env var, `source_weights.json`, or per-query in the request body.

### Turn-Level Storage

Every conversation turn gets its own embedding with `EmbeddingInfo` metadata (provider, dimension, vector). Queries can retrieve at turn granularity — no more session-level aggregation loss.

### Cross-Encoder Reranking

The `/rerank` endpoint runs gte-modernbert (ONNX, 599 MB) over a candidate list. No Ollama dependency — pure ONNX inference.

---

## Configuration

| Env Var | Default | Description |
|---------|---------|-------------|
| `KNOWWHERE_API_KEY` | (none) | API key for bearer auth |
| `KNOWWHERE_PORT` | 3737 | Server listen port |
| `KNOWWHERE_TEMPORAL_WEIGHT` | none | Server-wide recency weight (0.0–0.8) |
| `KNOWWHERE_SOURCE_TYPE_WEIGHTS` | none | JSON: `{"real":1.0,...}` |
| `KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE` | none | Path to weights JSON file |
| `OLLAMA_EMBEDDING_MODEL` | nomic-embed-text | Embedding model name |
| `OLLAMA_EMBEDDING_DIMENSION` | 768 | Embedding vector dimension |
| `FRIGATE_URL` | none | Enable Frigate NVR connector |
| `FRIGATE_WEBHOOK_SECRET` | none | Frigate webhook authentication |
| `HASS_WEBHOOK_SECRET` | none | Home Assistant webhook auth |
| `DREAM_ENABLED` | true | Enable Dream Mode scheduler |

See `ARCHITECTURE_MAP.md` for module-level architecture and `.env.example` for all available settings.

# KnowWhere Soul

> **Identity:** Fractal Memory System for AI Agents — every fact has an address.
> **Version:** v0.6.0
> **Server:** http://localhost:3737 (Production) / http://localhost:3739 (Dev)

---

## Identity

KnowWhere is a **lossless fractal memory system** for AI agents. Unlike vector databases
that flatten memory into a single similarity space, KnowWhere maintains memory across
three fractal tiers:

- **L0** — Raw, high-fidelity session memory (full text + embeddings)
- **L1** — Consolidated facts extracted from sessions (VLM-gated, cross-referenced)
- **L2** — Abstracted semantic knowledge (requires OpenAI API key for VLM consolidation)

### Core Personality

- **Pointer-First:** Every fact is stored with a pointer to its origin. Retrieval always
  returns source provenance — "where did I learn this?" is a first-class question.
- **Fractal, not flat:** Memory is hierarchically compressed. L0 is raw, L1 is curated,
  L2 is abstracted.
- **Lossless by default:** Facts are never deleted; they decay in energy and can be
  re-energized by access.
- **Self-healing:** The system detects inconsistencies (conflicts, energy decay) and
  schedules Dream-mode consolidation automatically via a background scheduler.

---

## Memory Architecture

### Fractal Tiers

| Tier | Storage | TTL | Consolidation |
|------|---------|-----|---------------|
| **L0** | PostgreSQL `memory_items` + pgvector index | `L0_TTL` (default: 30d raw, indefinite with embeddings) | Auto-promoted to L1 via Dream consolidation |
| **L1** | PostgreSQL, same table, `tier=1` | `L1_TTL` (default: 90d) | VLM-gated promotion to L2 |
| **L2** | PostgreSQL, `tier=2`, semantic abstraction | `L2_TTL` (default: indefinite) | Requires OPENAI_API_KEY for VLM consolidation |

### Consolidation

Background process (`ConsolidationScheduler`) runs every `CONSOLIDATION_INTERVAL_SECS`
(default: 3600 = 1 hour):

1. **Energy Decay** — Unaccessed memories lose energy over time
2. **Conflict Detection** — Contradictory facts are flagged
3. **Deduplication** — Near-duplicate facts are merged
4. **Tier Promotion** — L0→L1 (auto), L1→L2 (VLM-gated)

### Key Types

- `FractalNode` — Core data structure with `id`, `content`, `embedding`, `tier`, `source`
- `MemorySource` — Enum: `Session`, `External`, `Dream`, `SelfImprove`
- `MemoryType` — Enum: `Fact`, `Conversation`, `Skill`, `Preference`

---

## API Conventions

### Base URL

All endpoints are available at the root. Production server runs on `:3737`.

### Versioning (since v0.6.0)

- **Legacy endpoints** (no prefix): Deprecated, sunset **November 1, 2026**
- **v1 endpoints** (`/v1/...`): Current stable API

### Authentication

Bearer token via `Authorization: Bearer <api_key>` header. Localhost bypass available
when no API key is configured (dev mode).

### Core Endpoints

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/health` | GET | No | Health check + node count |
| `/metrics` | GET | No | Prometheus metrics (histograms: method + path + status) |
| `/register` | POST | No | Register new API user |
| `/login` | POST | No | Get JWT token |
| `/refresh` | POST | Yes | Refresh JWT token |
| `/embed` | POST | Yes | Generate embedding for text |
| `/store_session` | POST | Yes | Store a session transcript (chunked, embedded, tiered) |
| `/store_external` | POST | Yes | Store external memory (URLs, notes, docs) |
| `/retrieve/{id}` | GET | Yes | Fetch a single node by ID |
| `/retrieve_fractal` | POST | Yes | Fractal retrieval with hybrid BM25+Dense search |
| `/nodes/recent` | GET | Yes | Recent nodes |
| `/nodes/purge_dummy` | POST | Yes | Remove test/dummy nodes |
| `/nodes/reembed_all` | POST | Yes | Re-embed all nodes |
| `/dream/status` | GET | Yes | Dream scheduler status |
| `/events` | POST | Yes | Get event stream |
| `/governance/policy` | GET/POST | Yes | Get/set governance policy |

### OpenAPI Documentation

Available at `/docs` (Swagger UI) and `/openapi.json` (spec).

---

## OpenClaw Integration

KnowWhere serves as a memory provider for **OpenClaw** (Hermes Agent's plugin system)
via the `openclaw-plugin/` directory.

### Implemented Hooks

| Hook | Description |
|------|-------------|
| `before_prompt_build` | Injects relevant memories into the agent's context before each turn |
| `agent_end` | Stores conversation turns as L0 memories after each agent response |
| `before_compaction` | Preserves high-energy memories before context window compaction |

### Plugin Directory

`openclaw-plugin/` — TypeScript plugin using the OpenClaw SDK. Connects to KnowWhere
via REST API (default: `http://localhost:3737`).

---

## Configuration

### Required Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgres://localhost:5432/knowwhere` | PostgreSQL connection string |
| `OLLAMA_URL` | `http://localhost:11434` | Ollama server for embeddings |
| `OLLAMA_MODEL` | `nomic-embed-text` | Default embedding model |
| `DREAM_ENABLED` | `false` | Enable Dream-mode consolidation |
| `CONSOLIDATION_INTERVAL_SECS` | `3600` | How often consolidation runs |
| `AUDIT_INTERVAL_SECS` | `86400` | How often audit runs |

### L2 Consolidation (VLM)

To enable L1→L2 promotion, you need:
- `OPENAI_API_KEY` — OpenAI API key for GPT-4V/VLM-based consolidation
- Alternatively: `GROK_API_KEY` for Grok-based VLM

Without a VLM key, L1→L2 promotion is skipped. L0→L1 still works (uses local Ollama
summarizer via `qwen2.5:3b`).

---

## Known Limitations

1. **No multi-tenancy** — Single namespace model. Multiple Hermes profiles share the
   same memory space (addressed via `namespaces` but not fully isolated).
2. **No backups** — PostgreSQL is the single source of truth. No automated pg_dump.
3. **VLM dependency for L2** — Requires OpenAI/Grok API key. Without it, the top tier
   of the fractal hierarchy is unreachable.
4. **No P99 latency monitoring** — Metrics endpoint exists (v0.6.0+) but no Grafana
   dashboard or alerting is configured.
5. **Summarizer is local-only** — Uses qwen2.5:3b via Ollama. Quality is model-dependent.
6. **No horizontal scaling** — Single PostgreSQL instance. No read replicas or sharding.

---

## Schemas

### Core Tables (PostgreSQL)

- **`memory_items`** — Primary table for all memories (L0/L1/L2). Columns: `id`, `content`,
  `embedding` (pgvector), `tier`, `source`, `session_id`, `turn_index`, `energy`,
  `created_at`, `accessed_at`.
- **`namespaces`** — Logical grouping of memories. Columns: `id`, `path`, `description`.
- **`api_keys`** — API key management. Columns: `id`, `key_hash`, `user_id`, `created_at`,
  `expires_at`.
- **`auth_users`** — User accounts for JWT auth. Columns: `id`, `username`, `password_hash`,
  `created_at`.

### Embedding Dimensions

- Default: 768d (nomic-embed-text)
- Matryoshka truncation: 256d (Fractal Zoom), 64d (Fast Scan)
- Cross-encoder: 384d (gte-modernbert-base via ONNX)

---

## Testing Strategy

### Running Tests

```bash
# All tests (requires PostgreSQL for integration tests)
cargo test

# Unit tests only
cargo test --lib

# With docker-compose for PostgreSQL
docker-compose up -d postgres
cargo test
```

### Test Structure

- `src/**/` — Unit tests (`#[test]` and `#[tokio::test]` inline)
- `tests/` — Integration tests (7 files, 5,617 LOC total)
  - `integration.rs` — API endpoint integration tests (2,190 LOC)
  - `turn_storage.rs` — Turn-level storage roundtrip tests (1,717 LOC)
  - `state_management.rs` — State lifecycle tests (835 LOC)
  - `retrieval_quality.rs` — Retrieval quality benchmarks (290 LOC)
  - `openapi_contract.rs` — OpenAPI spec compliance (74 LOC)
  - `distance_matrix.rs` — Embedding distance matrix tests (254 LOC)
  - `test_soul.rs` — Soul.md documentation compliance (257 LOC)

### Coverage

Coverage tracked via `cargo tarpaulin` in CI (`.github/workflows/ci.yml`). HTML report
generated in `coverage/` directory.

---

## References

- [BUG-TRACKING.md](BUG-TRACKING.md) — Known bugs and issue tracking
- [ARCHITECTURE_MAP.md](ARCHITECTURE_MAP.md) — Module map and data flow
- [docs/reviews/2026-06-16-gstack-full-review.md](docs/reviews/2026-06-16-gstack-full-review.md) — Latest gStack review
- [openclaw-plugin/](openclaw-plugin/) — OpenClaw/Hermes plugin source

<div align="center">

# KnowWhere

### Dein KI-Gedaechtnis, das Pointer statt Rohdaten speichert.

**Pointer-first fractal memory service for AI agents.**

[![CI](https://github.com/Jind0la/knowwhere/actions/workflows/ci.yml/badge.svg)](https://github.com/Jind0la/knowwhere/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

KnowWhere is a long-term memory backend for AI agents. It stores session data as full text plus embeddings, but stores external sources as **pointers only**. Retrieval combines semantic vector search, BM25 keyword search, reciprocal rank fusion, and optional fractal zooming.

## Start Here

- **5-minute setup:** [docs/QUICKSTART.md](docs/QUICKSTART.md)
- **Full first-run walkthrough:** [docs/WALKTHROUGH.md](docs/WALKTHROUGH.md)
- **Beta scope, limitations, roadmap:** [docs/BETA-README.md](docs/BETA-README.md)
- **Product scope:** [docs/PRD.md](docs/PRD.md)
- **Technical architecture:** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

## Current main status

- Repository/package version on `main`: `0.1.0`
- Core REST API is live: store, retrieve, chat, governance, dream status, events
- Auth exposes token capabilities via `GET /auth/me`
- Retrieval profiles are enforced server-side: `user-facing`, `agent-debug`, `full-fidelity`
- Storage works in two modes: default `MemoryStore` with JSON persistence, optional `PostgresStore` behind `postgres-storage`
- React operator dashboard lives in `dashboard/` and is built in CI
- A minimal static fallback UI still exists in `frontend/`, but it is not the primary dashboard surface

## How it works

```text
User / Agent message -> store_session -> embedding + BM25 index
Next query           -> retrieve_fractal -> hybrid retrieval -> ranked context
Optional chat        -> chat/subconscious -> answer + cited sources
External data        -> store_external -> pointer + metadata only
```

## Quick start

### Local Rust server

Requires [Rust 1.85+](https://rustup.rs) and [Ollama](https://ollama.ai).

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
ollama pull nomic-embed-text-v2-moe
KNOWWHERE_API_KEY=my-secret-key cargo run
```

Open [http://localhost:3737/swagger-ui/](http://localhost:3737/swagger-ui/) for the API docs.

If you want a different local embedding model, set it explicitly before startup:

```bash
export OLLAMA_MODEL=snowflake-arctic-embed2
export OLLAMA_EMBEDDING_DIMENSION=1024
KNOWWHERE_API_KEY=my-secret-key cargo run
```

### Dashboard

The active operator UI lives in `dashboard/` and talks to the backend through Vite's `/api` proxy.

```bash
cd dashboard
npm ci
npm run dev
```

By default the dashboard proxies to `http://localhost:3737`. Override that for local testing if needed:

```bash
VITE_API_TARGET=http://localhost:3750 npm run dev
```

Open [http://localhost:5173](http://localhost:5173), paste a Bearer token into the UI, and the dashboard will load capabilities from `GET /auth/me`.

### Docker

Quick test, default build:

```bash
docker build -t knowwhere-server:local .
docker run -d --name knowwhere -p 3737:3737 \
  -e KNOWWHERE_API_KEY=my-secret-key \
  -e OLLAMA_URL=http://host.docker.internal:11434 \
  -e RUST_LOG=info \
  knowwhere-server:local
```

Persistent PostgreSQL setup via the checked-in compose file:

```bash
export KNOWWHERE_API_KEY=my-secret-key
export POSTGRES_PASSWORD=kw
docker compose up -d
```

Notes:

- `docker-compose.yml` builds with `FEATURES=postgres-storage`
- The compose file defaults `OLLAMA_MODEL` to `snowflake-arctic-embed2`
- On Linux, set `OLLAMA_URL` to a reachable host address if `host.docker.internal` is unavailable

## Pointer-first data model

- `store_session`: full text plus embedding for conversations, notes, decisions
- `store_external`: pointer string plus embedding plus metadata, never raw external payloads
- Retrieval responses intentionally omit raw vectors to keep payloads small

## Auth and retrieval profiles

Protected routes require a Bearer token whenever `KNOWWHERE_API_KEY` is set. If no key is set, KnowWhere runs with auth disabled for local development only.

`GET /auth/me` returns:

- `token_kind`: `admin` or `user`
- `allowed_retrieval_profiles`: the profiles the current token may request

Current behavior:

- **Static admin key** via `KNOWWHERE_API_KEY`: full access plus all retrieval profiles
- **Self-service user tokens** via `POST /register`, `POST /login`, `POST /refresh`: available only when built with `postgres-storage` and started with `DATABASE_URL`
- **Admin login through `/login` is intentionally disabled**. The admin key must be used directly as Bearer token

Profile access today:

- `admin` tokens: `user-facing`, `agent-debug`, `full-fidelity`
- `user` tokens: `user-facing`

## API overview

### Public

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/health` | Liveness plus node count |
| `GET` | `/swagger-ui/*` | OpenAPI / Swagger UI |
| `POST` | `/register` | Create user plus initial API key (`postgres-storage` only) |
| `POST` | `/login` | Mint session token (`postgres-storage` only) |
| `POST` | `/refresh` | Rotate session token (`postgres-storage` only) |

### Protected core memory routes

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/auth/me` | Token capabilities |
| `POST` | `/embed` | Embedding helper |
| `POST` | `/store_session` | Store full-text session memory |
| `POST` | `/store_external` | Store external pointer memory |
| `GET` | `/retrieve/{id}` | Fetch a single node |
| `POST` | `/retrieve_fractal` | Hybrid retrieval |
| `POST` | `/chat/subconscious` | Retrieval-backed chat response with sources |
| `GET` | `/nodes/recent` | Recent nodes |
| `POST` | `/nodes/reembed_all` | Re-embed all nodes with the active provider |
| `GET` | `/dream/status` | Dream-mode status |
| `GET` | `/events` | Event stream snapshot |
| `GET` / `POST` | `/governance/policy` | Read / update governance policy |

### Protected Postgres-only routes

When `postgres-storage` is enabled and a working `DATABASE_URL` is present, KnowWhere also exposes:

- retrieval analytics: `/retrieval/runs`, `/retrieval/runs/{id}`, `/retrieval/runs/{id}/trajectory`
- lifecycle operations: `/memories/{id}`, `/memories/{id}/compact`, `/memories/{id}/energy/boost`
- energy management: `/energy/low`, `/energy/decay`, `/energy/compress`
- deduplication and conflicts: `/deduplication/*`, `/conflicts/*`
- self-healing: `/memories/{id}/reindex`, `/memories/{id}/health`, `/self-healing/stats`
- namespaces and skills: `/namespaces/*`, `/skills/*`

## Embedding providers

Selection order at runtime:

1. `KNOWWHERE_EMBEDDING_PROVIDER` if explicitly set
2. Grok when `GROK_API_KEY` is present and the `grok-provider` feature is enabled
3. OpenAI when `OPENAI_API_KEY` is present and the `openai-provider` feature is enabled
4. Local Ollama otherwise

Local Ollama details:

- default model in code: `nomic-embed-text-v2-moe`
- override model with `OLLAMA_MODEL`
- override dimension with `OLLAMA_EMBEDDING_DIMENSION`
- override base URL with `OLLAMA_URL`

## Storage modes

### Default mode

- Backend: `MemoryStore`
- Persistence: JSON state under `KNOWWHERE_DATA_DIR`
- Good for local development and single-node testing

### PostgreSQL mode

- Build with `--features postgres-storage`
- Start with a working `DATABASE_URL`
- Enables `PostgresStore`, auth-backed user tokens, analytics, deduplication, conflicts, energy management, self-healing, namespaces, and skills

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `KNOWWHERE_PORT` | `3737` | HTTP port |
| `KNOWWHERE_API_KEY` | unset | Static admin Bearer token; if unset, auth is disabled |
| `KNOWWHERE_DATA_DIR` | `./data` | JSON persistence directory |
| `DATABASE_URL` | unset | Enables PostgreSQL-backed runtime when compiled with `postgres-storage` |
| `KNOWWHERE_EMBEDDING_PROVIDER` | unset | Force `ollama`, `openai`, or `grok` selection |
| `OLLAMA_URL` | `http://localhost:11434` | Local Ollama base URL |
| `OLLAMA_MODEL` | `nomic-embed-text-v2-moe` | Local embedding model |
| `OLLAMA_EMBEDDING_DIMENSION` | unset | Manual embedding dimension override |
| `OLLAMA_VLM_MODEL` | `llama3.2` | Ollama VLM model for summarization worker |
| `OPENAI_API_KEY` | unset | OpenAI embeddings |
| `GROK_API_KEY` | unset | Grok/xAI embeddings |
| `FRIGATE_URL` | unset | Enables Frigate connector |
| `AUTH_SESSION_TTL_DAYS` | `30` | Session token lifetime in PostgreSQL auth mode |
| `AUTH_STRICT_MIGRATIONS` | `false` | Fail startup on auth migration problems |
| `RATE_LIMIT_MODE` | `off` | Set to `proxy` behind a reverse proxy |
| `RATE_LIMIT` | unset | Legacy fallback that behaves like `RATE_LIMIT_MODE=proxy` |
| `RUST_LOG` | `info` | Tracing verbosity |

## Integration rules

KnowWhere is additive, never destructive:

1. Import existing memories first
2. Keep original host files untouched
3. Append to host configuration instead of replacing it
4. Let the host memory system continue to run in parallel
5. Degrade gracefully if KnowWhere is offline

## SDK and integrations

- Python SDK: `sdk/python`
- OpenClaw plugin: `openclaw-plugin/`
- Import guide: [docs/IMPORT_GUIDE.md](docs/IMPORT_GUIDE.md)

## CI

`/.github/workflows/ci.yml` currently validates:

- `cargo fmt`, `cargo clippy`, `cargo check`, `cargo test --lib`
- OpenAPI contract smoke tests
- PostgreSQL integration tests with `pgvector` plus local Ollama
- feature-matrix builds for `openai-provider`, `grok-provider`, and PostgreSQL combinations
- `dashboard` production build
- Docker image build

## Build matrix

| Feature flag | Effect |
|--------------|--------|
| default | `MemoryStore` plus local Ollama |
| `postgres-storage` | PostgreSQL storage and the extended memory lifecycle routes |
| `openai-provider` | OpenAI embeddings |
| `grok-provider` | Grok/xAI embeddings |

Examples:

```bash
cargo build
cargo build --features postgres-storage
cargo build --features openai-provider
cargo build --features "postgres-storage,grok-provider"
```

## Contributing

Contributions are welcome. Please open an issue or pull request on [GitHub](https://github.com/Jind0la/knowwhere).

### Git Hook Setup

This repo tracks a pre-commit hook in `scripts/pre-commit-hook.sh`. After cloning, activate it:

```bash
ln -sf scripts/pre-commit-hook.sh .git/hooks/pre-commit
```

The hook runs `cargo sqlx prepare` to keep the offline query cache up-to-date. PostgreSQL must be running on port 5433 (`docker start knowwhere-kw-postgres-1`) or you can skip the check with `git commit --no-verify`.

## License

[MIT](LICENSE) — 2026 KnowWhere contributors
# test

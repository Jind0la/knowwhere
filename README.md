<div align="center">

# KnowWhere

### Dein KI-Gedaechtnis, das nie vergisst.

**Pointer-first fractal memory service for AI agents.**

[![CI](https://github.com/NimarMoradbakhti/knowwhere/actions/workflows/ci.yml/badge.svg)](https://github.com/NimarMoradbakhti/knowwhere/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

KnowWhere is a long-term memory backend for AI agents. It stores session data (full text + embeddings) and references external data sources via **pointers only** — never raw files. It features fractal vector retrieval, a "Dream Mode" for organic cluster formation, and pluggable embedding providers (Grok, OpenAI, local Ollama).

## Screenshots

| Dashboard | Swagger UI | LangChain Example |
|:---------:|:----------:|:-----------------:|
| ![Dashboard](docs/screenshots/dashboard.png) | ![Swagger UI](docs/screenshots/swagger-ui.png) | ![LangChain](docs/screenshots/langchain-example.png) |

> Screenshots coming soon — run the server locally to explore!

## Quickstart (3 Commands)

```bash
git clone https://github.com/NimarMoradbakhti/knowwhere.git
cd knowwhere
docker compose up --build
```

Open [http://localhost:3000](http://localhost:3000) for the Dashboard and [http://localhost:3000/swagger-ui/](http://localhost:3000/swagger-ui/) for the API docs.

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
print(context)
```

## Environment Variables

Copy the example file and fill in your values:

```bash
cp .env.example .env
```

| Variable             | Required | Default        | Description                                              |
|----------------------|----------|----------------|----------------------------------------------------------|
| `KNOWWHERE_API_KEY`  | No       | *(unset)*      | If set, all routes except `/health` require Bearer token |
| `GROK_API_KEY`       | No       | *(unset)*      | Grok embedding provider API key (xAI)                    |
| `OPENAI_API_KEY`     | No       | *(unset)*      | OpenAI embedding provider API key                        |
| `RUST_LOG`           | No       | `info`         | Tracing log level (`debug`, `info`, `warn`, `error`)     |

If neither `GROK_API_KEY` nor `OPENAI_API_KEY` is set, KnowWhere falls back to a local-ollama placeholder.

## Authentication

KnowWhere uses Bearer token authentication (MVP mode).

```bash
export KNOWWHERE_API_KEY=my-secret-key-123
docker compose up --build
```

```bash
curl -H "Authorization: Bearer my-secret-key-123" http://localhost:3000/embed \
  -d '{"text":"hello"}' -H "Content-Type: application/json"
```

**Public endpoints** (no token required): `/health`, `/swagger-ui/*`

**With the Python SDK:**

```python
client = KnowWhereClient(api_key="my-secret-key-123")
```

## API Endpoints

| Method | Path                | Auth     | Description                          |
|--------|---------------------|----------|--------------------------------------|
| GET    | `/health`           | Public   | Server status + node count           |
| GET    | `/swagger-ui/`      | Public   | Interactive API documentation        |
| POST   | `/embed`            | Required | Generate embedding for text          |
| POST   | `/store_session`    | Required | Store session (full content)         |
| POST   | `/store_external`   | Required | Store external pointer (no raw data) |
| GET    | `/retrieve/{id}`    | Required | Retrieve node by ID                  |
| POST   | `/retrieve_fractal` | Required | Fractal vector search                |
| GET    | `/nodes/recent`     | Required | Recent nodes (sorted by created_at)  |
| GET    | `/dream/status`     | Required | Dream mode status                    |

## Architecture

- **Backend:** Rust (Axum 0.8, Tokio, Tower)
- **Embeddings:** Pluggable (Grok, OpenAI, local-ollama)
- **Vector Store:** USearch (cosine similarity)
- **Graph:** In-memory fractal graph with Dream Mode clustering
- **SDK:** Python 3.11+ with LangChain/LlamaIndex compatibility
- **Dashboard:** Vanilla JS + Tailwind CSS
- **Docs:** OpenAPI 3.0 via utoipa + Swagger UI
- **Principle:** Pointer-First — external data is never stored, only referenced

## Deployment

### Railway

```bash
# Install Railway CLI, then:
railway login
railway init
railway up
```

Set environment variables in the Railway dashboard. The included `Dockerfile` is detected automatically.

### Fly.io

```bash
# Install flyctl, then:
fly launch
fly secrets set KNOWWHERE_API_KEY=your-secret
fly secrets set GROK_API_KEY=your-key
fly deploy
```

Fly.io will detect the `Dockerfile` and deploy accordingly. Ensure port 3000 is exposed.

### Local (without Docker)

```bash
cargo run
```

Requires Rust 1.85+ installed via [rustup](https://rustup.rs).

## Running Tests

```bash
cargo test
```

## Contributing

Contributions are welcome! Please open an issue or pull request on [GitHub](https://github.com/NimarMoradbakhti/knowwhere).

---

> **Beta Notice**
>
> KnowWhere is currently in **Beta (v0.1.0)**. We are actively looking for early testers and feedback.
> Reach out to **@NimarMoradbakhti** on X or via email to get involved!

## License

[MIT](LICENSE) — 2026 Nimar Moradbakhti & KnowWhere Contributors

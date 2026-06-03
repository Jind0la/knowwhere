# KnowWhere Quickstart — v0.6.0

Get KnowWhere running locally in 5 minutes.

## Prerequisites

- **Rust** 1.85+ (`rustup default stable`)
- **Ollama** ([ollama.com](https://ollama.com)) with models:
  ```bash
  ollama pull nomic-embed-text    # 274 MB — embeddings
  ollama pull qwen2.5:3b          # 1.9 GB — consolidation (optional)
  ```
- **PostgreSQL 16+** (optional — only needed for persistent storage)

## 1. Clone & Configure

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
cp .env.example .env
# Edit .env — at minimum set KNOWWHERE_API_KEY
```

## 2. Start the Server

```bash
cargo run --release
# → Listening on http://localhost:3737
```

Or with just the essential features (no PostgreSQL):
```bash
cargo run --release --no-default-features
```

## 3. Verify It Works

```bash
# Health check (no auth needed)
curl http://localhost:3737/health

# Store a memory (requires API key)
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer $KNOWWHERE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "session_id": "test-1",
    "turns": [{
      "role": "user",
      "content": "I prefer Rust over Python for systems programming.",
      "turn_index": 0
    }]
  }'

# Retrieve memories
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer $KNOWWHERE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query_text": "programming language preference", "top_k": 5}'
```

## 4. Docker (Alternative)

```bash
docker compose up --build
# → PostgreSQL on :5433, KnowWhere on :3737
```

The Docker setup includes pgvector-enabled PostgreSQL automatically.
Set `DATABASE_URL` in `.env` to enable persistent storage.

## Next Steps

- **API Reference:** [docs/API_REFERENCE.md](docs/API_REFERENCE.md) — all 32 endpoints
- **Architecture:** [ARCHITECTURE_MAP.md](ARCHITECTURE_MAP.md) — module diagram
- **Walkthrough:** [docs/WALKTHROUGH.md](docs/WALKTHROUGH.md) — end-to-end guide
- **Contributing:** [CONTRIBUTING.md](CONTRIBUTING.md) — dev setup & conventions

## Troubleshooting

**"Ollama not reachable"**
→ Make sure Ollama is running: `ollama serve`

**"Embedding dimension mismatch"**
→ KnowWhere uses 768-dim embeddings. Verify: `ollama list | grep nomic-embed-text`

**"Port already in use"**
→ Set `KNOWWHERE_PORT=3738` in `.env`

**"No API key configured"**
→ Set `KNOWWHERE_API_KEY` in `.env` or all routes are public.

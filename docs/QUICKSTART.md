# KnowWhere Quickstart Guide

Get KnowWhere running and usable in under 10 minutes.

---

## What you are starting

KnowWhere is a long-term memory service for AI agents:

- session memories are stored as full text plus embeddings
- external sources are stored as pointers plus metadata only
- retrieval uses semantic search, BM25, and reciprocal rank fusion

---

## Step 1: Choose a runtime

### Option A: Local Rust server

Best for development and debugging.

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
    ollama pull snowflake-arctic-embed2
    KNOWWHERE_API_KEY=*** cargo run
```

If you want a different Ollama embedding model:

```bash
export OLLAMA_MODEL=snowflake-arctic-embed2
export OLLAMA_EMBEDDING_DIMENSION=1024
KNOWWHERE_API_KEY=my-secret-key cargo run
```

### Option B: Docker quick test

Good for a fast single-container smoke test.

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
docker build -t knowwhere-server:local .
docker run -d --name knowwhere -p 3737:3737 \
  -e KNOWWHERE_API_KEY=my-secret-key \
  -e OLLAMA_URL=http://host.docker.internal:11434 \
  -e RUST_LOG=info \
  knowwhere-server:local
```

### Option C: Docker Compose with PostgreSQL

Good when you want persistence plus the self-service auth flow.

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
export KNOWWHERE_API_KEY=my-secret-key
export POSTGRES_PASSWORD=kw
docker compose up -d
```

Notes:

- `docker-compose.yml` builds with `FEATURES=postgres-storage`
- it defaults `OLLAMA_MODEL` to `snowflake-arctic-embed2`
- on Linux, replace `host.docker.internal` with a reachable host IP if required

### Verify the server

```bash
curl http://localhost:3737/health
```

Expected shape:

```json
{"status":"ok","node_count":0}
```

API docs: [http://localhost:3737/swagger-ui/](http://localhost:3737/swagger-ui/)

---

## Step 2: Choose your auth mode

KnowWhere currently supports two practical modes.

### Mode A: Static admin key

Works in every deployment mode.

- set `KNOWWHERE_API_KEY` before startup
- use that exact value as `Authorization: Bearer ...`
- admin tokens can use all retrieval profiles

Quick capability check:

```bash
curl http://localhost:3737/auth/me \
  -H "Authorization: Bearer my-secret-key"
```

### Mode B: Self-service user token

Available only when:

- the binary was built with `postgres-storage`
- `DATABASE_URL` is configured and reachable

Register:

```bash
curl -X POST http://localhost:3737/register \
  -H "Content-Type: application/json" \
  -d '{
    "username": "beta_user",
    "email": "beta_user@example.com",
    "password": "very-secret-password"
  }'
```

Login:

```bash
curl -X POST http://localhost:3737/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "beta_user",
    "password": "very-secret-password"
  }'
```

Important:

- `/register`, `/login`, and `/refresh` return `503` when `postgres-storage` is not active
- user tokens currently get only the `user-facing` retrieval profile
- admin login through `/login` is intentionally disabled; use `KNOWWHERE_API_KEY` directly

---

## Step 3: Optional dashboard

The current operator UI is the React dashboard in `dashboard/`.

```bash
cd dashboard
npm ci
npm run dev
```

Open [http://localhost:5173](http://localhost:5173), paste your token into the UI, and the app will read `GET /auth/me` to decide which retrieval profiles to show in Search and Chat.

---

## Step 4: Store and retrieve one memory

Store:

```bash
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer my-secret-key" \
  -H "Content-Type: application/json" \
  -d '{
    "content": "My favorite color is blue",
    "memory_type": "preference",
    "metadata": { "source": "quickstart" }
  }'
```

Retrieve:

```bash
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer my-secret-key" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "what is my favorite color?",
    "top_k": 5,
    "max_depth": 3,
    "retrieval_profile": "user-facing"
  }'
```

Chat on top of retrieved memories:

```bash
curl -X POST http://localhost:3737/chat/subconscious \
  -H "Authorization: Bearer my-secret-key" \
  -H "Content-Type: application/json" \
  -d '{
    "message": "What do you know about my preferences?",
    "retrieval_profile": "user-facing",
    "persist": false
  }'
```

---

## Step 5: Connect OpenClaw

Open your OpenClaw config, usually:

```bash
~/.openclaw/openclaw.json
```

Add the KnowWhere plugin entry:

```json
{
  "plugins": {
    "allow": ["knowwhere"],
    "slots": {
      "memory": "knowwhere"
    },
    "entries": {
      "knowwhere": {
        "enabled": true,
        "config": {
          "endpoint": "http://127.0.0.1:3737",
          "apiKey": "my-secret-key",
          "autoRecall": true,
          "autoCapture": true,
          "topK": 5,
          "importLookbackDays": 7
        }
      }
    }
  }
}
```

Restart OpenClaw and verify KnowWhere has recent nodes:

```bash
curl http://localhost:3737/nodes/recent?limit=10 \
  -H "Authorization: Bearer my-secret-key"
```

---

## Common issues

### 401 or unauthorized

1. Verify the token matches the running server
2. Check capabilities via `GET /auth/me`
3. Remember: user tokens do not expose `agent-debug` or `full-fidelity`

### `503` on `/register` or `/login`

The server is not running with PostgreSQL auth support. Start it with:

- `--features postgres-storage`
- a valid `DATABASE_URL`

### Ollama connection fails

1. Check Ollama is running: `curl http://localhost:11434/api/tags`
2. Verify `OLLAMA_URL`
3. Verify the chosen model exists with `ollama list`

### First request is slow

Cold-start embedding on Ollama can take a second or two. Subsequent requests are usually faster.

---

## Next steps

- API docs: [http://localhost:3737/swagger-ui/](http://localhost:3737/swagger-ui/)
- Full walkthrough: [WALKTHROUGH.md](./WALKTHROUGH.md)
- Architecture: [ARCHITECTURE.md](./ARCHITECTURE.md)
- Import guide: [IMPORT_GUIDE.md](./IMPORT_GUIDE.md)
- OpenClaw plugin: [../openclaw-plugin/README.md](../openclaw-plugin/README.md)

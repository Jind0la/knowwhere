# KnowWhere Quickstart Guide

Get KnowWhere running and connected to OpenClaw in under 10 minutes.

---

## What is KnowWhere?

KnowWhere is a **long-term memory service** for AI agents. It stores what your agent learns and retrieves it when relevant — like giving your AI a persistent memory that never forgets.

**Use case:** You tell your agent "I prefer dark mode." Later, without any configuration, it remembers. You ask "What was I working on last week?" and it knows.

---

## Step 1: Choose Your Setup

### Option A — Docker (Recommended for Most Users)

Requires: [Docker Desktop](https://docker.com) installed and running.

**1a. Quick test (in-memory, data lost on restart):**

```bash
git clone https://github.com/NimarMoradbakhti/knowwhere.git
cd knowwhere

# Build the Docker image
docker build -t knowwhere-server:local .

# Run the server
docker run -d --name knowwhere -p 3737:3737 \
  -e KNOWWHERE_API_KEY=my-secret-key-123 \
  -e RUST_LOG=info \
  knowwhere-server:local
```

**1b. With persistent storage (PostgreSQL, data survives restarts):**

```bash
git clone https://github.com/NimarMoradbakhti/knowwhere.git
cd knowwhere

# Start KnowWhere + PostgreSQL together
docker-compose up -d
```

**Note:** `docker-compose.yml` sets `KNOWWHERE_API_KEY` from your local Docker environment. Set it first:

```bash
export KNOWWHERE_API_KEY=my-secret-key-123
docker-compose up -d
```

**Verify the server is running:**

```bash
curl http://localhost:3737/health
```

You should see JSON with `"status": "ok"` and a `node_count`.

---

### Option B — Local Development (Rust)

Requires: [Rust 1.85+](https://rustup.rs) and [Ollama](https://ollama.ai) running locally.

```bash
git clone https://github.com/NimarMoradbakhti/knowwhere.git
cd knowwhere

# Download the embedding model (one-time setup)
ollama pull snowflake-arctic-embed2

# Start the server
KNOWWHERE_API_KEY=my-secret-key-123 cargo run
```

**Verify the server is running:**

```bash
curl http://localhost:3737/health
```

---

## Step 2: Get Your API Key

KnowWhere supports two beta modes:

1) **Static admin key (works with and without PostgreSQL)**
- Set `KNOWWHERE_API_KEY` when starting the server.
- Use that exact value as `Authorization: Bearer ...` and in OpenClaw `apiKey`.

2) **Self-service users (requires `postgres-storage` + `DATABASE_URL`)**
- `/register`, `/login`, `/refresh` are available only in this mode.
- API keys are persisted in PostgreSQL.

Register/login example:

```bash
curl -X POST http://localhost:3737/register \
  -H "Content-Type: application/json" \
  -d '{
    "username": "beta_user",
    "email": "beta_user@example.com",
    "password": "very-secret-password"
  }'

# response contains: api_key, user_id, message

curl -X POST http://localhost:3737/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "beta_user",
    "password": "very-secret-password"
  }'

# response contains: token
```

If `postgres-storage` is disabled, these endpoints return `503 Service Unavailable`.


**API documentation:** Open [http://localhost:3737/swagger-ui/](http://localhost:3737/swagger-ui/) in your browser.

---

## Step 3: Configure OpenClaw Plugin

The OpenClaw plugin connects your AI agent to KnowWhere so memories are stored and retrieved automatically.

**1. Find your OpenClaw config file:**

```bash
# Usually at:
~/.openclaw/openclaw.json
```

**2. Add the KnowWhere plugin configuration:**

Open `openclaw.json` and add the `plugins` section inside the existing JSON. Your file should look like:

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
          "apiKey": "my-secret-key-123",
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

**Config options explained:**

| Option | What it does | Default |
|--------|-------------|---------|
| `endpoint` | Where KnowWhere is running | `http://127.0.0.1:3737` |
| `apiKey` | Your KnowWhere API key | (empty) |
| `autoRecall` | Retrieve memories before each AI response | `true` |
| `autoCapture` | Store conversations automatically | `true` |
| `topK` | Max memories to retrieve per query | `5` |
| `importLookbackDays` | How many days of history to import on startup | `7` |

**3. Restart OpenClaw gateway:**

```bash
openclaw gateway restart
```

**4. Verify the plugin is connected:**

```bash
# Check KnowWhere has memories
curl -H "Authorization: Bearer my-secret-key-123" http://localhost:3737/nodes/recent

# Should return JSON with your memories
```

---

## Step 4: Test It

Try asking your OpenClaw agent something that references what you told it before. For example:

```
You: Remember that my cat is named Miau.
Agent: (confirms it)

You: What's the name of my cat?
Agent: Your cat is named Miau.
```

Or test directly via API:

```bash
# Store a memory
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer my-secret-key-123" \
  -H "Content-Type: application/json" \
  -d '{"content": "My favorite color is blue", "metadata": {"source": "test"}}'

# Retrieve it
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer my-secret-key-123" \
  -H "Content-Type: application/json" \
  -d '{"query_text": "what is my favorite color?"}'
```

---

## Step 5: Import Existing Memories (Optional)

If you have an existing OpenClaw setup with memories, import them:

```bash
# Check what memories exist
curl -H "Authorization: Bearer my-secret-key-123" \
  http://localhost:3737/nodes/recent?limit=10
```

The OpenClaw plugin imports the last 7 days of sessions automatically on startup (`importLookbackDays: 7`).

For full import of existing workspace files, see [docs/IMPORT_GUIDE.md](./IMPORT_GUIDE.md).

---

## Common Issues

### "Connection refused" or 401 errors

1. Check KnowWhere is running: `curl http://localhost:3737/health`
2. Check your API key matches between `KNOWWHERE_API_KEY` env var and the OpenClaw plugin config
3. Restart KnowWhere if needed: `docker restart knowwhere`

### Ollama embedding is slow (first query)

Cold-start embedding takes 1–3 seconds. This is normal — subsequent queries are faster. The plugin has a 5-second timeout to handle this.

### Docker: port already in use

If port 3737 is busy, change the mapping:

```bash
docker run -d --name knowwhere -p 3738:3737 ...
```

Then update the OpenClaw config endpoint to `http://127.0.0.1:3738`.

### No memories retrieved

1. Check the gateway log: `tail ~/.openclaw/logs/gateway.log | grep knowwhere`
2. Verify the plugin registered: look for `registered: before_prompt_build` in the log
3. Try storing a test memory and retrieving it via curl (Step 4 above)

---

## Next Steps

- **API docs:** [http://localhost:3737/swagger-ui/](http://localhost:3737/swagger-ui/)
- **Architecture overview:** [ARCHITECTURE.md](./ARCHITECTURE.md)
- **OpenClaw plugin details:** [openclaw-plugin/README.md](../openclaw-plugin/README.md)
- **Importing existing memories:** [IMPORT_GUIDE.md](./IMPORT_GUIDE.md)

# KnowWhere Memory Plugin for OpenClaw

Fractal memory layer that gives OpenClaw agents persistent, retrieval-augmented context across sessions — powered by [KnowWhere](https://github.com/NimarMoradbakhti/knowwhere).

## What It Does

The plugin implements a **triple-store loop**:

```
┌─────────────────────────────────────────────────────────────────┐
│                     OpenClaw Agent Loop                          │
│                                                                  │
│  1. message_received (gateway mode only)                        │
│         ↓ store incoming message to KnowWhere                   │
│  2. before_prompt_build  ──────────────────────────────────┐    │
│         ↓ retrieve relevant memories from KnowWhere        │    │
│         ↓ inject as ## Relevant Memories section            │    │
│         ↓                                                 ↓    │
│  3. LLM call → Agent responds                          inject │    │
│         ↓                                                 │    │
│  4. before_reset / session_end ───────────────────────────┘    │
│         ↓ store session transcript to KnowWhere                  │
│  5. gateway_start (on daemon startup)                           │
│         ↓ import recent sessions from last N days               │
└─────────────────────────────────────────────────────────────────┘
```

## Zero-Friction Beta Setup

### Step 1 — Start KnowWhere

```bash
git clone https://github.com/NimarMoradbakhti/knowwhere.git
cd knowwhere

# Option A: Docker (recommended — one command, fully self-contained)
docker compose up --build

# Option B: Native (requires Ollama installed)
cargo run --features postgres-storage
```

KnowWhere will be running at `http://localhost:3737`.

### Step 2 — Register (no account needed)

Self-serve beta onboarding — no API key required upfront:

```bash
curl -X POST http://localhost:3737/auth/register -H "Content-Type: application/json" -d '{}'
# Response: { "api_key": "kw_Ab3x...", "message": "..." }
```

Save the returned `api_key` — it is shown only once.

### Step 3 — Configure the Plugin

Add to `~/.openclaw/openclaw.json`:

```json
{
  "plugins": {
    "slots": {
      "memory": "knowwhere"
    },
    "entries": {
      "knowwhere": {
        "enabled": true,
        "config": {
          "endpoint": "http://localhost:3737",
          "apiKey": "kw_Ab3x...",        // ← your key from Step 2
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

### Step 4 — Enable and Restart

```bash
openclaw plugins enable knowwhere
openclaw gateway restart
```

Done. Talk to your agent and it will remember context across sessions.

## Requirements

- **OpenClaw** `>= 2026.3.24`
- **KnowWhere** running on a reachable host (`http://localhost:3737` by default)
- **Ollama** running locally for embeddings (or use an external embedding provider)

## Installation

### Option 1: Local Extension (Recommended)

```bash
ln -s /path/to/knowwhere/openclaw-plugin ~/.openclaw/extensions/knowwhere
```

### Option 2: npm Install (Future)

```bash
npm install -g @nimar/knowwhere
```

## Configuration

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `endpoint` | `string` | `http://127.0.0.1:3737` | KnowWhere API base URL |
| `apiKey` | `string` | `""` | Bearer token for KnowWhere auth |
| `autoRecall` | `boolean` | `true` | Retrieve memories before each prompt |
| `autoCapture` | `boolean` | `true` | Store transcripts after each agent run |
| `topK` | `number` | `5` | Max memories to retrieve per query |
| `importLookbackDays` | `number` | `7` | Import sessions from the last N days on gateway startup |
| `minSessionSizeBytes` | `number` | `200` | Skip sessions smaller than this on import (filters out heartbeat/noise) |

## How Memory Retrieval Works

1. `before_prompt_build` fires before every LLM call
2. The plugin extracts the user query from `event.prompt`
3. Sends the query to KnowWhere `/retrieve_fractal` with `query_text` + `top_k`
4. KnowWhere returns semantically similar nodes (hybrid: embeddings + BM25)
5. The plugin injects them as:

```
## Relevant Memories
[Memory 1]
<node content>

[Memory 2]
<node content>
```

6. This is returned as `prependContext` — injected into the system prompt for this turn only

## How Memory Storage Works

**Gateway mode** (`message_received`):
- Each incoming user message is stored immediately
- Works for Telegram, Discord, and other channel integrations

**Session transitions** (`session_end`, `before_reset`):
- When a session ends or is reset, the session file is read
- All user messages are parsed from the JSONL session file
- Stored to KnowWhere with metadata: `session_id`, `role`, `source`

**Daemon startup** (`gateway_start`):
- On gateway startup, recent sessions (last N days) are imported
- Ensures the agent has context from past sessions immediately

## API Keys Setup

Beta testers get an API key from the self-serve `/auth/register` endpoint. The admin key (set via `KNOWWHERE_API_KEY` env var) is only needed for server operators.

```bash
# Get your beta tester key
curl -X POST http://localhost:3737/auth/register -H "Content-Type: application/json" -d '{}'
# Save the api_key from the response

# Use it to call protected endpoints
curl -H "Authorization: Bearer kw_Ab3x..." http://localhost:3737/retrieve_fractal \
  -X POST -H "Content-Type: application/json" \
  -d '{"query_text": "what was I working on?", "top_k": 5}'
```

## Troubleshooting

### "Hook never fired" / No memories retrieved

- Ensure OpenClaw `>= 2026.3.24` is installed
- Check that the plugin is enabled: `openclaw plugins list`
- Check the log: `tail ~/.openclaw/logs/gateway.log | grep knowwhere`
- Verify KnowWhere is running: `curl http://localhost:3737/health`

### 401 Unauthorized from KnowWhere

- Beta testers: you need to call `/auth/register` first to get a key
- Server operators: KnowWhere was started without `KNOWWHERE_API_KEY` or with a different key

### KnowWhere won't start in Docker (Ollama not reachable)

- On **macOS/Windows**: `host.docker.internal` is pre-configured — no changes needed
- On **Linux**: Add `--add-host=host.docker.internal:host-gateway` to the docker run command, or set `OLLAMA_API_URL=http://172.17.0.1:11434`

## Architecture Notes

- **Plugin Kind**: `memory` — occupies the `memory` slot in OpenClaw's plugin system
- **Non-blocking**: File reads are async and non-blocking
- **Best-effort storage**: KnowWhere failures are logged but don't crash the agent loop
- **API timeout**: 5 s per call — Ollama cold-start embedding can take 1–3 s

## Test Report (2026-03-29)

|| Test | Result |
||------|--------|
| `before_prompt_build` fires | ✅ |
| `session_end` / `before_reset` fires | ✅ |
| All 6 hooks registered | ✅ |
| KnowWhere `/store_session` stores | ✅ |
| KnowWhere `/retrieve_fractal` finds | ✅ |
| KnowWhere auth (Bearer token) | ✅ |
| Self-serve `/auth/register` generates key | ✅ |

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
│  3. LLM call → Agent responds                          inject │
│         ↓                                                 │    │
│  4. before_reset / session_end ───────────────────────────┘    │
│         ↓ store session transcript to KnowWhere                  │
│         ↓ (saves before context is lost on reset/switch)       │
│  5. gateway_start (on daemon startup)                           │
│         ↓ import recent sessions from last N days               │
└─────────────────────────────────────────────────────────────────┘
```

## Requirements

- **OpenClaw** `>= 2026.3.24` (required for `before_prompt_build` + `prependContext`)
- **KnowWhere** running on a reachable host (`http://127.0.0.1:3737` by default)
- **KnowWhere API key** if authentication is enabled on the KnowWhere server

## Installation

### Option 1: Local Extension (Recommended for Development)

```bash
# Copy or symlink this directory to the OpenClaw extensions folder
ln -s /path/to/knowwhere-plugin ~/.openclaw/extensions/knowwhere

# Or copy directly
cp -r ./knowwhere-plugin ~/.openclaw/extensions/knowwhere
```

### Option 2: npm Install (Future)

```bash
npm install -g @nimar/knowwhere
```

## Configuration

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
          "endpoint": "http://127.0.0.1:3737",
          "apiKey": "your-knowwhere-api-key",
          "autoRecall": true,
          "autoCapture": true,
          "topK": 5,
          "storeOnCompaction": true
        }
      }
    }
  }
}
```

### Config Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `endpoint` | `string` | `http://127.0.0.1:3737` | KnowWhere API base URL |
| `apiKey` | `string` | `""` | API key for KnowWhere auth |
| `autoRecall` | `boolean` | `true` | Retrieve memories before each prompt |
| `autoCapture` | `boolean` | `true` | Store transcripts after each agent run |
| `topK` | `number` | `5` | Max memories to retrieve per query |
| `storeOnCompaction` | `boolean` | `true` | Store transcript before context compaction |

## Usage

After installation and configuration, just talk to your OpenClaw agent normally:

```
You: What am I working on?
Agent: Based on your memory, you're working on KnowWhere, a fractal memory
       service for AI agents written in Rust with Axum, USearch, and BM25.

You: Remember: I prefer dark mode and use Vim keybindings.
Agent: Got it. Stored: dark mode, Vim keybindings. ✓
```

The plugin works silently in the background. To see what's happening:

```bash
# Watch the OpenClaw gateway log
tail -f ~/.openclaw/logs/gateway.log | grep knowwhere
```

## How Memory Retrieval Works

1. `before_prompt_build` fires before every LLM call
2. The plugin extracts the user query from `event.prompt` (or first user message in history)
3. Sends the query to KnowWhere `/retrieve_fractal` with `query_text` + `top_k`
4. KnowWhere returns semantically similar nodes (hybrid: embeddings + BM25)
5. The plugin injects them as:

````
## Relevant Memories
[Memory 1]
<node content>

[Memory 2]
<node content>
````

6. This is returned as `prependContext` — injected into the system prompt for this turn only

## How Memory Storage Works

Two storage paths depending on mode:

**Gateway mode** (`message_received`):
- Each incoming user message is stored immediately
- Works for Telegram, Discord, and other channel integrations

**Session transitions** (`session_end`, `before_reset`):
- When a session ends or is reset, the session file is read
- All user messages are parsed from the JSONL session file
- Stored to KnowWhere with metadata: `session_id`, `role`, `source`
- This ensures conversation history is **never lost** even when OpenClaw clears context

**Daemon startup** (`gateway_start`):
- On gateway startup, recent sessions (last N days) are imported
- Ensures the agent has context from past sessions immediately

## Development

```bash
# Watch for changes and restart gateway
openclaw daemon restart

# Check plugin logs
tail -f /tmp/openclaw/openclaw-$(date +%Y-%m-%d).log | grep knowwhere

# Or use the file-based logger (in development mode)
cat /tmp/knowwhere-plugin.log
```

## API Keys Setup

KnowWhere requires an API key. Set it via environment variable when starting the KnowWhere server:

```bash
KNOWWHERE_API_KEY=your-secret-key ./target/debug/knowwhere-server
```

Then configure the same key in `openclaw.json`:

```json
"apiKey": "your-secret-key"
```

## Troubleshooting

### "Hook never fired" / No memories retrieved

- Ensure OpenClaw `>= 2026.3.24` is installed (older versions don't support `before_prompt_build` with `prependContext`)
- Check that the plugin is enabled: `openclaw plugins list`
- Check the log: `tail ~/.openclaw/logs/gateway.log | grep knowwhere`
- Verify KnowWhere is running: `curl http://127.0.0.1:3737/health`

### 401 Unauthorized from KnowWhere

- The KnowWhere server was started without `KNOWWHERE_API_KEY` or with a different key
- Restart KnowWhere with the correct key and update `openclaw.json`

### Memories retrieved but not injected in response

- The `prependContext` injection is internal to the agent's context — it won't appear in the visible chat
- To verify injection is working, ask the agent a question about something you stored and it should answer from memory

## Architecture Notes

- **Plugin Kind**: `memory` — occupies the `memory` slot in OpenClaw's plugin system
- **Config Schema**: Uses `@sinclair/typebox` for runtime validation
- **Non-blocking**: File reads in `before_compaction` are async and non-blocking
- **Best-effort storage**: KnowWhere failures are logged but don't crash the agent loop
- **Prompt caching**: `prependContext` is per-turn (not cached). For static plugin guidance, use `prependSystemContext` instead
- **API timeout**: 5 s per call — Ollama cold-start embedding can take 1–3 s; 500 ms was too short

## Test Report (2026-03-27)

### Verified Working

| Test | Result | Evidence |
|------|--------|----------|
| `before_prompt_build` fires | ✅ | Log: `retrieved 1 memories for "[Fri 2026-03-27 14:57 GMT+1] What am I working on..."` |
| `session_end` / `before_reset` fires | ✅ | Log: `session_end: storing N messages` |
| All 6 hooks registered | ✅ | Log: `registered: before_prompt_build, message_received, session_start, before_reset, gateway_start, session_end` |
| KnowWhere `/store_session` stores | ✅ | HTTP 201, node_count went from 2 → 3 |
| KnowWhere `/retrieve_fractal` finds | ✅ | Retrieved memory about "cat named Miau" by query |
| KnowWhere auth (Bearer token) | ✅ | HTTP 201 with `Authorization: Bearer ***` |

### Bugs Fixed

1. **API timeout was 500 ms** (far too short for Ollama cold-start embedding, which takes 1–3 s). Fixed: added `KW_TIMEOUT_MS = 5000` with `AbortController`-based timeout in both `kwRetrieve` and `kwStore`.

2. **Silent failure on store errors**. The `agent_end` catch block was empty. Fixed: added `console.error` for non-AbortError exceptions and HTTP error responses.

3. **`console.error` appearing as ERROR log level** in gateway.log. Every `[knowwhere] registered` line appeared twice — once as `INFO` (correct, via `api.logger.info`) and once as `ERROR` (incorrect, via `console.error`). Root cause: OpenClaw intercepts `console.error` and routes it to the ERROR channel. Fixed: use `console.error` only for genuine error conditions; informational messages use `api.logger.info`.

### Open Issues

- **Gateway restart requires Node 22** (`openclaw gateway restart` → `Node.js v22.12+ required, current: v18.16.1`). Fix: install Node 22 via `nvm install 22 && nvm use 22`. The running gateway (PID 99985) still runs old code until next restart.

## License

MIT / Nimar's project

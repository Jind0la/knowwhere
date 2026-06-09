# KnowWhere Hermes MemoryProvider — Implementation Plan

> `~/.hermes/plugins/knowwhere/` — Native Hermes Memory Plugin
> Author: Hermes + Nimar | Date: 2026-05-03

---

## 1. Architecture Overview

```
┌──────────────────────────────────────────────────────┐
│ HERMES AGENT LOOP (run_agent.py)                     │
│                                                      │
│  MemoryManager                                       │
│  ├─ BuiltinMemoryProvider (MEMORY.md / USER.md)      │
│  └─ KnowWhereProvider (THIS PLUGIN)                  │
│       │                                              │
│       ├─ initialize(session_id)                      │
│       │   └─ Generate turn-counter, health-check     │
│       │                                              │
│       ├─ prefetch(query)        ← BEFORE each LLM    │
│       │   └─ POST /retrieve_fractal → format → str   │
│       │                                              │
│       ├─ sync_turn(user, asst)  ← AFTER each turn    │
│       │   └─ POST /store_session × 2 (crash-safe!)   │
│       │                                              │
│       └─ shutdown()                                  │
│           └─ Flush queue, close connections           │
└──────────────────────────────────────────────────────┘
```

**Key insight:** Hermes calls `sync_turn` AFTER EACH turn, not after the session. This is inherently crash-safe — we designed KnowWhere's `session_id` + `turn_index` exactly for this pattern.

---

## 2. File Structure

```
~/.hermes/plugins/knowwhere/
├── __init__.py        # KnowWhereProvider(MemoryProvider) + register(ctx)
├── plugin.yaml        # Metadata, dependencies, hooks
└── cli.py             # hermes knowwhere setup | status | test
```

No external Python dependencies — we use `urllib.request` from stdlib to avoid pip install requirements.

---

## 3. Configuration

### plugin.yaml
```yaml
name: knowwhere
version: 1.0.0
description: "KnowWhere — lossless fractal memory for AI agents. Stores full transcripts and retrieves via hybrid vector+keyword search with fractal zoom."
requires_env: []
hooks:
  - sync_turn
  - prefetch
  - on_session_end
  - on_pre_compress
```

### Config Schema (get_config_schema)
| Key | Description | Secret | Default |
|-----|-------------|--------|---------|
| `endpoint` | KnowWhere API URL | No | `http://127.0.0.1:3737` |
| `api_key` | KnowWhere API key | Yes | `kw_testkey_12345` |
| `top_k` | Memories to retrieve per query | No | `5` |
| `auto_recall` | Inject memories before LLM calls | No | `true` |
| `auto_capture` | Store turns to KnowWhere | No | `true` |

Config stored in `~/.hermes/config.yaml` under `memory.providers.knowwhere`.

## 3.1 Current Hardened Runtime Contract

The active plugin lives outside this repository at `~/.hermes/plugins/knowwhere/__init__.py`.
As of 2026-05-05 the hardened contract is:

- `prefetch()` sends raw user-facing retrieval with `reflect=false` by default.
- `prefetch()` also performs a separate `memory_type_filter=decision` request, but only injects nodes whose response type is actually `decision`.
- `<knowwhere_reflect>`, `<knowwhere_memory>`, and `memory_type=meta` are filtered before Hermes prompt injection.
- Stored Hermes turns include `session_id`, `turn_index`, `role`, `agent`, `source_system`, `observed_at`, and `claim_scope`.
- On initialization the plugin stores a current-state observation that KnowWhere is active as Hermes memory provider.
- Retrieved context is background evidence, not an authority above the current user instruction or live observations.

To sync the active runtime plugin after checkout:

```bash
mkdir -p ~/.hermes/plugins/knowwhere
cp hermes-plugin/knowwhere/__init__.py ~/.hermes/plugins/knowwhere/__init__.py
```

The repository mirror lives at `hermes-plugin/knowwhere/__init__.py`.

---

## 4. KnowWhereProvider Class Design

### 4.1 `is_available() → bool`
```python
def is_available(self):
    """Check if KnowWhere server is reachable."""
    try:
        req = urllib.request.Request(f"{self.endpoint}/health")
        with urllib.request.urlopen(req, timeout=5) as resp:
            data = json.loads(resp.read())
            return data.get("status") == "ok"
    except Exception:
        return False
```

### 4.2 `initialize(session_id, **kwargs)`
```python
def initialize(self, session_id, **kwargs):
    self._session_id = session_id
    self._turn = 0
    self._turn_lock = threading.Lock()  # Thread-safe turn counter

    # Persist turn counter to disk so it survives Hermes restarts
    self._state_file = Path(kwargs["hermes_home"]) / "plugins" / "knowwhere" / "state.json"
    self._load_state()

    # Verify KnowWhere is reachable
    if not self.is_available():
        logger.warning("KnowWhere not reachable at %s — plugin disabled", self.endpoint)
        self._enabled = False
```

**Edge case:** If Hermes restarts mid-session, the `session_id` changes. We detect this via `on_session_switch()` and reset the turn counter.

### 4.3 `prefetch(query, *, session_id="") → str`
```python
def prefetch(self, query, *, session_id=""):
    """Called before each LLM call. Retrieve relevant memories from KnowWhere."""
    if not self._enabled or not self.auto_recall:
        return ""
    if not query or len(query.strip()) < 3:
        return ""

    try:
        memories = self._retrieve(query)
        if not memories:
            return ""

        # Format for injection into system prompt
        blocks = []
        for i, m in enumerate(memories):
            content = m.get("content", "")[:300]  # Truncate per memory
            score = m.get("score", 0)
            sid = m.get("metadata", {}).get("session_id", "?")
            blocks.append(f"[KW-{i+1}] (score={score:.3f}, session={sid[:12]}...)\n{content}")

        return "\n## Relevant Memories (KnowWhere)\n" + "\n\n".join(blocks)
    except Exception as e:
        logger.warning("KnowWhere prefetch failed: %s", e)
        return ""  # Graceful degradation
```

**Design decision:** Return empty string on failure — never block the LLM call. KnowWhere is additive, not critical.

### 4.4 `sync_turn(user_content, assistant_content, *, session_id="")`
```python
def sync_turn(self, user_content, assistant_content, *, session_id=""):
    """Called after each turn. Store both messages to KnowWhere."""
    if not self._enabled or not self.auto_capture:
        return

    sid = session_id or self._session_id

    def _store():
        with self._turn_lock:
            turn_u = self._turn
            self._turn += 1
            turn_a = self._turn
            self._turn += 1
            self._save_state()

        # Store user message
        self._store_message(user_content, sid, turn_u, role="user", trust="primary")
        # Store assistant response
        self._store_message(assistant_content, sid, turn_a, role="assistant", trust="derived")

    # Fire-and-forget — don't block the agent loop
    threading.Thread(target=_store, daemon=True).start()
```

**Design decision:** Background thread with fire-and-forget. The agent loop must not wait for HTTP calls. If KnowWhere is down, turns are lost — but the agent keeps working.

### 4.5 Helper methods

```python
def _retrieve(self, query):
    """POST /retrieve_fractal"""
    data = json.dumps({"query_text": query, "top_k": self.top_k}).encode()
    req = urllib.request.Request(f"{self.endpoint}/retrieve_fractal", data=data,
        headers={"Content-Type": "application/json",
                 "Authorization": f"Bearer {self.api_key}"},
        method="POST")
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())

def _store_message(self, content, session_id, turn_index, role, trust):
    """POST /store_session"""
    payload = json.dumps({
        "content": f"[{role}] {content[:4000]}",  # Truncate long messages
        "session_id": session_id,
        "turn_index": turn_index,
        "source": "conversation",
        "memory_type": "episodic",
        "metadata": {"role": role, "trust_tier": trust, "agent": "hermes"}
    }).encode()
    req = urllib.request.Request(f"{self.endpoint}/store_session", data=payload,
        headers={"Content-Type": "application/json",
                 "Authorization": f"Bearer {self.api_key}"},
        method="POST")
    urllib.request.urlopen(req, timeout=10)
```

### 4.6 State persistence

```python
def _load_state(self):
    try:
        if self._state_file.exists():
            state = json.loads(self._state_file.read_text())
            self._turn = state.get("turn", 0)
            self._last_session = state.get("last_session", "")
    except Exception:
        self._turn = 0

def _save_state(self):
    try:
        self._state_file.parent.mkdir(parents=True, exist_ok=True)
        self._state_file.write_text(json.dumps({
            "turn": self._turn,
            "last_session": self._session_id,
            "updated": datetime.now(timezone.utc).isoformat()
        }))
    except Exception as e:
        logger.warning("Failed to save KnowWhere state: %s", e)
```

---

## 5. CLI (cli.py)

```python
# Commands:
#   hermes knowwhere setup    — Interactive setup wizard
#   hermes knowwhere status   — Show connection status, turn count, health
#   hermes knowwhere test     — Store + retrieve test roundtrip

def register_cli(subparsers):
    parser = subparsers.add_parser("knowwhere", help="KnowWhere fractal memory")
    sub = parser.add_subparsers(dest="knowwhere_action")

    sub.add_parser("setup", help="Configure KnowWhere connection")
    sub.add_parser("status", help="Show KnowWhere status")
    sub.add_parser("test", help="Test store+retrieve roundtrip")

def knowwhere_command(args):
    if args.knowwhere_action == "setup":
        # Walk user through endpoint, api_key, top_k config
        ...
    elif args.knowwhere_action == "status":
        # Show health, turn count, last sync
        ...
    elif args.knowwhere_action == "test":
        # Store a test message, retrieve it, show results
        ...
```

---

## 6. Error Handling & Edge Cases

| Scenario | Behavior |
|----------|----------|
| KnowWhere down at startup | `is_available()` → False. `_enabled = False`. Plugin no-ops. Hermes works normally. |
| KnowWhere crashes mid-session | `prefetch()` catches exception, returns "". `sync_turn()` catches in thread, logs warning. Agent continues. |
| Network timeout | `timeout=10` on all HTTP calls. No retry (fire-and-forget for writes). |
| Very long messages | Truncate to 4000 chars in `_store_message()`. KnowWhere server also has its own limits. |
| Session switch | `on_session_switch()` resets turn counter. Old session's turns are already stored. |
| Hermes restart | `_load_state()` recovers turn counter from disk. New `session_id` from Hermes — old turns preserved. |
| Thread safety | `threading.Lock` on turn counter. `urllib.request` is not thread-safe for connection pooling but we open fresh connections. |
| Memory pressure | Content truncated to 4000 chars before storage. Retrieval results truncated to 300 chars per memory. |

---

## 7. Measurement Integration

The `prefetch()` output is injected into the system prompt. Hermes sees it as:

```
## Relevant Memories (KnowWhere)
[KW-1] (score=0.852, session=hermes-2026-...)
[Nimar] Ok, sehr nice!! Danke für die ehrliche und meiner Meinung nach völlig richtige Einschätzung!

[KW-2] (score=0.734, session=hermes-2026-...)
[Nimar] Kannst du bitte einen deepdive in knowwhere machen...
```

The `[KW-N]` markers make the source unambiguous. Nimar evaluates each response:
- `+kw` → KnowWhere context improved the response
- `-kw` → KnowWhere context degraded the response
- `~kw` → KnowWhere context was irrelevant

Over 50+ rated responses: Precision, Helpfulness, Harm rate.

---

## 8. Files to Create

| File | Lines | Purpose |
|------|-------|---------|
| `__init__.py` | ~200 | KnowWhereProvider class + register() |
| `plugin.yaml` | ~10 | Plugin metadata |
| `cli.py` | ~80 | setup/status/test commands |

Total: ~290 lines. No external dependencies.

---

## 9. Activation Flow

```bash
# 1. Plugin is auto-discovered from ~/.hermes/plugins/knowwhere/
hermes memory status
# → knowwhere: installed ✓, not active

# 2. Configure
hermes config set memory.provider knowwhere
hermes config set memory.providers.knowwhere.endpoint http://127.0.0.1:3737
# api_key goes to .env automatically

# 3. Verify
hermes knowwhere status
# → KnowWhere v0.4.0 at http://127.0.0.1:3737
# → Health: OK (10 nodes)
# → Turns stored: 0

# 4. Use Hermes normally — every turn is now stored + retrieved
```

# KnowWhere Onboarding Review — Beta Tester Experience

**Review Date:** 2026-04-04
**Reviewer:** Research Engineer — KnowWhere Investigation & Features
**Last Update:** 2026-04-04 (Session 6 fixes applied)
**Status:** Beta-ready baseline reached; follow-up hardening remains (rate-limit policy, retention/GC policy, docs consistency)

---

## Executive Summary

KnowWhere is feature-complete for P0/P1 and onboarding has improved significantly. The remaining work is now mostly hardening and consistency: clear auth mode communication (static key vs PostgreSQL self-service), explicit rate-limit policy, and retention/GC policy for long-running beta environments.

---

## 1. Current Onboarding Flow (Zero to Working)

### Path A: Local Development (cargo run)

**Steps:**
1. `git clone https://github.com/NimarMoradbakhti/knowwhere.git && cd knowwhere`
2. Install Rust 1.85+ via rustup
3. `ollama pull nomic-embed-text-v2-moe`
4. `cargo run`
5. Open http://localhost:3737/swagger-ui/
6. (Undocumented) Register via POST /auth/register to get an API key
7. Configure openclaw.json with the key
8. Copy plugin to ~/.openclaw/extensions/knowwhere/

**Friction Points:**
- Step 6 is completely undocumented — a new user has no idea how to get an API key
- No explicit "get started with OpenClaw plugin" path in the README
- The OpenClaw plugin README is in a subdirectory and easy to miss
- No verification step to confirm the system is actually working after setup

### Path B: Docker

**Steps:**
1. `docker build -t knowwhere-server:local .`
2. `docker run -d --name knowwhere -p 3000:3000 -e KNOWWHERE_API_KEY=*** -e OPENAI_API_KEY=*** -e KNOWWHERE_PORT=3000 -e RUST_LOG=info knowwhere-server:local`
3. (If using PostgreSQL): Build with `--build-arg FEATURES=postgres-storage`, pass DATABASE_URL
4. Configure openclaw.json
5. Copy plugin to ~/.openclaw/extensions/knowwhere/

**Friction Points:**
- No `docker-compose up` — users must manually construct the docker run command
- `KNOWWHERE_API_KEY=***` placeholder — a new user doesn't know what key to use
- Port conflict: docker-compose exposes port 5433 for PostgreSQL (non-standard), README mentions 5432
- The `postgres-storage` feature is a build-time flag requiring custom docker build command
- docker-compose.yml has a malformed env var: `${KN...Y:-}` (looks like incomplete template)

### Path C: OpenClaw Plugin

**Steps:**
1. Copy or symlink `openclaw-plugin/` to `~/.openclaw/extensions/knowwhere/`
2. Add plugin config to `~/.openclaw/openclaw.json`
3. Ensure KnowWhere server is running at `http://127.0.0.1:3737`
4. (If auth enabled) Set same API key in both KnowWhere env var and openclaw.json

**Friction Points:**
- The "recommended for development" path requires manually finding the correct path
- No `npm install` command yet (marked as "Future" in README)
- No verification that the plugin actually loaded
- No mention that OpenClaw >= 2026.3.24 is required for before_prompt_build + prependContext

---

## 2. Critical Issues

### ISSUE 1 (BLOCKER): API Key Acquisition — No Flow for New Users

**Finding:** The README mentions `KNOWWHERE_API_KEY` everywhere, and the OpenClaw plugin README has an "API Keys Setup" section — but neither explains how a new external beta tester actually obtains an API key.

The server exposes:
- `POST /auth/register` — register new account
- `POST /auth/login` — login
- `POST /auth/refresh` — refresh token

**Problem:** There is no documented endpoint or flow for a beta tester to:
1. Sign up and get credentials
2. Exchange credentials for an API key
3. Use that API key in the plugin

A new user who clones the repo and follows the README has no way to generate an API key. The `/auth/register` endpoint exists but is not documented in the README's "Authentication" section. A beta tester would have to either:
- Guess the register endpoint exists
- Look at the Swagger UI (which requires running the server first)
- Read the source code

**Impact:** Complete blocker. A beta tester cannot complete setup without this knowledge.

---

### ISSUE 2 (BLOCKER): Docker-Compose Is Broken

**Finding:** The `docker-compose.yml` has a malformed environment variable:

```yaml
- KNOWWHERE_API_KEY=${KN...Y:-}
```

This is clearly a truncated/broken template for `KNOWWHERE_API_KEY`. The docker-compose file cannot be used as-is.

**Additional issues:**
- Port 5433 is exposed for PostgreSQL (non-standard; most local Postgres installs use 5432) — no explanation for why
- No `docker-compose up` command documented
- The postgres-storage feature is build-time, not runtime — users need to know to use a custom build command

**Impact:** Docker path is unusable for a new beta tester.

---

### ISSUE 3 (BLOCKER): Config/Schema Mismatch in Plugin

**Finding:** Three sources of truth for plugin config are inconsistent:

| Option | openclaw.plugin.json schema | README.md config table | index.js DEFAULT_CONFIG |
|--------|---------------------------|----------------------|------------------------|
| `importLookbackDays` | ✅ YES | ❌ NOT LISTED | ✅ YES |
| `storeOnCompaction` | ❌ NOT IN SCHEMA | ✅ YES (default: true) | ❌ NO (was in old src/index.ts) |

**Problem:** The README tells users to set `storeOnCompaction` in `openclaw.json`, but:
1. The plugin's own JSON schema (used for config validation) doesn't include `storeOnCompaction`
2. The actual running code (`index.js`) uses `importLookbackDays`, not `storeOnCompaction`
3. `importLookbackDays` IS in the schema but is NOT documented in the README

This means:
- A user following the README configures `storeOnCompaction: true`
- The plugin silently ignores it (additionalProperties: false + unknown key)
- The user thinks memories aren't being stored before compaction, but they actually can't be

The old `src/index.ts` (used for plugin development) has `storeOnCompaction` and `before_compaction` hook, while the compiled `index.js` (deployed plugin) has `importLookbackDays` and `gateway_start`. The two implementations are out of sync.

**Impact:** Beta testers will misconfigure the plugin and have no idea their settings are being ignored.

---

### ISSUE 4 (HIGH): Hook Documentation Doesn't Match Implementation

**Finding:** The README.md documents these hooks: `before_prompt_build`, `message_received`, `session_start`, `before_reset`, `gateway_start`, `session_end` (6 hooks).

The actual running `index.js` registers **7 hooks**: `before_prompt_build`, `message_received`, `session_start`, `before_reset`, `gateway_start`, `session_end`, **and `agent_end`**.

Additionally, the `before_compaction` hook exists in the old `src/index.ts` but NOT in the compiled `index.js`.

The `agent_end` hook fires after each agent run in gateway mode but NOT in embedded `--local` mode — this is mentioned in PHASE-2-STATUS.md but not in the README.

**Impact:** Beta testers may rely on hooks that don't fire in their mode (embedded vs gateway), or miss hooks that would benefit their use case.

---

### ISSUE 5 (HIGH): No Quickstart Guide Exists

**Finding:** There is no `docs/QUICKSTART.md` or similar. The README has a "Quickstart" section but it only covers running the server locally, not connecting to OpenClaw. All the actual OpenClaw integration docs are:
- In the middle of the README (lines 211-253) mixed with general API docs
- In `openclaw-plugin/README.md` (easy to miss)
- Scattered across `docs/PHASE-2-STATUS.md` and `docs/IMPORT_GUIDE.md`

A beta tester has no single document that says: "Here's everything you need to do, in order, to get KnowWhere + OpenClaw working."

**Impact:** Beta testers must piece together the flow from multiple documents, increasing chance of failure.

---

### ISSUE 6 (MEDIUM): No Health Check / Status Verification for Beta Testers

**Finding:** After a beta tester goes through setup, there is no documented way to verify everything is working. The README shows:
- `curl http://127.0.0.1:3737/health` for the server
- But no plugin-side verification

The OpenClaw plugin README mentions `tail -f ~/.openclaw/logs/gateway.log | grep knowwhere` — but this requires knowing to look at logs, and the log output format is not documented.

**Impact:** Beta testers may not know if the plugin is actually running, if memories are being stored, or if retrieval is working.

---

## 3. OpenClaw Plugin Config Analysis

### openclaw.json (from README)

```json
{
  "plugins": {
    "allow": ["knowwhere"],
    "slots": { "memory": "knowwhere" },
    "entries": {
      "knowwhere": {
        "enabled": true,
        "config": {
          "endpoint": "http://127.0.0.1:3737",
          "apiKey": "",
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

**Issues:**
- `apiKey` defaults to `""` — beta testers need to know to set this
- `importLookbackDays` is in the config but not documented in the README
- `storeOnCompaction` is documented but doesn't exist in the schema
- No `allow` array in the plugin schema — this may cause issues if OpenClaw expects different config structure

### Graceful Degradation When KnowWhere Is Down

**Finding:** The `index.js` handles KnowWhere server failures gracefully:

```javascript
// In kwRetrieve: returns [] on any error (network, 401, timeout)
// In kwStore: catches exceptions, logs, continues
// kwRetrieve: logs console.error for non-AbortError failures
```

**Assessment:** ✅ Correctly degrades. If KnowWhere is down, the plugin logs an error and continues the agent loop without crashing. This is the correct behavior for a memory layer.

### What Happens on 401 Unauthorized

**Finding:** If the API key is wrong or missing when the server has auth enabled:
- `kwRetrieve`: silently returns `[]` (no memories retrieved)
- `kwStore`: logs `store error 401: ...` via `console.error`

**Assessment:** ✅ Acceptable. The plugin won't crash, but a beta tester may not notice the 401 error logged as ERROR level. They may think "the plugin is running but memories aren't being stored" without realizing it's an auth issue.

---

## 4. Auth/API Key Flow Analysis

### Current State

The server has auth endpoints:
- `POST /auth/register` — requires username + password → returns tokens
- `POST /auth/login` — requires username + password → returns tokens
- `POST /auth/refresh` — refresh access token

The API key used in the plugin (`KNOWWHERE_API_KEY`) is a **server-side env var**, not the same as the JWT tokens from `/auth/login`.

**Finding:** There are TWO separate auth systems:
1. **JWT auth** for API users (via /auth/register, /auth/login)
2. **Static API key** for server access (via KNOWWHERE_API_KEY env var)

The plugin uses the **static API key** (`KNOWWHERE_API_KEY`), which is set as an environment variable on the server — NOT obtained through the /auth endpoints.

**Problem for beta testers:**
- A beta tester cannot use `/auth/register` to get the `KNOWWHERE_API_KEY` value
- The only way to get an API key is to have the server operator set the `KNOWWHERE_API_KEY` env var
- There is no self-service API key generation

**Impact:** For a hosted/shared beta, someone (Nimar) must generate keys and distribute them. For a self-hosted beta, users must set their own key. The self-service path is documented nowhere.

---

## 5. Docker Setup Analysis

### docker-compose.yml Issues

1. **Malformed env var**: `KNOWWHERE_API_KEY=${KN...Y:-}` — clearly broken template, should be `${KNOWWHERE_API_KEY:-}`
2. **Port 5433 vs 5432**: PostgreSQL is exposed on 5433, not standard 5432. No explanation given. Could cause confusion for users who also have local Postgres.
3. **No one-command start**: No `docker-compose up` documented in README
4. **postgres-storage feature**: Requires custom docker build command. No pre-built image available for this feature.
5. **No health check for KnowWhere service**: Only postgres has a healthcheck. If KnowWhere starts before postgres is ready, it fails.

### For a New Beta Tester

Using Docker means:
1. Build the image (no pre-built images published)
2. Know to add `--build-arg FEATURES=postgres-storage` for persistence
3. Set DATABASE_URL correctly (not documented in Docker section)
4. Manage API key distribution themselves

**Assessment:** Docker path is NOT ready for beta testers without significant documentation improvements.

---

## 6. Hook Audit (6 Required Hooks)

| Hook | In README? | In index.js? | Working per PHASE-2-STATUS? |
|------|-----------|--------------|---------------------------|
| `before_prompt_build` | ✅ | ✅ | ✅ |
| `message_received` | ✅ | ✅ | ✅ (gateway only) |
| `session_start` | ✅ | ✅ | ✅ |
| `before_reset` | ✅ | ✅ | ✅ |
| `gateway_start` | ✅ | ✅ | ✅ |
| `session_end` | ✅ | ✅ | ✅ |

**Plus extra in index.js:**
- `agent_end` — fires after agent run (gateway mode only, not embedded)
- `before_compaction` — in old src/index.ts, NOT in compiled index.js

**Assessment:** All 6 documented hooks are present and working. The extra `agent_end` hook is functional but not documented.

---

## 7. Top 5 Improvements Needed Before Beta

### Priority 1 (BLOCKER): Document API Key Acquisition Flow

**What:** Add a complete section explaining:
1. How to register/login to get a token (for API access)
2. The difference between JWT tokens and the static KNOWWHERE_API_KEY
3. For self-hosted beta: how to generate/set your own key
4. For hosted beta: how to request a key from the team
5. How to test the key works

**Owner:** Nimar (needs to decide on key distribution model)
**Complexity:** Medium (mostly documentation, but requires API key flow decision)

---

### Priority 2 (BLOCKER): Fix docker-compose.yml

**What:**
1. Fix `${KN...Y:-}` → `${KNOWWHERE_API_KEY:-}`
2. Add healthcheck for knowwhere service
3. Document `docker-compose up` as the one-command start
4. Add `.env` file template example

**Owner:** Backend/DevOps
**Complexity:** Low (small file edits)

---

### Priority 3 (BLOCKER): Align Plugin Config Schema, README, and Code

**What:**
1. Remove `storeOnCompaction` from README (doesn't exist in schema/code)
2. Add `importLookbackDays` to README config table
3. Update openclaw.plugin.json schema if options are missing
4. Ensure src/index.ts and index.js are in sync

**Owner:** Plugin developer
**Complexity:** Low-Medium (documentation + possible small code sync)

---

### Priority 4 (HIGH): Create a Consolidated Quickstart Guide

**What:** Create `docs/QUICKSTART-OPENCLAW.md` or merge everything into a single "Get Started" section in README that covers:
1. One-command install for KnowWhere server
2. How to get an API key
3. How to install the OpenClaw plugin (with correct path)
4. How to configure openclaw.json correctly
5. How to verify everything is working (with specific curl commands and expected outputs)

**Owner:** Documentation
**Complexity:** Low (mostly writing, existing code is complete)

---

### Priority 5 (HIGH): Add Plugin Verification Steps

**What:**
1. Document how to check if plugin loaded: `openclaw plugins list`
2. Document how to check if hooks fired: log parsing
3. Add a `/verify` endpoint or health endpoint that tests the full plugin → server → storage → retrieval loop
4. Add example expected output for each verification step

**Owner:** Plugin + Server
**Complexity:** Low (mostly documentation, small server endpoint addition possible)

---

## 8. What's Blocker vs Nice-to-Have

### Blockers (Must Fix Before Beta)
1. ~~API key acquisition flow (Issue 1)~~ ✅ FIXED — /register endpoint exists + documented in WALKTHROUGH.md
2. ~~docker-compose.yml broken env var (Issue 2)~~ ✅ FIXED — `${KNOWWHERE_API_KEY:-}` korrekt, CMD-SHELL typo gefixt
3. ~~Plugin config/schema mismatch (Issue 3)~~ ✅ FIXED — storeOnCompaction aus Doku entfernt, /webhooks/frigate Route existiert
4. ❌ Auth mode communication inconsistent in docs (static key vs PostgreSQL self-service)
5. ❌ Rate Limiting opt-in (RATE_LIMIT=1 nötig, nicht default)
6. ❌ Retention/Garbage Collection fehlt — keine automatische Bereinigung

### High Priority (Should Fix)
4. Create consolidated quickstart guide (Issue 5)
5. Add verification steps (Issue 6)

### Nice-to-Have (Post-Beta)
- npm install path for plugin (currently marked "Future")
- Pre-built Docker images for postgres-storage feature
- Auto-discovery of agent systems (/import/discover endpoint from IMPORT_GUIDE roadmap)
- LangChain/LlamaIndex quickstart guides

---

## 9. Summary Table

| Area | Status | Notes |
|------|--------|-------|
| README clarity | ✅ OK | Port/Modell Infos korrigiert, WALKTHROUGH.md neu |
| docker-compose | ✅ Fixed | `${KNOWWHERE_API_KEY:-}` korrekt, CMD-SHELL |
| Plugin README | ✅ Fixed | storeOnCompaction entfernt, /webhooks/frigate existiert |
| Plugin code | ✅ Working | 6 hooks all functional per PHASE-2-STATUS |
| API key flow | ✅ Working | /register + /login existieren, in WALKTHROUGH.md dokumentiert |
| Quickstart guide | ✅ Exists | docs/QUICKSTART.md + docs/WALKTHROUGH.md |
| Graceful degradation | ✅ Good | Server down = plugin continues |
| Auth persistence | ✅ With PostgreSQL | `/register`/`/login` keys persist in `auth_users` + `auth_api_keys` |
| Auth fallback mode | ✅ Static key | `KNOWWHERE_API_KEY` works without PostgreSQL |
| Rate Limiting | ⚠️ Opt-in | RATE_LIMIT=1 nötig, nicht default |
| Retention/GC | ❌ Missing | Kein Auto-Cleanup für energy=0 Nodes |

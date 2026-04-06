# Beta Onboarding Improvement Plan

**Plan Date:** 2026-04-04
**Owner:** Research Engineer — KnowWhere Investigation & Features
**Target:** External beta testers
**Priority:** Blockers must be resolved before beta launch
**Status (2026-04-04):** IMP-001, IMP-002, IMP-003 DONE. IMP-004 (Quickstart) + IMP-005 (Verify) DONE (QUICKSTART.md + WALKTHROUGH.md created). 3 real blockers remain: Auth in-memory, Rate Limiting opt-in, Retention/GC.

---

---

## Goal

Enable external beta testers to connect KnowWhere to OpenClaw with minimal friction. Based on ONBOARDING-REVIEW.md findings, 3 blockers and 2 high-priority items must be addressed before beta launch.

---

## Improvement Items

### IMP-001: Document API Key Acquisition Flow

**Priority:** BLOCKER
**Status:** ✅ DONE
**Complexity:** Medium

**Description:**
There is no documented path for a new beta tester to obtain an API key. The server exposes `/auth/register` and `/auth/login`, but these return JWT tokens — which are separate from the `KNOWWHERE_API_KEY` static key used by the plugin. A decision must first be made on the key distribution model.

**Decisions Required:**
1. **Self-hosted beta**: Users generate their own static key by setting `KNOWWHERE_API_KEY=xxx` in their environment. Document this clearly.
2. **Hosted beta**: Provide a self-service registration flow that generates a static API key (requires backend work: new endpoint or modify /auth/register to return a static key).
3. **Hybrid**: Allow both — users can bring their own key or get one from the service.

**Recommended Approach for Beta:**
For beta, go with **Option 1 (Self-hosted)**: A beta tester runs their own KnowWhere server and generates their own key. This is the path of least resistance and matches the current architecture.

**Changes Needed:**
1. `README.md` — Add "Getting an API Key" subsection under Authentication explaining:
   - For self-hosted: `openssl rand -hex 32` to generate a key, set as `KNOWWHERE_API_KEY`
   - For the plugin: same key goes in `openclaw.json` config
2. `openclaw-plugin/README.md` — Add a section "Step 1: Get Your API Key" with exact commands

**Owner:** Nimar (final decision on key model), Documentation writer
**Estimated Effort:** 2-3 hours (documentation + optional backend decision)

---

### IMP-002: Fix docker-compose.yml

**Priority:** BLOCKER
**Status:** ✅ DONE
**Complexity:** Low

**Changes Needed:**
1. Fix malformed env var on line 8: `${KN...Y:-}` → `${KNOWWHERE_API_KEY:-}`
2. Add a healthcheck for the `knowwhere` service so it waits for postgres properly
3. Add a note about port 5433 (non-standard, because many devs have 5432 occupied by local Postgres)
4. Create a `.env.example` file showing required variables
5. Consider renaming the service to `knowwhere-server` for clarity

**Current docker-compose.yml (broken):**
```yaml
environment:
  - KNOWWHERE_API_KEY=${KN...Y:-}   # ← BROKEN
```

**Fixed docker-compose.yml:**
```yaml
environment:
  - KNOWWHERE_API_KEY=${KNOWWHERE_API_KEY:-}
  - DATABASE_URL=postgresql://postgres:kw@kw-postgres:5432/kw
```

**Additional docker-compose.yml improvement:**
```yaml
knowwhere:
  # ... existing config ...
  healthcheck:
    test: ["CMD-SHELL", "curl -f http://localhost:3737/health || exit 1"]
    interval: 10s
    timeout: 5s
    retries: 5
    start_period: 30s
```

**.env.example:**
```
KNOWWHERE_API_KEY=your-secret-key-here
# Optional: if you want OpenAI embeddings instead of Ollama
OPENAI_API_KEY=
```

**Owner:** DevOps / Backend
**Estimated Effort:** 1 hour

---

### IMP-003: Align Plugin Config (Schema, README, Code)

**Priority:** BLOCKER
**Status:** ✅ DONE
**Complexity:** Low-Medium

**Problem:** The openclaw-plugin README documents `storeOnCompaction` which doesn't exist in the code or schema. The actual options (`importLookbackDays`) exist in the schema and code but aren't documented in the README.

**Changes Needed:**

1. **openclaw-plugin/README.md** — Remove `storeOnCompaction` from the config example and options table. Add `importLookbackDays` with description.

2. **openclaw-plugin/openclaw.plugin.json** — Add missing options to schema if they should be configurable:
   - Currently schema has: `endpoint`, `apiKey`, `autoRecall`, `autoCapture`, `topK`, `importLookbackDays`
   - `storeOnCompaction` should be removed from README (not in schema)
   - Actually, `storeOnCompaction` IS in the old src/index.ts but NOT in index.js — remove from docs

3. **Ensure src/index.ts and index.js are in sync:**
   - The `src/index.ts` (development version) has `storeOnCompaction` and `before_compaction` hook
   - The `index.js` (compiled/deployed version) has `importLookbackDays` and `gateway_start` hook
   - These are two different implementations — decide which is canonical
   - Recommendation: `index.js` is the deployed one — use it as the source of truth

**Config Table Fix (README.md):**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `endpoint` | `string` | `http://127.0.0.1:3737` | KnowWhere API base URL |
| `apiKey` | `string` | `""` | API key for KnowWhere auth |
| `autoRecall` | `boolean` | `true` | Retrieve memories before each prompt |
| `autoCapture` | `boolean` | `true` | Store session transcripts after each agent run |
| `topK` | `number` | `5` | Max memories to retrieve per query |
| `importLookbackDays` | `number` | `7` | Import sessions from last N days on gateway startup |

**Owner:** Plugin developer
**Estimated Effort:** 1-2 hours (documentation fix + code sync decision)

---

### IMP-004: Create Consolidated Quickstart Guide

**Priority:** HIGH
**Status:** Not started
**Complexity:** Low

**Changes Needed:**

Create a single document at `docs/QUICKSTART-OPENCLAW.md` (or integrate into README.md as a prominent top section) that covers:

**Section 1: Prerequisites**
- OpenClaw >= 2026.3.24 (`openclaw --version` to check)
- KnowWhere server running (with or without Docker)
- Node 22+ if building plugin from source

**Section 2: Start KnowWhere Server (Choose One)**

*Option A — Docker (Recommended for Beta):*
```bash
# 1. Clone and enter repo
git clone https://github.com/NimarMoradbakhti/knowwhere.git && cd knowwhere

# 2. Create .env file
cp .env.example .env  # then edit .env with your KNOWWHERE_API_KEY

# 3. Start with Docker Compose
docker-compose up -d

# 4. Verify server is running
curl http://localhost:3000/health
```

*Option B — Local Development:*
```bash
# 1. Install Rust 1.85+ and Ollama
# 2. Pull embedding model
ollama pull nomic-embed-text-v2-moe

# 3. Generate API key
export KNOWWHERE_API_KEY=$(openssl rand -hex 32)

# 4. Start server
cargo run
```

**Section 3: Install the OpenClaw Plugin**

```bash
# 1. Link the plugin
ln -s /path/to/knowwhere/openclaw-plugin ~/.openclaw/extensions/knowwhere

# 2. Add to openclaw.json (see full config below)
# 3. Restart OpenClaw daemon
openclaw daemon restart
```

**Section 4: Configure openclaw.json**

```json
{
  "plugins": {
    "slots": { "memory": "knowwhere" },
    "entries": {
      "knowwhere": {
        "enabled": true,
        "config": {
          "endpoint": "http://127.0.0.1:3737",
          "apiKey": "YOUR-API-KEY-HERE",
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

**Section 5: Verify It Works**

```bash
# 1. Check plugin is loaded
openclaw plugins list | grep knowwhere

# 2. Check gateway logs
tail -f ~/.openclaw/logs/gateway.log | grep knowwhere

# 3. Test: ask the agent something personal
# You: "Remember: my cat is named Whiskers"
# [restart session or start new session]
# You: "What is my cat's name?"
# Agent should answer "Whiskers"

# 4. Check health endpoint
curl http://localhost:3000/health | jq .node_count
```

**Section 6: Troubleshooting**
- 401 errors → API key mismatch
- "Hook never fired" → OpenClaw version too old
- No memories retrieved → check KnowWhere server is running

**Owner:** Documentation
**Estimated Effort:** 3-4 hours (writing + testing the steps yourself)

---

### IMP-005: Add Plugin Verification Commands

**Priority:** HIGH
**Status:** Not started
**Complexity:** Low

**Changes Needed:**

1. Add a verification script at `openclaw-plugin/verify.sh`:
```bash
#!/bin/bash
set -e

echo "=== KnowWhere + OpenClaw Plugin Verification ==="

# Check plugin exists
if [ ! -d ~/.openclaw/extensions/knowwhere ]; then
  echo "❌ Plugin not installed at ~/.openclaw/extensions/knowwhere"
  exit 1
fi
echo "✅ Plugin installed"

# Check KnowWhere server
if curl -s http://127.0.0.1:3737/health > /dev/null; then
  echo "✅ KnowWhere server is running at http://127.0.0.1:3737"
else
  echo "❌ KnowWhere server not responding at http://127.0.0.1:3737"
  exit 1
fi

# Check node count
NODES=$(curl -s http://127.0.0.1:3737/health | grep -o '"node_count":[0-9]*' | cut -d: -f2)
echo "✅ KnowWhere has $NODES nodes stored"

# Check openclaw.json config
if grep -q "knowwhere" ~/.openclaw/openclaw.json; then
  echo "✅ knowwhere plugin configured in openclaw.json"
else
  echo "❌ knowwhere not found in openclaw.json"
  exit 1
fi

echo ""
echo "=== All Checks Passed ==="
echo "Try asking your agent: 'What was my last message to you?'"
```

2. Update README to reference this verification script
3. Document the log location and how to grep for knowwhere entries

**Owner:** DevOps / Documentation
**Estimated Effort:** 1-2 hours

---

## Post-Beta Improvements (Nice-to-Have)

### IMP-006: npm Install Path for Plugin

**Priority:** Nice-to-have
**Complexity:** Medium

Currently the README says "Option 2: npm install (Future)". For beta, this doesn't matter since the manual path works. Post-beta, a proper npm package would simplify installation.

### IMP-007: Pre-built Docker Images

**Priority:** Nice-to-have
**Complexity:** Medium

Publish `knowwhere-server:latest` and `knowwhere-server:postgres` to Docker Hub. Currently users must build from source.

### IMP-008: Self-Service API Key via API

**Priority:** Nice-to-have
**Complexity:** High

Modify `/auth/register` to return a static `KNOWWHERE_API_KEY` for the user's own server instance, enabling hosted beta with self-service onboarding.

---

## Timeline Recommendation

**Week 1 (Before Beta Invite):**
- IMP-001: API key documentation (owner: Nimar + docs)
- IMP-002: Fix docker-compose.yml (owner: backend)
- IMP-003: Align plugin config docs (owner: plugin dev)

**Week 2 (Before Beta Invite):**
- IMP-004: Create quickstart guide (owner: documentation)
- IMP-005: Add verification commands (owner: devops/docs)

**Post-Beta:**
- IMP-006, IMP-007, IMP-008

---

## Files to Modify

| File | Changes |
|------|---------|
| `docker-compose.yml` | Fix env var, add healthcheck |
| `.env.example` | Create (new file) |
| `README.md` | Add API key section, fix Docker section |
| `openclaw-plugin/README.md` | Fix config table, add verify steps |
| `openclaw-plugin/verify.sh` | Create (new file) |
| `docs/QUICKSTART-OPENCLAW.md` | Create (new file) |
| `docs/plans/beta-onboarding.md` | This file |

---

## Files Created

| File | Purpose |
|------|---------|
| `docs/ONBOARDING-REVIEW.md` | Full findings and friction point analysis |
| `docs/plans/beta-onboarding.md` | This file — improvement plan with owners and complexity |

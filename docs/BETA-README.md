# KnowWhere Beta — Current Status

This document describes the actual beta state of the repository on `main`, not an aspirational release plan.

**Current repository/package version:** `0.1.0`
**Status:** beta on `main` — usable, actively evolving, still opinionated and not yet productized end-to-end

---

## What beta users get today

KnowWhere is a pointer-first memory service for AI agents. It stores:

- session data as full text plus embeddings
- external sources as pointers plus metadata only

Current beta capabilities:

| Capability | Status | Notes |
|------------|--------|-------|
| Session storage and retrieval | Stable | Core memory loop works |
| Hybrid retrieval | Stable | Vector + BM25 + RRF |
| Retrieval profiles | Stable | `user-facing`, `agent-debug`, `full-fidelity` |
| Static admin auth | Stable | `KNOWWHERE_API_KEY` |
| Self-service user auth | Beta | Requires `postgres-storage` + `DATABASE_URL` |
| Local Ollama operation | Stable | Default local path |
| PostgreSQL-backed runtime | Beta | Adds analytics and memory lifecycle routes |
| React dashboard | Beta | Overview, memory stream, search, chat, governance |
| Dream status and event views | Beta | Exposed in API and dashboard |
| OpenClaw plugin integration | Beta | Practical integration path exists |

---

## Known limitations

These are the current gaps that still matter in beta.

1. **The dashboard is useful but not a full admin console.** The React UI covers overview, stream, search, chat, and governance, but not every backend route has a first-class screen yet.

2. **There are two UI surfaces.** `dashboard/` is the active React frontend; `frontend/` is still served as a minimal backend fallback and should not be treated as the complete operator UI.

3. **Auth depends on deployment mode.** Without `postgres-storage`, only the static `KNOWWHERE_API_KEY` path exists. With `postgres-storage` plus `DATABASE_URL`, `/register`, `/login`, and `/refresh` are enabled.

4. **User tokens are intentionally restricted.** `GET /auth/me` for user tokens currently exposes only `user-facing`. `agent-debug` and `full-fidelity` are reserved for admin tokens.

5. **Provider switching still requires a restart.** You can force provider selection with `KNOWWHERE_EMBEDDING_PROVIDER`, but changing providers or models is not hot-reloaded.

6. **Migration tooling is still manual.** Moving data between JSON-backed local state and PostgreSQL is not yet a guided product workflow.

7. **Historical import is still operator-driven.** OpenClaw can import a recent session window automatically, but broad host-system discovery and deep historical import are not yet turnkey.

8. **Rate limiting assumes reverse-proxy deployment.** `RATE_LIMIT_MODE=proxy` only makes sense when a proxy provides `X-Forwarded-For` or `X-Real-IP`.

9. **Retention is policy-driven, not hard-delete automation.** The energy and compression APIs help manage stale memories, but automatic destructive cleanup is intentionally not enabled.

---

## Recommended beta operating mode

- Always set `KNOWWHERE_API_KEY` outside throwaway local development
- Use `/register` and `/login` only when PostgreSQL auth mode is really enabled
- Treat `GET /auth/me` as the source of truth for token capabilities
- Set `AUTH_STRICT_MIGRATIONS=true` in production-like environments
- Enable `RATE_LIMIT_MODE=proxy` only behind a real reverse proxy
- If you use non-default Ollama models, set `OLLAMA_MODEL` and, when needed, `OLLAMA_EMBEDDING_DIMENSION`

---

## How to get help

### GitHub issues

Use the issue tracker for bugs, regressions, and deployment problems:

- [github.com/Jind0la/knowwhere/issues](https://github.com/Jind0la/knowwhere/issues)

Include:

- repository version or commit
- deployment method
- auth mode
- embedding provider or model
- reproduction steps
- relevant logs

### Direct project contact

For security-sensitive reports or issues you do not want to publish publicly, use the project's direct contact channel.

---

## Good bug report template

```text
## Repository Version / Commit
0.1.0 or <git sha>

## Deployment Method
cargo run / docker / docker compose

## Auth Mode
static KNOWWHERE_API_KEY / postgres user token

## Embedding Setup
OLLAMA_MODEL=... / OPENAI / GROK

## What Happened
...

## Steps to Reproduce
1.
2.
3.

## Expected Behavior
...

## Actual Behavior
...

## Logs / Screenshots
...
```

---

## Roadmap focus

### Near-term

- bring dashboard coverage closer to backend route coverage
- finish documentation alignment around auth, profiles, and deployment modes
- improve migration and import ergonomics
- harden PostgreSQL auth and lifecycle operations

### Mid-term

- broader multi-user story beyond the current beta split of admin key vs user token
- guided host-memory discovery and import
- stronger observability for retrieval quality and drift

### Long-term

- operationally simpler production deployment story
- larger-scale storage and graph backends
- more polished framework integrations

---

## Versioning policy

Until the first tagged production release, the safest assumption is:

- `0.1.x` tracks the beta repository line
- behavior may still tighten between minors
- docs should follow the code on `main`, not a marketing version string

---

## Your data

- JSON/local mode writes to `KNOWWHERE_DATA_DIR` and persists as local state
- PostgreSQL mode persists in the configured database
- external content should remain pointer-first
- KnowWhere is intended to run on your own infrastructure; it does not require a hosted service

---

## Beta feedback that matters most

The most valuable reports right now are:

- auth and onboarding confusion
- retrieval-profile regressions
- dashboard/backend mismatches
- PostgreSQL mode problems
- import workflow gaps
- documentation inaccuracies

---

## Current change summary

The repository currently ships beta support for:

- pointer-first storage
- hybrid retrieval
- token capability introspection via `GET /auth/me`
- server-side retrieval-profile enforcement
- React dashboard support in `dashboard/`
- CI coverage for Rust, PostgreSQL mode, feature matrix, dashboard build, and Docker build

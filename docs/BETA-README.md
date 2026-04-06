# KnowWhere Beta — Welcome!

Thank you for joining the KnowWhere beta. You're helping shape the future of AI memory systems. This document covers everything you need to get started and how to report issues.

**Current version:** v0.3.0 (Beta)
**Status:** Open beta — actively developing, expect breaking changes

---

## What You're Getting

KnowWhere is a **fractal memory service** for AI agents. It stores what your agent learns and retrieves it contextually — giving your AI a persistent memory that never forgets.

### Core Features in Beta

| Feature | Status | Notes |
|---------|--------|-------|
| Session storage & retrieval | Stable | Works well |
| Hybrid search (vectors + keywords) | Stable | USearch + BM25 + RRF |
| OpenClaw plugin integration | Stable | Tested with OpenClaw 2026.3.24+ |
| Docker deployment | Stable | Both in-memory and PostgreSQL |
| Local Rust + Ollama | Stable | Requires Rust 1.85+ |
| PostgreSQL persistence | Beta | Full-text search, deduplication |
| Dream Mode (auto-clustering) | Beta | Background process |
| Namespaces | Beta | Organize memories into directories |
| Skills system | Beta | Reusable agent skills |
| VLM summarization | Beta | Compress memories via VLM |

---

## Known Limitations

These are known gaps — some are on the roadmap, others need your feedback:

1. **No web UI for memory management** — Currently only API + Swagger UI. A dashboard is planned.

2. **Auth depends on deployment mode** — Without `postgres-storage`, only static `KNOWWHERE_API_KEY` is available. With `postgres-storage` + `DATABASE_URL`, `/register` + `/login` + `/refresh` are enabled.

3. **Embedding provider lock-in** — Switching between Ollama/OpenAI/Grok requires restarting the server.

4. **No automatic migration tool** — Moving from in-memory to PostgreSQL requires manual export/import.

5. **Session import is selective** — The OpenClaw plugin only imports the last 7 days by default. Full historical import is manual.

6. **Rate limiting needs reverse proxy** — `RATE_LIMIT=1` requires nginx or Cloudflare in front.

7. **Docker: no default API key** — If you don't set `KNOWWHERE_API_KEY`, the server runs without auth (anyone can access).

8. **Retention/GC is policy-driven, not automatic by default** — Low-energy memories are surfaced via `/energy/low` and can be processed via `/energy/decay/apply` and `/energy/compress`. Automatic deletion is not enabled in beta by default.

### Beta Operations Policy (recommended)

- **Auth:** Always set `KNOWWHERE_API_KEY` for self-hosted beta. Use `/register`/`/login` only when PostgreSQL mode is enabled.
- **Rate limit:** Set `RATE_LIMIT=1` only when running behind a reverse proxy that provides client IP headers.
- **Retention/GC MVP:** Run a scheduled maintenance job:
  1) `POST /energy/decay/apply`
  2) `GET /energy/low`
  3) `POST /energy/compress` for selected clusters

---

## How to Get Help

### Option 1: GitHub Issues (Bug Reports)

For bugs, crashes, or unexpected behavior:

1. Check existing issues first: [github.com/NimarMoradbakhti/knowwhere/issues](https://github.com/NimarMoradbakhti/knowwhere/issues)
2. If not reported, open a new issue with:
   - KnowWhere version (`cargo run --version` or Docker image tag)
   - How you deployed (Docker, local Rust, etc.)
   - Steps to reproduce
   - Error messages or logs

### Option 2: Discord (Beta Testers Channel)

For real-time discussion, questions, and early access to new features:

**Invite link:** [Join the KnowWhere Discord](#) _(link coming soon)_

Look for the `#beta-testers` channel.

### Option 3: X / Twitter

For general questions and updates:

- **@NimarMoradbakhti** — Follow for release announcements

### Option 4: Direct Contact

For security issues or sensitive bugs:
- Email: (contact via X/Twitter DMs)

---

## How to Report Issues Effectively

### Bug Report Template

```
## KnowWhere Version
v0.3.0 (Docker / local Rust — specify which)

## Deployment Method
Docker docker-compose / Docker standalone / cargo run / etc.

## What Happened
[Clear description of what went wrong]

## Steps to Reproduce
1.
2.
3.

## Expected Behavior
[What you expected to happen]

## Actual Behavior
[What actually happened]

## Logs
[Relevant logs from KnowWhere server and/or OpenClaw gateway]
```

### Performance Issue Template

```
## KnowWhere Version
## Deployment
## Query / Operation Type
## Data Size (approx nodes, age of installation)
## Expected Performance
## Actual Performance
## Timing Data
[How long operations take]
```

---

## Beta Roadmap

Help us prioritize! React to issues or comment with your use case.

### Near-term (v0.4.x)
- [ ] Dashboard UI for memory browsing
- [ ] Export/import tool for migrations
- [ ] Auto-discovery of OpenClaw memories

### Mid-term (v0.5.x)
- [ ] Multi-user authentication (not just shared API keys)
- [ ] REST API for auto-discovery + import
- [ ] Cursor IDE integration

### Long-term (v1.0)
- [ ] Production-ready PostgreSQL storage
- [ ] Horizontal scaling via message queue
- [ ] Official npm package for OpenClaw plugin

---

## Versioning Policy

KnowWhere uses semantic versioning during beta:

- **Patch** (0.3.1 → 0.3.2): Bug fixes, no API changes
- **Minor** (0.3.0 → 0.4.0): New features, backward-compatible API changes
- **Major** (0.3.0 → 1.0.0): Breaking changes (rare during beta)

**Breaking changes will be announced in Discord and via GitHub releases.**

---

## Your Data

- **In-memory mode (default Docker):** Data is stored in `data/state.json`. It is **lost when the container is deleted**.
- **PostgreSQL mode (docker-compose):** Data persists in the `kw-postgres` container. Delete the volume to wipe data.
- **We do not collect any usage data.** KnowWhere runs entirely on your infrastructure.

---

## Contributing to the Beta

We're actively looking for:
- **Integration partners** — Connect KnowWhere to your agent framework
- **Performance testers** — Push the limits of retrieval quality
- **Documentation reviewers** — Help us write docs that work for non-developers

See [github.com/NimarMoradbakhti/knowwhere](https://github.com/NimarMoradbakhti/knowwhere) for the codebase and contribution guidelines.

---

## Changelog

### v0.3.0 (Beta) — 2026-03-?
- Initial beta release
- Hybrid retrieval (USearch + BM25 + RRF)
- OpenClaw plugin integration
- Docker + Docker Compose support
- PostgreSQL persistence (beta)
- Dream Mode auto-clustering (beta)
- Namespaces and Skills system (beta)

# KnowWhere Beta — Current Status

**Current version:** `0.3.0`
**Status:** beta on `main` — actively developed, 111 tests passing, Docker Compose ready

---

## What works today

| Capability | Status | Notes |
|------------|--------|-------|
| Session storage + retrieval | ✅ Stable | Auto-chunking multi-round conversations |
| External pointer storage | ✅ Stable | Pointer-first, never copies raw data |
| Fractal Zoom retrieval | ✅ Stable | L2→L1→L0 hierarchical search with pruning |
| 5-Type memory system | ✅ Stable | Episodic, Semantic, Preference, Procedural, Meta |
| Trust tiers | ✅ Stable | Auto-detected: primary/reference/derived/volatile |
| L2→L1→L0 Compaction | ✅ Stable | LocalSummarizer (Ollama) + VLM fallback |
| Hybrid retrieval | ✅ Stable | USearch + BM25 + RRF fusion |
| Energy Decay | ✅ Beta | Ebbinghaus forgetting curve |
| Deduplication | ✅ Beta | Finds and merges duplicates |
| Conflict detection | ✅ Beta | Semantic conflicts with resolution |
| Self-healing | ✅ Beta | Orphaned nodes, broken links, embedding drift |
| Static admin auth | ✅ Stable | `KNOWWHERE_API_KEY` |
| Self-service user auth | ✅ Beta | PostgreSQL-backed |
| Retrieval profiles | ✅ Stable | user-facing / agent-debug / full-fidelity |
| Local Ollama | ✅ Stable | snowflake-arctic-embed2 (1024-dim) |
| Docker Compose | ✅ Stable | PostgreSQL + Ollama + KnowWhere |
| OpenClaw plugin | ✅ Beta | 6 hooks, E2E tested |
| Tests | ✅ 111/111 | 70 unit + 41 integration |

---

## Known limitations

1. **Provider switching requires restart.** Changing `KNOWWHERE_EMBEDDING_PROVIDER` or models is not hot-reloaded.
2. **Migration tooling is manual.** Moving between JSON and PostgreSQL is operator-driven.
3. **Dashboard is beta.** React UI covers overview, stream, search, chat, governance — not every backend route.
4. **User tokens are restricted.** User tokens get `user-facing` only. `agent-debug` and `full-fidelity` are admin-only.
5. **Rate limiting assumes reverse-proxy.** Use `RATE_LIMIT_MODE=proxy` behind nginx/traefik.
6. **Phase 2 connectors pending.** HomeAssistant, Google Drive, Cross-Modal Embedding are planned but not implemented.

---

## Recommended operating mode

- Set `KNOWWHERE_API_KEY` outside throwaway dev
- Use `docker compose up -d --build` for full stack
- `OLLAMA_MODEL=snowflake-arctic-embed2` for multilingual embeddings (EN+DE+FR+ES+IT)
- `OLLAMA_SUMMARIZER_MODEL=llama3.2` for L2→L1→L0 compaction
- Enable PostgreSQL (`DATABASE_URL`) for full feature set
- Treat `GET /auth/me` as source of truth for token capabilities

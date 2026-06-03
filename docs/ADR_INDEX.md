# Architecture Decision Records — Index

This index catalogs all significant architectural decisions in KnowWhere's history. Each ADR captures *what* was decided, *when*, *why*, and the *alternatives considered*.

## Active ADRs

| # | Title | Date | Status | Summary |
|---|-------|------|--------|---------|
| 006 | [Semantic Linking Architecture](ADR-006-semantic-linking.md) | 2026-05 | Proposed | Two-stage retrieval with optional third stage for semantic linking between knowledge fragments |

## Decisions Documented in Code/Changelog

The following architecture decisions are documented in `CHANGELOG.md` or the source code rather than dedicated ADR files:

| Date | Decision | Documented In |
|------|----------|---------------|
| 2026-06 | **API routes.rs split into 14 submodules** — health, store, retrieve, rerank, maintenance, trajectory, conflicts, energy, dedup, healing, namespaces, skills_routes, turn_handlers. Shared types extracted to `api/types.rs`. | `CHANGELOG.md` [Unreleased], `ARCHITECTURE_MAP.md` |
| 2026-05 | **Turn-Level Storage replaces Session-Level** — Every conversation turn gets its own embedding with `EmbeddingInfo` metadata. Session-level embedding column dropped. | `CHANGELOG.md` v0.6.0 |
| 2026-05 | **nomic-embed-text (768-dim) replaces snowflake-arctic-embed2 (1024-dim)** — Better German-language performance, smaller model. | `CHANGELOG.md` v0.6.0, `docs/MODEL-EVALUATION.md` |
| 2026-05 | **gte-modernbert ONNX replaces bge-reranker-v2-m3** — Faster inference, no Ollama dependency for reranking. | `CHANGELOG.md` v0.6.0 |
| 2026-05 | **Summarizer removed (~4K LOC)** — VLM fallback chain, LocalSummarizer, and Ollama summarization replaced by ollama qwen2.5:3b consolidation. | `CHANGELOG.md` v0.6.0 |
| 2026-05 | **Hybrid BM25 + Dense Retrieval** — Keyword + semantic fusion via RRF with k=5 (was k=60, which caused score collapse). | `CHANGELOG.md` v0.6.0, `docs/SIGNAL-TRACE.md` |
| 2026-05 | **Source-Type Weighting** — Provenance-aware scoring: Real(1.0), Synthetic(0.85), Derived(0.70), Unknown(0.95). Configurable via env var, file, or per-query. | `CHANGELOG.md` v0.6.0 |
| 2026-05 | **Fact Extraction Pipeline** — Symbolic knowledge extraction from conversations using regex rules with confidence scoring. | `CHANGELOG.md` v0.6.0 |
| 2026-05 | **Temporal-Aware Scoring** — Recency decay via `temporal_weight` parameter (0.0–0.8). Newer information scores higher. | `CHANGELOG.md` v0.6.0 |
| 2026-05 | **Production unwrap() elimination** — All 30 production `.unwrap()` calls replaced with `.expect("...")` with descriptive messages. Test-only unwraps preserved. | `CHANGELOG.md` [Unreleased] |
| 2026-05 | **Env-var test serialization** — `ENV_LOCK: Mutex<()>` prevents race conditions in tests that manipulate process-global environment variables. | `CHANGELOG.md` [Unreleased] |
| 2026-05 | **Storage Backend trait** — Abstracted `StorageBackend` trait with `InMemoryStore` + `PostgresStore` implementations. PostgreSQL for production, in-memory for dev/testing. | `src/storage/backend.rs`, `docs/CRIT-003-postgresql-architecture.md` |
| 2026-04 | **Cross-Modal Embedding Router** — Content-type-based dispatch: text→Ollama, image→CLIP, audio→Whisper. Unified 768-dim space. | `src/embedding/router.rs`, `docs/plans/2026-05-02-cross-modal-embedding.md` |
| 2026-03 | **PostgreSQL as single source of truth** — Storage Backend trait design with `PostgresStore` as production backend. | `docs/CRIT-003-postgresql-architecture.md` |

## Future ADRs (Planned)

| Title | Status | Reference |
|-------|--------|-----------|
| Entity Graph Layer | Approved (not yet implemented) | `docs/ENTITY_GRAPH_IMPLEMENTATION_PLAN.md` |
| Fractal-Core Geometry Hierarchy | Design Proposal | `docs/plans/2026-05-15-fractal-core.md` |
| Qwen3-VL Embedding Upgrade | Evaluated (not deployed) | `docs/qwen3-vl-embedding-prototype.md` |

---

## Writing a New ADR

Use `ADR-006-semantic-linking.md` as a template. Each ADR should contain:

1. **Status** — Proposed / Accepted / Deprecated / Superseded
2. **Context** — What's the problem? Why now?
3. **Decision** — What did we decide?
4. **Alternatives Considered** — What else was on the table? Why rejected?
5. **Consequences** — What gets easier? What gets harder?

Name files sequentially: `ADR-007-<slug>.md`. Add them to this index.

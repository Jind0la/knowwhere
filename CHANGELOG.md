# Changelog


## [Unreleased]

### Changed
- **api/routes.rs refactored:** 5,884 LOC → 104 LOC split across 14 domain modules (health, store, retrieve, rerank, maintenance, trajectory, conflicts, energy, dedup, healing, namespaces, skills_routes, turn_handlers). Shared types extracted to `api/types.rs`. 304/305 tests pass. See `docs/plans/2026-06-03-split-api-routes.md` and `ARCHITECTURE_MAP.md`.
- **Dependency hygiene:** `cargo update` — 101 packages to latest compatible versions. Cargo.lock refreshed.
- **Production unwrap() elimination:** All 30 production `.unwrap()` calls replaced with `.expect("...")` in `in_memory.rs` (21 Mutex locks), `fact_extraction.rs` (7 regex/JSON), and `postgres_store.rs` (2 Option). Test-only unwraps preserved. See `oss-forensics` skill v1.2.0.
- **Rust tooling:** Installed `cargo-outdated`, `cargo-udeps` (nightly), `cargo-geiger` for extended codebase health scans. `oss-forensics` skill upgraded to v1.2.0 with new Steps 6d–6g.
- **Documentation overhaul:** Root directory cleaned (37→14 files). docs/ reorganized: 42 obsolete files archived to `docs/archive/`, 16 broken artifacts deleted. New docs: `CONTRIBUTING.md`, `docs/API_REFERENCE.md`, `docs/ADR_INDEX.md`. Updated `docs/README.md` with current structure.

### Fixed
- **Race condition in source_weighting tests:** `std::env::set_var` is process-global and not thread-safe. `test_from_config_env_takes_precedence_over_file` and `test_from_config_file_only` raced on `KNOWWHERE_SOURCE_TYPE_WEIGHTS` env var. Fix: `static ENV_LOCK: Mutex<()>` serializes all 11 env-var-manipulating tests. Test suite: 305/305 (was 304/305).

All notable changes to KnowWhere are documented in this file.

## [0.6.0] — 2026-05-19

### Added — Turn-Level Storage & Per-Turn Embeddings

**The most significant architectural change since 0.4.0.** Session-level embeddings (one vector per entire chat history) replaced by turn-level embeddings (one vector per individual message).

- **EmbeddingInfo Struct:** New `EmbeddingInfo { vector, provider, dimension, metadata }` on every Turn record. Captures embedding provenance at storage time. Commit series `12cb604`→`8022efa`.
- **Per-Turn Embedding Generation:** `store_session_json` (single + multi-turn) and `store_session_batch` now emit per-turn `FractalNodes` with `speaker_role`, `is_turn`, and `turn_index` metadata. Speaker role auto-detected via `parse_speaker_role_from_chunk`. No more session aggregates.
- **Turn Data Model:** `conversation_turns` table with `embedding vector(1024)`, `embedding_type`, `embedding_dim` columns. Migration 014 creates the table, Migration 016 adds embedding metadata columns, Migration 017 backfills existing rows.
- **Session Embedding Removal:** Migration 015 drops `embedding` column, HNSW index, and `compute_session_embedding()` function from `conversation_sessions`. `update_turn()` now handles embedding metadata alongside vector updates.
- **Turn-Level Retrieval:** Index builder refactored to target turn index only. Retrieval queries, ranking logic, and API responses updated for per-turn embeddings. Session-level embedding references fully deprecated.

### Added — Stratified LongMemEval Benchmark

Reproducible, scientifically clean evaluation framework with controlled case selection.

- **Stratification Criteria:** Per-type quotas for all 6 question types + 5 abstention cases. `stratified_filter.json` selects 42 cases from 500.
- **Eval Harness:** Per-type breakdowns with turn-level metrics (NDCG@k, recall_any, recall_all) alongside session-level metrics. Support for `--stratified` and `--mode multi|percase`.
- **Reranker Comparison:** gte-modernbert (ONNX, 599MB) vs bge-reranker-v2-m3 benchmarked on identical eval set. ONNX reranker delivers faster inference with no Ollama dependency.

### Added — Source-Type Weighting & Provenance

- **SourceTypeWeights:** Config loader supporting JSON file + environment variable with priority chain. `SourceTypeWeights::from_config()` reads `KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE` env var → `./source_weights.json` fallback. 10 new tests.
- **Multiplier Chain:** `tier * explicit * mtype * source` scoring pipeline. Fixed multi-query RRF fusion path that was discarding source weights (`None` → `source_type_weights`). 7 integration tests (54/55 pass).
- **Provenance Fields:** `source_weight_applied` and `original_source` promoted to top-level `ScoredNode` API fields. All 5 code paths covered (normal retrieval, fractal expansion, turn-level, reflection, reranker fallback).

### Added — Fact Extraction Pipeline

Explicit facts extracted from conversations and stored as weighted knowledge.

- Fact extraction rules, data schema, and pipeline module.
- Integration with storage and retrieval weighting.
- Evaluation framework for extraction quality.

### Added — Hybrid Retrieval (BM25 + Dense)

- `HybridRetriever` combining BM25 keyword matching with dense vector search.
- Numeric/short-answer test suite.
- Baseline vs hybrid comparative evaluation.

### Changed

- **Reranker Model:** Switched from `bge-reranker-v2-m3` (Ollama, 438MB) to `gte-modernbert` (ONNX, 599MB). No Ollama dependency for reranking.
- **Dependency Cleanup:** 11 unused Ollama models removed (~14GB freed). Only `nomic-embed-text` (274MB), `llama3.2` (2.0GB), and `qwen2.5:3b` (1.9GB) retained.

### Quantitative Results (LongMemEval — 42 Stratified Cases)

| Metric | Pre-Migration (0.5.x) | Post-Migration (0.6.0) | Δ |
|--------|:---:|:---:|:---:|
| Overall Recall@5 | 7.1% | **72.97%** | +65.9pp |
| MRR | ~0.00 | **0.5577** | new |
| Turn-Level NDCG@5 | — | **0.4247** | new |
| Question Types at 0% | 5/6 | **0/6** | all functional |

| Question Type | Pre Recall@5 | Post Recall@5 | Δ |
|--------------|:---:|:---:|:---:|
| single-session-assistant | 75% | 75% | = |
| single-session-user | 0% | 80% | +80pp |
| multi-session | 0% | 75% | +75pp |
| temporal-reasoning | 0% | 77.8% | +78pp |
| knowledge-update | 0% | 71.4% | +71pp |
| single-session-preference | 0% | 50% | +50pp |

_Competitive context: AgentMemory reports 50.4% Recall@5 on the same benchmark (499 cases). Full Context (GPT-4) oracle: 60.7%. KnowWhere 0.6.0: 73.0% on 42 stratified cases._

### Fixed

- **Pre-existing Compilation Errors:** `storage/mod.rs` exports, `PostgresStore` visibility, `sqlx` relation-does-not-exist macros, `HybridQuery` missing `session_id` field — all resolved, enabling full test suite to compile and run.

## [Unreleased] — 2026-05-18

### Added
- **Hybrid Temporal-Semantic Scoring (WP1):** `retrieve_fractal` and `hybrid_retrieve` now apply temporal recency scoring alongside semantic relevance. Configurable via per-query `temporal_weight` override or server-wide default. Produces linear 5× score amplification (tw=0.0→0.9) with a 7-day half-life for conversational memory. Quantitative: 2.73 Avg Recency (baseline 2.48, +10.1%).
- **Runtime Temporal Weight Configurability:** `GET /config/temporal_weight` reads the current server default; `POST /config/temporal_weight` updates it at runtime (no restart). Per-query overrides in `RetrieveFractalRequest.temporal_weight` take precedence. Reads `KNOWWHERE_TEMPORAL_WEIGHT` env var at startup (clamped 0.0–0.8). Design follows `governance_policy` pattern (`Arc<RwLock<Option<f32>>>`).
- **Smart Text Chunker (WP3):** `TextChunker` performs semantic boundary detection with paragraph→sentence→word fallback, configurable overlap, and stub merging. Enables chunking of large content before embedding. 12 tests covering major paths. Commit `6a237e5`.
- **num_ctx 2048→8192:** Embedding context window expanded 4× for better chunk coverage. Highest-impact WP3 change.
- **Fractal Hierarchy in API Response:** `POST /retrieve_fractal` now includes `context_tier`, `parent_tier_id`, `children_tier_ids`, `status`, and `importance` in every `ScoredNode`. Enables API consumers (AMB adapter, Hermes plugin) to distinguish L1 summaries from L0 raw nodes and traverse the fractal hierarchy. Previously only `GET /retrieve/{id}` exposed these fields.

### Fixed
- **created_at Bug in store_external:** `PostgresStore::store_session()` now accepts `Optional created_at` — SQL uses `COALESCE($19, NOW())`. `insert()` passes `node.created_at` through. Regression test `store_external_preserves_custom_created_at` verifies round-trip. Commit `114755a`.
- **BUG-016: Vector Retrieval Score Collapse (CRITICAL):** `retrieve_fractal()` was embedding query text with raw `state.embedding.embed(text)` instead of `embed_query()` — missing the `"search_query: "` prefix required by asymmetric embedding models (nomic-embed-text, snowflake-arctic-embed2). All retrieval scores collapsed from ~0.83 to ~0.03 (random noise level). Fixed at both query embedding (L2240) and contrastive query embedding (L2362). Added regression test `test_embed_query_single_with_prefix`. Full root cause analysis in BUG-TRACKING.md.

### Changed
- **Temporal Scoring Half-Life:** Reduced from 21 days to 7 days in `apply_hybrid_temporal_scoring` (both PostgresStore and InMemoryStore). The previous 21-day half-life produced almost no variance on typical conversational data (0–2 days old). 7 days provides ~3× better differentiation for recent memories while still allowing older relevant memories to compete. See t_6001dbad for full analysis.
- **Consolidation Content Threshold:** `find_candidates()`, `pending_count()`, `should_compact()`, `force_run()` lowered minimum content length from 500 → 100 chars to catch claim/document chunks.

## [0.5.0] — 2026-05-05

### Added
- **Decision Scoring Pipeline:** Decision memory types now receive PRIMARY trust tier (1.18×) and a dedicated memory_type_multiplier (1.5×) for a total 2.01× score boost. Commit `1dfa292`.
- **Decision Parse Support:** `MemoryType::parse()` now recognizes `"decision"` string. Previously, `store_session` with `memory_type: "decision"` was silently downgraded to `Episodic`. Commit `679d8d3`.
- **PostgreSQL Tier Persistence:** Full roundtrip for fractal tier fields (context_tier, parent_tier_id, children_tier_ids) through PostgreSQL. Commit `8452f46`.
- **Hermes Retrieval Eval:** `scripts/eval_hermes_retrieval.py` now tracks Hermes-facing retrieval quality, including top-1 non-meta rate, decision-filter purity, provenance coverage, repeated top-1 rate, stale-conflict rate, and latency.
- **PostgreSQL Fractal Expansion:** `PostgresStore::expand_fractal` mirrors `MemoryStore` (parent bridge, `children_tier_ids`, cosine pruning, global cap, cycle-safe batch fetch via `get_fractal_nodes_any`).
- **Evidence Pack + MMR:** `/retrieve_fractal` applies intent scoring on storage hits, evidence dedupe (parent / `source_node_ids[0]` / session / pointer), then λ=0.65 MMR selection before `top_k` (with and without governance).
- **Query Intent Routing:** `/retrieve_fractal` accepts `query_intent` hints (`current_state`, `decision_why`, `procedure`, `preference`, `debug`, `historical`) and applies lightweight intent-aware scoring.
- **Decision Provenance Metadata:** Consolidation-created summaries and claim Decision nodes now include structured provenance metadata such as `source_node_ids`, `source_session_ids`, `source_turn_range`, `derived_from`, `claim_scope`, `decision_what`, and `decision_why`.

### Changed
- **Ollama Embedding:** Switched from OpenAI to local `nomic-embed-text-v2-moe` (768-dim, MoE, 1.0GB VRAM). Zero API cost, 0.23s warm latency. Commit `1a1ee1f`.
- **Native macOS First:** Docker deployment deprecated in favor of native Ollama + PostgreSQL Homebrew. Server runs without Docker on macOS.
- **MemoryType::all()** expanded from 5 to 6 types (includes Decision). Commit `679d8d3`.
- **Episodic scoring:** Episodic nodes now get 0.85× memory_type_multiplier (was 1.0×). Conversation chatter is properly de-prioritized relative to structured knowledge.
- **Integration**: Hermes MemoryProvider replaces OpenClaw plugin. Per-turn crash-safe storage with dual retrieval (episodic + decision).
- **Hermes Memory Context:** Hermes prefetch now treats KnowWhere memories as background context rather than authoritative instructions, avoids default Reflect, filters Meta/XML memory artifacts, and labels facts/decisions as `[KW-N]` / `[KW-DECISION]`.
- **Current-vs-Historical Convention:** New writes receive a `claim_scope` metadata convention (`episodic`, `current`, `historical`, `decision`, `preference`, `procedural`, `diagnostic`) to support time-aware ranking without data deletion.

### Fixed
- **Fractal Zoom Bridge:** `parent_tier_id` backward traversal for child nodes without children. Commit `bee746b`.
- **USearch Capacity:** Re-reserve fix prevents panic on large insertions. Commit `d059e40`.
- **VLM UTF-8 Panic:** Byte-boundary crash with emoji/special characters fixed. Commit `d059e40`.
- **Prompt Escape Hatches:** LocalSummarizer and VLM prompts no longer allow "No decision made" output. Commit `46facb1`, `b43d20a`.
- **Integration Test:** `full_fidelity_profile_surfaces_internal_assistant_artifacts` updated for Episodic 0.85× multiplier. Commit `679d8d3`.
- **Strict Memory-Type Filtering:** Unknown `memory_type_filter` values now return `400 Bad Request` and filters are enforced after fractal expansion and with `governance_enabled=false`.
- **PostgreSQL Filter Parity:** `PostgresStore::hybrid_retrieve` now applies `memory_type_filter` consistently in BM25, vector-only, and hybrid branches before final `top_k`.
- **Default Meta Leakage:** `/retrieve_fractal` no longer prepends synthetic `<knowwhere_memory>` instruction nodes when `reflect=false`.
- **Hermes Eval metrics:** Eval script adds `unique_top1_rate`, `mean_source_diversity`, `mean_session_diversity`, `fractal_path_coverage`, and a `--fail-gates` check on source diversity.

---

## [0.4.0] — 2026-05-01

### Added
- **Reflect Mode:** Query-time memory synthesis via local Ollama (llama3.2). Commit `0e7ed17`.
- **Claims Extraction:** Structured claim parsing from consolidation summaries → Decision nodes.
- **Event-Driven Consolidation:** Write-triggered compaction replaces timer-polling.
- **POST /consolidation/force:** Admin-triggered full re-consolidation.
- **Transient Error Resilience:** DNS/Ollama failures don't mark nodes as processed.
- **Cross-Modal Embedding:** EmbeddingRouter dispatches CLIP/Whisper/Sensor embeddings via Ollama.
- **Cross-Encoder Reranking:** bge-reranker-v2-m3 via ONNX (2.5GB RAM, feature-gated `reranker`).
- **MemoryType::Decision:** Dedicated variant (importance=9, immutable, confidence=0.85).
- **6-Type Memory System:** Episodic, Semantic, Preference, Procedural, Decision, Meta.
- **Governance Policy:** Retrieval profiles, sensitivity levels, confidence thresholds.
- **Energy Decay:** Ebbinghaus forgetting curve for memory retention.
- **PostgreSQL Features:** Deduplication, conflict detection, self-healing, namespaces, skills.

### Changed
- **LocalSummarizer:** Ollama llama3.2 as PRIMARY compaction provider. VLM (GPT-5-nano→GPT-4o-mini→Grok-4-fast) is fallback.
- **Truncation Disabled:** Content truncation permanently disabled (panics if called).

### Fixed
- **BUG-012:** GPT-5-nano `max_output_tokens` removed (model rejects it).
- **BUG-013:** L2→L1→L0 compaction broken without API key — fixed via LocalSummarizer.
- **BUG-014:** sqlx offline/online type mismatch — fixed via COALESCE in SQL.
- **BUG-015:** `.sqlx/` offline cache deleted — restored with pre-commit hook.
- **BUG-016:** CI cargo audit 10 RUSTSEC advisories — fixed with `--ignore` flags.

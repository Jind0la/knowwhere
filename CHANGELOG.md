# Changelog

All notable changes to KnowWhere are documented in this file.

## [Unreleased] — 2026-05-12

### Added
- **Fractal Hierarchy in API Response:** `POST /retrieve_fractal` now includes `context_tier`, `parent_tier_id`, `children_tier_ids`, `status`, and `importance` in every `ScoredNode`. Enables API consumers (AMB adapter, Hermes plugin) to distinguish L1 summaries from L0 raw nodes and traverse the fractal hierarchy. Previously only `GET /retrieve/{id}` exposed these fields.

### Fixed
- **BUG-016: Vector Retrieval Score Collapse (CRITICAL):** `retrieve_fractal()` was embedding query text with raw `state.embedding.embed(text)` instead of `embed_query()` — missing the `"search_query: "` prefix required by asymmetric embedding models (nomic-embed-text, snowflake-arctic-embed2). All retrieval scores collapsed from ~0.83 to ~0.03 (random noise level). Fixed at both query embedding (L2240) and contrastive query embedding (L2362). Added regression test `test_embed_query_single_with_prefix`. Full root cause analysis in BUG-TRACKING.md.

### Changed
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

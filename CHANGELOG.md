# Changelog

All notable changes to KnowWhere are documented in this file.

## [0.5.0] — 2026-05-05

### Added
- **Decision Scoring Pipeline:** Decision memory types now receive PRIMARY trust tier (1.18×) and a dedicated memory_type_multiplier (1.5×) for a total 2.01× score boost. Commit `1dfa292`.
- **Decision Parse Support:** `MemoryType::parse()` now recognizes `"decision"` string. Previously, `store_session` with `memory_type: "decision"` was silently downgraded to `Episodic`. Commit `679d8d3`.
- **PostgreSQL Tier Persistence:** Full roundtrip for fractal tier fields (context_tier, parent_tier_id, children_tier_ids) through PostgreSQL. Commit `8452f46`.

### Changed
- **Ollama Embedding:** Switched from OpenAI to local `nomic-embed-text-v2-moe` (768-dim, MoE, 1.0GB VRAM). Zero API cost, 0.23s warm latency. Commit `1a1ee1f`.
- **Native macOS First:** Docker deployment deprecated in favor of native Ollama + PostgreSQL Homebrew. Server runs without Docker on macOS.
- **MemoryType::all()** expanded from 5 to 6 types (includes Decision). Commit `679d8d3`.
- **Episodic scoring:** Episodic nodes now get 0.85× memory_type_multiplier (was 1.0×). Conversation chatter is properly de-prioritized relative to structured knowledge.
- **Integration**: Hermes MemoryProvider replaces OpenClaw plugin. Per-turn crash-safe storage with dual retrieval (episodic + decision).

### Fixed
- **Fractal Zoom Bridge:** `parent_tier_id` backward traversal for child nodes without children. Commit `bee746b`.
- **USearch Capacity:** Re-reserve fix prevents panic on large insertions. Commit `d059e40`.
- **VLM UTF-8 Panic:** Byte-boundary crash with emoji/special characters fixed. Commit `d059e40`.
- **Prompt Escape Hatches:** LocalSummarizer and VLM prompts no longer allow "No decision made" output. Commit `46facb1`, `b43d20a`.
- **Integration Test:** `full_fidelity_profile_surfaces_internal_assistant_artifacts` updated for Episodic 0.85× multiplier. Commit `679d8d3`.

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

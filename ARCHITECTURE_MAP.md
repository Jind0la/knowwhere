# KnowWhere Architecture Map

**Stand: 16. Juni 2026 | v0.6.0 | 88 Rust-Files, 38.874 LOC | 14 API-Module, routes.rs: 84 LOC**

---

## Auf einen Blick

```
Client (Hermes Plugin / REST API)
    │
    ▼
┌─────────────────────────────────────────────┐
│ main.rs (413 LOC)                            │
│ Server-Startup: Axum, Governor, CORS, State  │
└──────────────┬──────────────────────────────┘
               │
    ┌──────────┴──────────┐
    ▼                     ▼
┌──────────────────────────────────┐
│ api/ (14 Module, 104 LOC Router) │
│                                  │
│ routes.rs — nur Modul-Deklaration│
│ types.rs  — Shared Types         │
│ health.rs — Health + Embed       │
│ store.rs  — Session + External   │
│ retrieve.rs — Fractal Retrieval  │
│ rerank.rs — Cross-Encoder        │
│ maintenance.rs — Delete/Purge    │
│ trajectory.rs — Retrieval Runs   │
│ conflicts.rs — Conflict Resolve  │
│ energy.rs — Energy/Decay         │
│ dedup.rs — Deduplication         │
│ healing.rs — Self-Healing        │
│ namespaces.rs — Namespace CRUD   │
│ skills_routes.rs — Skill CRUD    │
│ turn_handlers.rs — Turn Storage  │
└──────────────┬───────────────────┘
     │
     ▼
┌──────────────────────────────────────────────┐
│               MEMORY ENGINE                   │
│                                              │
│  types.rs         — MemoryType (6 Variants)  │
│  fractal_node.rs  — FractalNode Struct       │
│  conversation.rs  — Turn/Session Storage     │
│  chunking.rs      — TextChunker (Semantic)   │
│  fact_extraction  — Symbolic Facts           │
│  governance.rs    — Policy Layer (L4)        │
│  control_room.rs  — Multi-Agent Scoping      │
│  agent.rs         — Agent Management         │
│  namespaces.rs    — Tenant Isolation         │
│  skills.rs        — Skill Storage            │
│  self_healing.rs  — Auto-Repair              │
│                                              │
│  dream/                                      │
│    consolidation  — L0→L1→L2 Chains          │
│    energy_decay   — Ebbinghaus Curve          │
│    deduplication  — Duplicate Detection       │
│    conflict_detection — Conflict Resolution   │
│    audit.rs       — Dream Mode Audit          │
│                                              │
├──────────────────────────────────────────────┤
│               RETRIEVAL ENGINE                │
│                                              │
│  hybrid.rs          — BM25 + Dense + RRF     │
│  cross_encoder.rs   — gte-modernbert (ONNX)  │
│  source_weighting   — Provenance Scoring     │
│  query_expansion    — NER + Temporal Markers │
│                                              │
├──────────────────────────────────────────────┤
│               STORAGE LAYER                   │
│                                              │
│  postgres_store.rs  — PostgreSQL (Production)│
│  in_memory.rs       — JSON-File (Dev)        │
│  backend.rs         — Trait + Factory        │
│  trajectory.rs      — Event Sourcing         │
│                                              │
├──────────────────────────────────────────────┤
│               EMBEDDING                       │
│                                              │
│  provider.rs   — Ollama/OpenAI/Grok          │
│  router.rs     — Multi-Modal Dispatch        │
│  clip.rs       — Image Embeddings            │
│  audio.rs      — Audio Embeddings            │
│  sensor.rs     — IoT Sensor Data             │
│                                              │
├──────────────────────────────────────────────┤
│               EXTERNAL                        │
│                                              │
│  connectors/drive.rs   — Google Drive Sync   │
│  connectors/frigate.rs — Frigate NVR         │
│  reflector/mod.rs      — Ollama Reflection   │
│  scheduler/mod.rs      — Cron Jobs           │
│  services/lifecycle    — Startup/Shutdown    │
└──────────────────────────────────────────────┘
```

---

## Key Files — nach Wichtigkeit

| # | Datei | LOC | Verantwortung |
|---|-------|-----|---------------|
| 1 | `storage/postgres_store.rs` | 2.481 | PostgreSQL-Backend: SQL, Indexes, Migrations |
| 2 | `storage/in_memory.rs` | 2.108 | JSON-File-Backend (Dev) |
| 3 | `retrieval/source_weighting.rs` | 1.976 | Provenance-Scoring Pipeline |
| 4 | `api/retrieve.rs` | 1.811 | Fractal Retrieve + Subconscious Chat |
| 5 | `memory/dream/consolidation.rs` | 1.387 | L0→L1→L2 Dream Mode |
| 6 | `api/store.rs` | 1.277 | Session Storage + External + Self-Improve |
| 7 | `memory/dream/conflict_detection.rs` | 1.128 | Conflict Resolution |
| 8 | `vlm/mod.rs` | 1.086 | Vision-Language Models |

---

## Architektur-Entscheidungen (ADR)

1. **Turn-Level > Session-Level** (Mai 2026): Per-Turn-Embeddings statt Session-Aggregaten. Migration 014-017.
2. **Matryoshka Truncation**: 768d → 256d/64d für Fractal Zoom. Tradeoff: Precision vs. Speed.
3. **Summarizer entfernt** (Mai 2026): ~4K LOC gelöscht. Consolidation deaktiviert bis Neubau.
4. **PostgreSQL als Single Source of Truth**: InMemoryStore nur für Dev. Kein JSON-File in Production.
5. **nomic-embed-text lokal**: Keine API-Kosten. 768d, 8192 Context. Ollama.
6. **gte-modernbert ONNX**: Cross-Encoder ohne Ollama-Dependency. 599 MB.
7. **Source-Type Weighting**: Echte Konversationen > synthetische Injects.
8. **API-Module pro Domain** (Juni 2026): 14 Submodule, routes.rs von 5884 → 104 LOC.

---

## Wo finde ich was?

| Frage | Datei |
|-------|-------|
| "Wie wird ein Turn gespeichert?" | `api/store.rs` oder `api/turn_handlers.rs` → `conversation.rs` |
| "Wie funktioniert die Suche?" | `api/retrieve.rs` → `retrieval/hybrid.rs` → `cross_encoder.rs` |
| "Wie entscheidet Governance?" | `memory/governance.rs` → genutzt in `api/retrieve.rs` |
| "Wie läuft Consolidation?" | `memory/dream/consolidation.rs` + `energy_decay.rs` |
| "Wie sind Embeddings strukturiert?" | `embedding/provider.rs` (Ollama/OpenAI) → `conversation.rs` (EmbeddingInfo) |
| "Wo ist das DB-Schema?" | `storage/postgres_store.rs` + `migrations/` (19 Files) |
| "Wie integriert Hermes?" | `hermes-plugin/knowwhere/__init__.py` |
| "Benchmark-Ergebnisse?" | `benchmarks/reports/` |

---

## Bekannte Problemzonen

1. ~~**`api/routes.rs` (5884 LOC)**~~ → ✅ **Gelöst! 14 Module, 104 LOC Router.**
2. **Consolidation deaktiviert**: Größter Gap zu Hindsight (8-12pp Recall). Neubau nötig.
3. **Reranker nicht aktiv**: ONNX-Modell liegt bereit, aber nicht in Pipeline.
4. **182 `unwrap()` Calls**: Viele davon in Test-Code, aber einige in Produktionspfaden.
5. **Matryoshka-Dimensionen**: 64d für Breitensuche verliert Precision. 768d/256d testen.

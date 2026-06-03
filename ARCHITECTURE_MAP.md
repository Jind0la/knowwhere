# KnowWhere Architecture Map

**Stand: 3. Juni 2026 | v0.6.0 | 65 Rust-Files, 31.758 LOC**

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
┌──────────┐      ┌──────────────┐
│ api/     │      │ runtime.rs   │
│ routes.rs│      │ Rate Limiter │
│ (5884!)  │      │ Embedding    │
└────┬─────┘      └──────────────┘
     │
     │  Alle Endpoints routen hierhin:
     │
     ├── store_session()          → memory/conversation.rs
     ├── store_session_batch()    → memory/conversation.rs
     ├── store_external()         → memory/fractal_node.rs
     ├── retrieve()               → retrieval/mod.rs
     ├── retrieve_fractal()       → retrieval/mod.rs
     ├── rerank()                 → retrieval/cross_encoder.rs
     ├── self_improve()           → reflector/mod.rs
     ├── subconscious_chat()      → api/subconscious_qa.rs
     └── delete_node()            → storage/
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
| 1 | `api/routes.rs` | 5.884 | ALLE REST-Endpoints. ⚠️ Monster-File — sollte aufgeteilt werden |
| 2 | `storage/postgres_store.rs` | 2.647 | PostgreSQL-Backend: SQL, Indexes, Migrations |
| 3 | `memory/governance.rs` | ~1.200 | Policy-Engine: Confidence, Sensitivity, Access |
| 4 | `retrieval/hybrid.rs` | ~800 | BM25 + Dense + RRF Fusion |
| 5 | `memory/dream/consolidation.rs` | ~700 | L0→L1→L2 Dream Mode |
| 6 | `memory/conversation.rs` | ~600 | Turn-Level Storage + EmbeddingInfo |
| 7 | `memory/fractal_node.rs` | 628 | FractalNode Struct (Zentral) |
| 8 | `memory/control_room.rs` | ~500 | Multi-Agent Query Scoping |

---

## Architektur-Entscheidungen (ADR)

1. **Turn-Level > Session-Level** (Mai 2026): Per-Turn-Embeddings statt Session-Aggregaten. Migration 014-017.
2. **Matryoshka Truncation**: 768d → 256d/64d für Fractal Zoom. Tradeoff: Precision vs. Speed.
3. **Summarizer entfernt** (Mai 2026): ~4K LOC gelöscht. Consolidation deaktiviert bis Neubau.
4. **PostgreSQL als Single Source of Truth**: InMemoryStore nur für Dev. Kein JSON-File in Production.
5. **nomic-embed-text lokal**: Keine API-Kosten. 768d, 8192 Context. Ollama.
6. **gte-modernbert ONNX**: Cross-Encoder ohne Ollama-Dependency. 599 MB.
7. **Source-Type Weighting**: Echte Konversationen > synthetische Injects.

---

## Wo finde ich was?

| Frage | Datei |
|-------|-------|
| "Wie wird ein Turn gespeichert?" | `api/routes.rs` → `store_session()` → `conversation.rs` |
| "Wie funktioniert die Suche?" | `retrieval/hybrid.rs` → `retrieve_fractal()` → `cross_encoder.rs` |
| "Wie entscheidet Governance?" | `memory/governance.rs` → `api/routes.rs` (apply_governance) |
| "Wie läuft Consolidation?" | `memory/dream/consolidation.rs` + `energy_decay.rs` |
| "Wie sind Embeddings strukturiert?" | `embedding/provider.rs` (Ollama/OpenAI) → `conversation.rs` (EmbeddingInfo) |
| "Wo ist das DB-Schema?" | `storage/postgres_store.rs` + `migrations/` (19 Files) |
| "Wie integriert Hermes?" | `hermes-plugin/knowwhere/__init__.py` |
| "Benchmark-Ergebnisse?" | `benchmarks/reports/` |

---

## Bekannte Problemzonen

1. **`api/routes.rs` (5884 LOC)**: Viel zu groß. Sollte nach Domain aufgeteilt werden (store, retrieve, admin, webhooks).
2. **Consolidation deaktiviert**: Größter Gap zu Hindsight (8-12pp Recall). Summarizer war Bloat, Neubau nötig.
3. **Reranker nicht aktiv**: ONNX-Modell liegt bereit, aber nicht in Pipeline.
4. **182 `unwrap()` Calls**: Viele davon in Test-Code, aber einige in Produktionspfaden.
5. **Matryoshka-Dimensionen**: 64d für Breitensuche verliert Precision. 768d/256d testen.

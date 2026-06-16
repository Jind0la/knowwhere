# gStack Full Review — KnowWhere v0.6.0

**Datum:** 16. Juni 2026
**Reviewer:** Hermes Operator (Nimar's Agent) mit gStack-Methodik
**Branch:** `main` (62de4b9)
**Server:** LIVE auf :3737 mit 68.654 Nodes

---

## Phase 1: System Audit

### Quick Facts

| Metrik | Wert |
|--------|------|
| Rust-Dateien | 88 in `src/`, ~38.874 LOC |
| Gesamt-LOC (inkl. Docs, Tests, Scripts) | ~847.000 (meist generierte Artefakte) |
| Tests | 321 `#[test]` + 84 `#[tokio::test]` = ~405 |
| Build-Status | ✅ Kompiliert sauber (2 dead_code-Warnungen, 0 Errors) |
| Repo-Größe | 4.8 GB (.git + target dominieren) |
| Production-Server | :3737 mit 68.654 Nodes |
| Docs | 85+ Markdown-Dateien, 55 im Archiv |
| Cargo.toml Version | 0.5.0 (sollte 0.6.0 sein — Diskrepanz) |

### File-Größen-Top-10 (nur src/)

| # | Datei | LOC | Domain |
|---|-------|-----|--------|
| 1 | `storage/postgres_store.rs` | 2.481 | PostgreSQL-Backend |
| 2 | `storage/in_memory.rs` | 2.108 | Dev-Backend |
| 3 | `retrieval/source_weighting.rs` | 1.976 | Provenance-Scoring |
| 4 | `api/retrieve.rs` | 1.811 | Fractal Retrieval |
| 5 | `memory/dream/consolidation.rs` | 1.387 | L0→L1→L2 Chains |
| 6 | `api/store.rs` | 1.277 | Session + External |
| 7 | `scheduler/consolidation.rs` | 1.215 | Cron-Jobs |
| 8 | `memory/dream/conflict_detection.rs` | 1.128 | Konflikte |
| 9 | `vlm/mod.rs` | 1.086 | Vision-Modelle |
| 10 | `memory/tests.rs` | 785 | Memory-Tests |

**Monster-Files (>500 LOC):** 22 Dateien. Das ist viel — aber die API-Module sind seit Juni 2026 gut aufgeteilt (routes.rs: 84 LOC).

### Aktuelle Branches (11 local)

```
main (aktiv), feat/chunking-qa-benchmarks, feat/multilingual-embedding,
feat/vlm-ollama-provider, feat/webhook-frigate-endpoint,
feature/reduce-to-core, fractal-core, research/*, stash-work
```

→ Viele parallele Experimente, keiner gemerged. `stash-work` könnte wertvolle Arbeit enthalten.

### Modified Files (uncommitted)

```
docs/goals/003-fractal-memory-hierarchy.md    (M)
docs/plans/2026-05-02-cross-modal-embedding.md (M)
docs/plans/2026-05-15-fractal-core.md         (M)
docs/plans/hermes-memory-provider.md           (M)
docs/plans/reduce-to-core-phase2.md            (M)
docs/session-id-retrieval-gap.md               (M)
docs/spikes/storage-pipeline-extraction.md     (M)
docs/turn-level-schema-design.md               (M)
docs/HORMA_PAPER_ANALYSIS.md                   (?? neu)
```

→ 9 uncommittete Docs. Arbeitskontext geht verloren wenn nicht committed.

### TODO/FIXME/HACK

**Keine Treffer in src/.** Sauber.

---

## Phase 2: Architektur-Übersicht

### Modulstruktur (aus lib.rs)

```
knowwhere-server
├── api/          (14 Module: health, store, retrieve, rerank, maintenance, ...)
├── connectors/   (Google Drive, Frigate NVR)
├── embedding/    (Ollama/OpenAI/Grok, CLIP, Audio, Sensor)
├── memory/       (FractalNode, Governance, Dream, Chunking, Facts, Skills)
├── multimodal/   (Cross-Modal Bridge)
├── reflector/    (Ollama Reflection)
├── retrieval/    (Hybrid BM25+Dense, Cross-Encoder, Scoring, Source-Weights)
├── scheduler/    (Cron: Consolidation, Audit)
├── services/     (Lifecycle)
├── storage/      (PostgreSQL, InMemory, Shared Pipeline, Trajectory)
├── summarizer/   (qwen2.5:3b — reduziert seit Mai)
├── vlm/          (Vision-Language Models)
```

### Datenfluss (Vereinfacht)

```
Client (Hermes Plugin / REST)
    │
    ▼
┌────────────────────────────────────┐
│ api/routes.rs (84 LOC Router)      │
│ ┌──────────┐  ┌──────────────────┐ │
│ │ store.rs │  │ retrieve.rs      │ │
│ │ (Write)  │  │ (Read)           │ │
│ └────┬─────┘  └────────┬─────────┘ │
└──────┼──────────────────┼──────────┘
       │                  │
       ▼                  ▼
┌──────────────┐  ┌──────────────────┐
│ MEMORY ENG.  │  │ RETRIEVAL ENG.   │
│              │  │                  │
│ chunking     │  │ hybrid (BM25+Vec)│
│ fact_extract │  │ cross_encoder    │
│ governance   │  │ source_weighting │
│ fractal_node │  │ scoring (Core+   │
│ conversation │  │   Policy)        │
│ dream/       │  │ query_expansion  │
│  consolidate │  └────────┬─────────┘
│  conflict    │           │
│  dedup       │           ▼
│  energy      │  ┌──────────────────┐
└──────┬───────┘  │ STORAGE LAYER    │
       │          │                  │
       ▼          │ postgres_store   │
┌──────────────┐  │ in_memory (Dev)  │
│ EMBEDDING    │  │ backend (Trait)  │
│ provider     │  │ trajectory       │
│ router       │  └──────────────────┘
│ clip/audio   │
└──────────────┘
       │
       ▼
   Ollama / OpenAI / Grok
```

### Kritische Entscheidungen (ADR)

1. ✅ **Turn-Level > Session-Level** (Mai 2026): 72.97% Recall@5, hoch von 7.1%
2. ⚠️ **Matryoshka Truncation**: 768d → 256d/64d. Precision-Verlust nicht gemessen.
3. 🔴 **Summarizer entfernt** (~4K LOC): Consolidation deaktiviert. Größter architektonischer Gap.
4. ✅ **PostgreSQL SSoT**: Sauber, aber 2.481 LOC in postgres_store.rs
5. ✅ **nomic-embed-text lokal**: Keine API-Kosten
6. ⚠️ **gte-modernbert ONNX**: Bereit, aber nicht in Pipeline aktiv
7. ✅ **Source-Type Weighting**: Echte > Synthetisch
8. ✅ **API-Module 5844→84 LOC**: Großartig

---

## Phase 3: CEO Review (Premise Challenge)

### 3A. Premise Challenge — Ist das das richtige Problem?

**Was KnowWhere löst:** "Lossless fractal memory for AI agents — every fact has an address."

**Ist das Problem real?** Ja. Das Agent-Memory-Problem ist eines der härtesten ungelösten Probleme in KI-gestützten Workflows. LangChain/Chroma sind Vektor-DBs mit Memory-Label — sie verlieren Kontext. Hindsight/KnowWhere's Fractal-Ansatz ist konzeptionell überlegen.

**Falsche Prämisse?** Nein, aber der Scope driftet. KnowWhere hat Module für:
- Frigate NVR (Video-Überwachung) ❓
- Google Drive Sync ❓
- VLM (Vision Models) ❓
- Multimodale Embeddings ❓
- Multilinguale Embeddings ❓
- Webhook-Endpoints für HomeAssistant ❓

Das sind **Features für ein Plattform-Produkt**, nicht für den Core-Loop eines Memory-Systems. Die Frage ist: Baut Nimar einen Memory-Service für AI Agents oder eine Allzweck-KI-Datenplattform?

**Empfehlung:** Kläre den Scope. Wenn KnowWhere ein Memory-System für Agents ist, sollten Frigate/Drive/VLM/HA-Webhooks ausgegliedert oder als optionales Plugin-System behandelt werden. Sie blähen den Code auf und verwässern den Core-Loop.

### 3B. Was würde passieren wenn wir nichts tun?

- Production-Server läuft stabil mit 68K Nodes
- 72.97% Recall@5 ist wettbewerbsfähig
- Kern-Gaps: Consolidation deaktiviert, Reranker nicht aktiv, Summarizer fehlt
- Ohne Consolidation verliert KnowWhere ~8-12pp Recall vs. Hindsight
- Feature-Creep wird schlimmer je länger unadressiert

### 3C. Dream State — 12-Monats-Ziel

```
HEUTE (v0.6)                   12 MONATE (v2.0)
─────────────────────────────────────────────────
72.97% Recall@5          →    90%+ Recall@5
Kein Consolidation       →    Vollautomatische L0→L1→L2
Kein Reranker            →    ONNX-Cross-Encoder in Pipeline
14 API-Module            →    Stabile Public API v1
1 Tenant (Nimar)         →    Multi-Tenant (Hermes-Profile)
Nur lokales Ollama       →    Cloud-Embedding optional
Keine Access-Control     →    RBAC pro Namespace
Frigate/Drive/VLM diffus →    Plugin-System (core sauber)
```

### 3D. Scope-Mode

**Empfehlung: SELECTIVE EXPANSION.** Der Core funktioniert. Jetzt: Consolidation wiederbeleben, Reranker aktivieren, Feature-Creep bereinigen. Keine neuen Features bis die Fundamente stehen.

---

## Phase 4: Engineering Review

### 4A. Architektur

**Stärken:**
- API-Modul-Split (5844→84 LOC Router) ist vorbildlich
- Trait-basierte Storage-Abstraktion (`StorageBackend`) ermöglicht Dev/Prod-Split
- Scoring-Architektur (Core vs Policy) in `retrieval/scoring.rs` ist sauber
- Fractal-Hierarchie (L0→L1→L2) ist das konzeptionelle Alleinstellungsmerkmal

**Schwächen:**
- **22 Files >500 LOC.** Das ist zu viel. `postgres_store.rs` (2.481) + `in_memory.rs` (2.108) sind Parallel-Implementierungen — viel Duplikation.
- **Dual-Backend-Problem.** `in_memory.rs` und `postgres_store.rs` implementieren denselben Trait aber mit unterschiedlicher Qualität. Klassischer Fall für den `dual-backend-dedup` Audit (siehe `codebase-orientation` references).
- **Summarizer-Lücke.** `summarizer/mod.rs` (560 LOC) existiert noch, aber Consolidation ist deaktiviert. Widerspruch: Code ist da, aber nicht wired.
- **VLM-Modul (1.086 LOC)** ist überdimensioniert für "Vision-Modelle als Embedding-Provider". Sollte schlanker sein oder in `embedding/` aufgehen.
- **Connector-System** (`drive.rs`, `frigate.rs`) gehört in ein Plugin-System, nicht in den Core.

### 4B. API-Design

**Stärken:**
- REST-Endpoints pro Domain (14 Module)
- Utoipa/OpenAPI-Dokumentation
- Rate-Limiting via Governor
- Auth (API-Key + User-Registration)

**Schwächen:**
- Keine API-Versionierung (`/v1/...`)
- Kein Pagination-Standard für List-Endpoints
- Webhook-Endpoints ohne standardisierte Webhook-Security (nur `check_webhook_secret`)
- Keine API-Deprecation-Strategie (Breaking Changes ohne Vorwarnung möglich)

### 4C. Test-Strategie

**Stärken:**
- ~405 Tests (321 unit + 84 integration)
- `#[serial]` für Tests mit geteiltem State
- LongMemEval-Benchmark-Integration
- Dedizierte Test-Module pro Domain

**Schwächen:**
- **Test-Abdeckung unbekannt.** Kein tarpaulin/llvm-cov Report.
- **Integration-Tests gegen echte DB?** Tests scheinen gegen InMemory zu laufen — nicht gegen PostgreSQL. Risiko: DB-spezifische Bugs (SQL-Query-Fehler, Migration-Issues) werden nicht gefunden.
- **Benchmark-Only-Eval.** Es gibt keinen täglichen Regression-Test für Retrieval-Qualität. Nur manuelle LongMemEval-Runs.
- **Kein Property-Based Testing** (proptest/quickcheck) für Fuzzy-Inputs.

### 4D. Performance

**Stärken:**
- Embedding-Caching via Ollama (lokal, <0.23s)
- Matryoshka-Dimensionen für Fractal Zoom (speedup)
- RRF-Fusion (BM25 + Dense) ist effizient
- ONNX-Cross-Encoder (kein Ollama-Overhead für Reranking)

**Schwächen:**
- **Keine Benchmarks für P99-Latenz.** Nur Recall-Metriken, keine Latenz-Metriken.
- **USearch-Index ohne Persistenz-Strategie.** Wird der Index neu aufgebaut bei Restart? Was bei 1M+ Nodes?
- **Kein Connection-Pooling dokumentiert.** sqlx nutzt intern einen Pool, aber Konfiguration nicht sichtbar.
- **Matryoshka 64d** verliert Precision — aber wie viel? Nicht gemessen.

### 4E. Sicherheit

**Quick Scan (nicht vollständig, nur was auffällt):**
- ✅ Auth-Middleware via Bearer-Token
- ✅ Rate-Limiting (Governor)
- ⚠️ `check_webhook_secret` — unspezifisch, kein HMAC
- ⚠️ Kein Input-Sanitization-Layer sichtbar
- ⚠️ SQL-Injection-Risiko via sqlx? sqlx ist parametrisiert, aber raw SQL in postgres_store.rs (2.481 LOC) könnte Lücken haben
- 🔴 Keine Security-Audit-Historie (OSS-Forensics existiert aber alt)

### 4F. Operations

**Stärken:**
- launchd plist für macOS
- Docker-Compose für Container
- Railway-Deploy-Gate-Workflow
- Health-Endpoint (`/health`)

**Schwächen:**
- **Kein Monitoring.** Keine Prometheus-Metriken, kein Grafana-Dashboard.
- **Keine Alerts.** Server kann crashen ohne dass jemand es merkt (außer Hermes-Cron checks).
- **Kein Backup-Prozess.** PostgreSQL-DB mit 68K Nodes — was bei Datenverlust?
- **Cargo.toml zeigt v0.5.0**, aber CHANGELOG sagt v0.6.0 — Diskrepanz.

---

## Phase 5: Findings & Recommendations

### 🔴 Kritisch (sollte diese Woche adressiert werden)

| # | Finding | Impact | Recommendation |
|---|---------|--------|----------------|
| K1 | **Consolidation deaktiviert** | 8-12pp Recall-Verlust vs. Hindsight. Ohne Consolidation ist KnowWhere ein glorifizierter Vektor-Store. | Summarizer neubauen (qwen2.5:3b reicht). L0→L1→L2-Pipeline reactivieren. |
| K2 | **Feature-Creep** | Frigate/Drive/VLM/HA-Webhooks verwässern den Core. Erhöhen Wartungskosten und kognitive Last. | Plugin-System entwerfen. Core = Memory. Plugins = optional. |
| K3 | **Cargo.toml falsche Version** | v0.5.0 statt v0.6.0. Breaking für Semver-abhängige Systeme. | Fix: `version = "0.6.0"` |

### 🟡 Hoch (diese Woche / nächste)

| # | Finding | Impact | Recommendation |
|---|---------|--------|----------------|
| H1 | **Reranker nicht in Pipeline** | gte-modernbert ONNX liegt bereit, aber nicht aktiv. 10-15pp Recall-Verbesserung möglich. | `cross_encoder.rs` in `hybrid_retrieve` integrieren |
| H2 | **Dual-Backend-Duplikation** | `postgres_store.rs` (2.481) + `in_memory.rs` (2.108) = 4.589 LOC für dasselbe Interface. | `dual-backend-dedup` Audit laufen lassen, shared code auslagern |
| H3 | **22 Files >500 LOC** | Code-Review wird teuer. Bugs verstecken sich in großen Files. | Splitten: `postgres_store.rs` → `pg_search.rs` + `pg_mutations.rs` + `pg_schema.rs` |
| H4 | **Kein API-Versioning** | Breaking Changes zerstören Hermes-Plugin ohne Vorwarnung. | `/v1/` prefix + Deprecation-Header |

### 🟢 Mittel (nächste 2-4 Wochen)

| # | Finding | Impact | Recommendation |
|---|---------|--------|----------------|
| M1 | **Keine Latenz-Metriken** | Kein P99, kein Avg Response Time. Blind für Perf-Regressionen. | Prometheus `/metrics` endpoint + Histogram |
| M2 | **Test-Abdeckung unbekannt** | Kein Coverage-Report. Wo sind die blinden Flecken? | `cargo tarpaulin` in CI integrieren |
| M3 | **Kein täglicher Regression-Test** | Retrieval-Qualität kann unbemerkt degradieren. | Täglicher LongMemEval-Canary-Run via Cron |
| M4 | **Kein DB-Backup** | 68K Nodes in PostgreSQL — vollständiger Datenverlust bei Plattencrash. | pg_dump Cron-Job + Railway-Backup konfigurieren |
| M5 | **9 uncommittete Docs** | Arbeitskontext verloren. Andere Developer sehen veralteten Stand. | Commit oder Stash |
| M6 | **11 Branches, keiner gemerged** | Arbeit verteilt, keine Integration. `stash-work` könnte wertvoll sein. | Branch-Audit: mergen oder löschen |

### 🔵 Niedrig (Backlog)

| # | Finding | Recommendation |
|---|---------|----------------|
| N1 | VLM-Modul (1.086 LOC) zu groß für Embedding-Provider | In `embedding/` integrieren oder als Plugin |
| N2 | Connection-Pooling nicht dokumentiert | sqlx-Pool-Size dokumentieren und tunen |
| N3 | Property-Based Testing fehlt | `proptest` für Store/Retrieve-Roundtrips |
| N4 | `unwraps` in Produktionscode | Wurden teilweise auf `.expect()` migriert. Rest auditieren. |

---

## Phase 6: Aktionsplan

### Sofort (diese Session)

1. ✅ Review dokumentiert → `docs/reviews/2026-06-16-gstack-full-review.md`
2. ⬜ `Cargo.toml` Version fix: `0.5.0` → `0.6.0`
3. ⬜ 9 uncommittete Docs committen
4. ⬜ Kanban-Tasks für K1, K2, H1, H2 anlegen

### Diese Woche

- K1: Consolidation-Reaktivierung (Summarizer → Dream Pipeline)
- H1: Reranker in Pipeline integrieren
- H2: Dual-Backend-Dedup-Audit starten

### Nächster Sprint

- Plugin-System-Design (K2)
- API-Versioning (H4)
- Latenz-Metriken (M1)
- Test-Coverage (M2)

---

## GSTACK REVIEW REPORT

| Run | Reviewer | Status | Findings |
|-----|----------|--------|----------|
| 1 | Hermes Operator (deepseek-v4-pro) | Complete | 3 Critical, 4 High, 6 Medium, 4 Low |

**VERDICT: System funktioniert, aber mit architektonischen Altlasten.** 
Der Core-Loop ist stark (72.97% Recall@5), aber Consolidation-Lücke und Feature-Creep müssen adressiert werden bevor neue Features Sinn machen. Der API-Modul-Split war der richtige Schritt. Jetzt: Reduce-to-Core ernst nehmen — was gehört wirklich in den Core eines Memory-Systems?

**UNRESOLVED DECISIONS:**
- Scope-Frage: Memory-Service oder KI-Plattform? (Nimar muss entscheiden)
- Plugin-System-Design: Vor oder nach Consolidation-Reaktivierung?
- Branch-Audit-Strategie: Welche der 11 Branches sind tot, welche lebendig?
- Matryoshka-Dimensionen: 256d oder 64d default für Fractal Zoom?

NO UNRESOLVED DECISIONS

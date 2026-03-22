# KnowWhere — Task-Liste

> Erstellt: 2026-03-21 | Letztes Update: 2026-03-22  
> Status: 🟡 In Bearbeitung

---

## 🔴 Kritisch

### [CRIT-001] Timing-Angriff im API-Key-Vergleich ✅

**Erledigt.** Constant-time comparison via `subtle::ConstantTimeEq`.

**Commit:** `e2182f2`

---

### [CRIT-002] Rate-Limiting auf Auth-Endpoints ✅

**Erledigt.** Zwei Iterationen — erst nur `/auth/*`, dann alle geschützten Endpoints.

**Commits:** `2a0d58e`, `32d5023`

---

### [CRIT-003] JSON-Persistenz → PostgreSQL

**Status:** 🟡 Gute Fortschritte — StorageBackend Trait definiert, MemoryStore implementiert

**Was bisher (diese Woche):**
- ✅ StorageBackend Trait definiert (backend-agnostic, kein PgPool-Leak)
- ✅ MemoryStore implementiert StorageBackend
- ✅ AppState nutzt `Arc<dyn StorageBackend>` für API-Layer

**Was noch fehlt:**
- `PostgresStore` ans Trait implementieren
- Schema-Migration auf PostgreSQL
- USearch + PostgreSQL dual maintain (oder pgvectorscale als Option D)

**Aufwand:** ~1 Tag  
**Dateien:** `src/storage/postgres_store.rs`, `src/storage/backend.rs`

---

## 🟡 Mittelfristig

### [MED-001] Lineares → Exponentielles Decay-Modell ✅

**Erledigt.** Ebbinghaus-Kurve mit `halflife_hours` Parameter.

**Commit:** `4bd5c98`

---

### [MED-002] Tiered Compaction — LLM-Summarization statt Truncation

**Status:** 🟡 Offen

**Problem:** `generate_overview()` und `generate_summary()` sind Truncation (erste 50 Zeichen) — macht Tiered Context wirkungslos.

**Aufwand:** ~1 Tag  
**Dateien:** `src/memory/tiered.rs`

---

### [MED-003] BM25-Corpus nach Neustart für External-Nodes

**Status:** 🟡 Offen

**Problem:** `bm25_corpus` fehlt in `PersistedState`. External-Nodes verlieren BM25-Eintrag nach Neustart.

**Aufwand:** ~2 Stunden  
**Dateien:** `src/storage/in_memory.rs`

---

### [MED-004] O(n²) Conflict Detection → Vektor-Ähnlichkeit

**Status:** 🟡 Offen

**Problem:** `detect_confidence_conflicts()` macht exakten String-Match. Skaliert nicht bei 100k+ Erinnerungen.

**Aufwand:** ~4 Stunden  
**Dateien:** `src/memory/governance.rs`

---

### [MED-005] Doppelte Governance-Logik konsolidieren

**Status:** 🟡 Offen

**Problem:** `GovernanceValidator::validate()` und `GovernanceCandidate::apply_governance()` implementieren ähnliche Logik.

**Aufwand:** ~1 Stunde  
**Dateien:** `src/memory/governance.rs`

---

### [MED-006] Test-Fixture Fix ✅

**Erledigt.** AppState hatte fehlende Felder (`events`, `governance_policy`).

**Commit:** `da43722`

---

### [MED-007] Tests mit OpenAI Embeddings ✅

**Erledigt.** Tests nutzen jetzt `OpenAIProvider` statt `LocalOllama`. CI-Secret gesetzt.

**Commits:** `8d38b55`, `c13902c`

---

### [MED-008] StorageBackend Trait für interne Komponenten

**Status:** 🟡 Offen

**Problem:** VLM Worker, ConsolidationScheduler, AuditScheduler und FrigateConnector nutzen konkretes `MemoryStore` statt `Arc<dyn StorageBackend>`. Das verhindert volle Backend-Flexibilität.

**Aufwand:** ~3 Stunden  
**Dateien:** `src/main.rs`, `src/vlm/mod.rs`, `src/memory/dream/`

**Hinweis:** Niedrig priorisiert — aktueller Stand (konkretes MemoryStore intern, Trait für API) ist funktional ausreichend. Erst relevant wenn echte Multi-Backend-Unterstützung gebraucht wird.

---

## 🟢 Niedrig

### [LOW-001] FractalNode.children: Arena-Allocation

**Status:** 🟢 Offen

**Problem:** `children: Vec<FractalNode>` speichert inline. Tiefes Klonen kopiert alle Kinder + Enkel.

**Aufwand:** ~3 Tage (größere Architektur-Änderung)  
**Empfehlung:** Niedrig priorisieren, bis Performanz-Problem messbar

---

### [LOW-002] Embedding-Batching-Support

**Status:** 🟢 Offen

**Problem:** `embed()` unterstützt nur Einzel-Embeddings. Bulk-Import macht N sequenzielle HTTP-Requests.

**Aufwand:** ~4 Stunden  
**Dateien:** `src/embedding/provider.rs`

---

### [LOW-003] CI erweitern: Clippy, Audit, Fmt

**Status:** 🟢 Offen

**Problem:** CI macht nur check + test. Fehlt Linting, Security-Audit, Format-Check.

**Aufwand:** ~1 Stunde  
**Dateien:** `.github/workflows/ci.yml`

---

### [LOW-004] RRF statt Score-Addition für Hybrid-Retrieval ✅

**Erledigt.** `rrf_fuse()` existiert in `in_memory.rs` — nutzt `k=60`, state-of-the-art.

---

## 📋 Aktuelle Übersicht

| Task | Status | Aufwand | Commit |
|------|--------|---------|--------|
| CRIT-001 Timing-Angriff | ✅ Erledigt | 30min | e2182f2 |
| CRIT-002 Rate-Limiting | ✅ Erledigt | 2h | 2a0d58e, 32d5023 |
| CRIT-003 PostgreSQL | 🟡 In Progress | ~1 Tag | StorageBackend ✅ |
| MED-001 Exp. Decay | ✅ Erledigt | 1h | 4bd5c98 |
| MED-002 LLM Compaction | 🟡 Offen | ~1 Tag | — |
| MED-003 BM25 Persistenz | 🟡 Offen | ~2h | — |
| MED-004 Vektor Conflict | 🟡 Offen | ~4h | — |
| MED-005 Gov. Dedup | 🟡 Offen | ~1h | — |
| MED-006 Test-Fixture | ✅ Erledigt | 1h | da43722 |
| MED-007 OpenAI Tests | ✅ Erledigt | 30min | 8d38b55 |
| MED-008 StorageBackend Intern | 🟡 Offen | ~3h | — |
| LOW-001 Arena | 🟢 Offen | ~3 Tage | — |
| LOW-002 Batch Embed | 🟢 Offen | ~4h | — |
| LOW-003 CI erweitern | 🟢 Offen | ~1h | — |
| LOW-004 RRF | ✅ Erledigt | — | rrf_fuse() |

---

## 🔗 Research-Quellen

- Auth Security: subtle crate, axum-governor docs
- Decay Model: Ebbinghaus curve, spaced repetition literature
- RRF: OpenSearch RRF implementation, TopK hybrid retrieval research

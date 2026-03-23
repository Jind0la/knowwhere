# KnowWhere — Task-Liste

> Erstellt: 2026-03-21 | Letztes Update: 2026-03-22  
> Status: 🟢 Gute Fortschritte

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

### [CRIT-003] JSON-Persistenz → PostgreSQL ✅

**Erledigt.**

**Was implementiert wurde:**
- `StorageBackend` Trait in `src/storage/backend.rs` — backend-agnostic, kein PgPool-Leak
- `PostgresStore` implementiert alle 11 Trait-Methoden (insert, get, delete, update_vector, update, hybrid_retrieve, retrieve_fractal, search_bm25, list_all, recent, count, purge_dummy_vectors)
- RRF-Fusion für hybrid_retrieve (Vektor + BM25 via Reciprocal Rank Fusion)
- `search_bm25` via PostgreSQL `ts_rank` als BM25-Approximation
- `update_vector` und `purge_dummy_vectors` als neue Methoden

**Commits:** `6f9cfc6`, `4cfa5b7`

---

## 🟡 Mittelfristig

### [MED-001] Lineares → Exponentielles Decay-Modell ✅

**Erledigt.** Ebbinghaus-Kurve mit `halflife_hours` Parameter.

**Commit:** `4bd5c98`

---

### [MED-002] Tiered Compaction — LLM-Summarization statt Truncation ✅

**Erledigt.** 

**Was implementiert:**
- Retrieval-optimierte Prompts: L1 Overview + L0 Summary mit System-Directive + User-Template
- `TieredCompactionWorker::compact_memory()` → thin dispatcher, enqueued VlmJob, VlmWorker async
- VLM-Fehler → truncation fallback (statt silent drop), compaction chain completes
- `SummaryContext::Detailed` → `Overview` Bug in consolidation.rs gefixt
- Filter: importance >= 3, content_length > 500
- Budget-Cap: `DREAM_VLM_MAX_JOBS_PER_CYCLE` (default 100)

**Commits:** `279265c`, `7bf6f01`

---

### [MED-003] BM25-Corpus Persistenz ✅

**Erledigt.** `bm25_corpus` ist jetzt Teil von `PersistedState` in state.json.

**Was geändert:**
- `PersistedState` um `bm25_corpus: Vec<(Uuid, String)>` erweitert
- `save_to_disk()` speichert bm25_corpus mit
- `load_state()` lädt bm25_corpus direkt — kein Rebuild aus nodes mehr
- Backward compat: alte state.json (ohne bm25_corpus) rebuilt aus nodes

**Commit:** `fcee458`

---

### [MED-004] O(n²) Conflict Detection → Vektor-Ähnlichkeit ✅

**Erledigt.** Vektor-basierte Conflict Detection mit Cosine Similarity.

**Was implementiert:**
- `ConflictDetector` unterstützt jetzt `PgPool` und `Arc<dyn StorageBackend>`
- `detect_confidence_conflicts_vector()` — cosine similarity > 0.85 threshold
- Semantischer Check via `content_contradicts()` (Negations-Patterns)
- Fallback auf String-Match wenn kein Vektor-Backend verfügbar
- Batch-Verarbeitung (50er chunks)

**Commit:** `352505d`

---

### [MED-005] Doppelte Governance-Logik konsolidieren ✅

**Erledigt.** `GovernancePolicy::governance_check()` als shared core.

**Was geändert:**
- `GovernanceValidator::validate()` und `GovernanceCandidate::apply_governance()` nutzen jetzt beide `policy.governance_check()` als shared logic
- Duplizierte Checks (confidence, superseded, sensitivity, status) sind konsolidiert

**Commit:** `dab48dc`

---

### [MED-006] Test-Fixture Fix ✅

**Erledigt.** AppState hatte fehlende Felder (`events`, `governance_policy`).

**Commit:** `da43722`

---

### [MED-007] Tests mit OpenAI Embeddings ✅

**Erledigt.** Tests nutzen jetzt `OpenAIProvider` statt `LocalOllama`. CI-Secret gesetzt.

**Commits:** `8d38b55`, `c13902c`

---

### [MED-008] StorageBackend Trait für interne Komponenten ✅

**Erledigt.**

**Was implementiert wurde:**
- `UpdateOperation` Enum: `MultiplyWeight`, `SetWeight`, `SetParentTierId`, `SetStatus`, `ApplyAudit` (dyn Trait-kompatibel)
- `StorageBackend::update(id, UpdateOperation)` — ersetzt closure-basiertes `update_node`
- `MemoryStore::update()` delegiert an bestehendes `update_node()`
- `PostgresStore::update()` mit SQL UPDATE pro Operation
- Alle internen Komponenten migriert: DreamMode, ConsolidationScheduler, AuditScheduler, VlmWorker

**Commits:** `eceb6e2`, `b4244db`, `4cfa5b7`

---

## 🟢 Niedrig

### [LOW-001] FractalNode: Hotpath Clone Elimination

**Status:** 🟡 In Evaluation

**Problem:** `zoom_retrieve()` gab `Vec<(f32, FractalNode)>` zurück — `self.clone()` bei jedem Rekursions-Schritt. Hotpath: alle Results werden gecloned, sortiert, dann nur `top_k` behalten → 90%+ Clones umsonst.

**Lösung (LOW-001a ✅):** Signatur geändert zu `Vec<(f32, &FractalNode)>` — Referenz-Rückgabe. Commit `7db3c80`.

**LOW-001b (offen):** Evaluation: Ist Arena Allocation nach dem Clone-Elimination-Fix noch nötig?

**Externer Review:** 23.03.2026

---

### [LOW-001b] Evaluation: Arena Allocation — Notwendig? (DONE)

**Fazit: NEIN. Arena Allocation ist zum jetzigen Zeitpunkt nicht gerechtfertigt.**

---

#### Analyse: Was passiert nach LOW-001a noch mit FractalNode clones?

**Hotpath `zoom_retrieve` (jetzt mit LOW-001a):**
```rust
// fractal_node.rs — KEIN clone() mehr
pub fn zoom_retrieve<'a>(&'a self, ...) -> Vec<(f32, &'a FractalNode)> {
    let mut results = vec![(sim, self)];  // Referenz, kein Clone
    if let Some(best) = self.find_best_child(query_vector) {
        results.extend(best.zoom_retrieve(...));  // Referenzen durchgereicht
    }
}

// in_memory.rs — NUR top_k Nodes werden geclont (nach Sort)
scored.into_iter().take(top_k).map(|(_, n)| n.clone()).collect()
```
→ Nur die wenigsten Nodes (top_k, typisch 5-20) werden am Ende geclont. Kein deep clone der gesamten Tree-Struktur mehr.

**Verbleibende FractalNode::clone() Stellen im Code:**

| Ort | Was | Häufigkeit | Problem? |
|-----|-----|------------|----------|
| `in_memory.rs:799,819` | `.map(\|(_, n)\| n.clone())` | Pro Retrieval-Call, N=top_k | ✅ Nur top_k, nicht whole tree |
| `in_memory.rs:496` `recent()` | `nodes.values().cloned().collect()` | Pro `/recent` API-Call | ⚠️ Nicht zoom_retrieve hotpath |
| `dream/mod.rs:186` | `vec![a.clone(), b.clone()]` | Pro Consolidation-Cycle | ✅ Hintergrund-Task, selten |
| `postgres_store.rs` | Einzelne Feld-clones (vector, content) | Pro Insert/Update | ✅ Kein deep clone |

**Was Arena lösen würde:**
Wenn jemand `FractalNode::clone()` aufruft → rekursiv alle Kinder + Vektoren + Metadata klonen.

**Was tatsächlich passiert:**
- Nach LOW-001a: `zoom_retrieve` klont nicht mehr rekursiv
- `find_best_child()` iteriert nur über `children` (Referenz, kein Clone)
- `children: Vec<FractalNode>` wird NUR noch gelesen (via Referenz), nicht mehr geklont im Hotpath
- Die verbleibenden Clones sind einzelne Nodes (top_k, background tasks)

**Würde Arena das `children`-Problem lösen?**
Arena löst das Problem "wenn ich einen Node klone, klont er alle Kinder mit". Aber:
1. Niemand klont im Hotpath noch ganze Subtrees
2. `children` wird nur noch iteriert, nicht mehr rekursiv geklont
3. Selbst wenn jemand `node.clone()` aufruft — das wäre ein Bug oder bewusster Akt, kein alltäglicher Pfad

**Performance-Problem heute:**
- `recent()` klont alle Nodes → aber das ist ein anderer Code-Pfad
- `zoom_retrieve` hotpath ist jetzt optimal (Referenzen)

** Empfehlung:**
- Arena Allocation auf "Future / Only if profiling shows it" setzen
- Stattdessen: `recent()` optimieren wenn es zum Bottleneck wird (aber aktuell kein Problem)
- Arena wäre auch breaking change für Serialisierung (Arc<Self> ist nicht Serialize-friendly ohne custom impl)

**Conclusion:** Arena Allocation löst ein Problem, das nach LOW-001a nicht mehr existiert. **LOW-001b = Done, Arena not needed.**

---

### [LOW-002] Embedding-Batching-Support

**Status:** 🟢 Offen

**Problem:** `embed()` unterstützt nur Einzel-Embeddings. Bulk-Import macht N sequenzielle HTTP-Requests.

**Aufwand:** ~4 Stunden  
**Dateien:** `src/embedding/provider.rs`

---

### [LOW-003] CI erweitern: Clippy, Audit, Fmt ✅

**Erledigt.** CI jetzt mit drei zusätzlichen Steps.

**Was hinzugefügt:**
- `cargo fmt --all -- --check`: Format-Prüfung
- `cargo clippy --all-targets -D warnings`: Linting, dead code, style (treats warnings as errors)
- `cargo audit`: Security-Vulnerability-Check in Dependencies

**Commit:** `06be859`

---

### [LOW-004] RRF statt Score-Addition für Hybrid-Retrieval ✅

**Erledigt.** `rrf_fuse()` existiert in `in_memory.rs` — nutzt `k=60`, state-of-the-art.

---

### [LOW-005] DreamStatus mit cycle_count ✅

**Erledigt.** `cycle_count` jetzt in `ConsolidationScheduler` via `AtomicU64`, exponiert über `GET /dream/status`.

**Was implementiert:**
- `ConsolidationScheduler::cycle_count: Arc<AtomicU64>`
- `spawn()` gibt `Arc<Self>` zurück für API-Zugriff
- `DreamStatus.cycle_count: u64` Feld
- `/dream/status` liest `cycle_count` aus Scheduler
- `AppState` speichert `Option<Arc<ConsolidationScheduler>>`

**Commits:** `3238e1a`, `f34e8fc`

---

## 📋 Aktuelle Übersicht

| Task | Status | Aufwand | Commit |
|------|--------|---------|--------|
| CRIT-001 Timing-Angriff | ✅ | 30min | e2182f2 |
| CRIT-002 Rate-Limiting | ✅ | 2h | 2a0d58e, 32d5023 |
| CRIT-003 PostgreSQL | ✅ | ~1 Tag | 6f9cfc6, 4cfa5b7 |
| MED-001 Exp. Decay | ✅ | 1h | 4bd5c98 |
| MED-002 LLM Compaction | ✅ | ~1 Tag | 279265c, 7bf6f01 |
| MED-003 BM25 Persistenz | ✅ | ~2h | fcee458 |
| MED-004 Vektor Conflict | ✅ | ~4h | 352505d |
| MED-005 Gov. Dedup | ✅ | ~1h | dab48dc |
| MED-006 Test-Fixture | ✅ | 1h | da43722 |
| MED-007 OpenAI Tests | ✅ | 30min | 8d38b55 |
| MED-008 StorageBackend Intern | ✅ | ~4h | eceb6e2, b4244db, 4cfa5b7 |
| LOW-001 Arena | 🟢 Offen | ~3 Tage | — |
| LOW-002 Batch Embed | 🟢 Offen | ~4h | — |
| LOW-003 CI erweitern | ✅ | ~1h | 06be859 |
| LOW-004 RRF | ✅ | — | rrf_fuse() |
| LOW-005 DreamStatus + cycle_count | ✅ | ~30min | 3238e1a, f34e8fc |

---

## 🔗 Research-Quellen

- Auth Security: subtle crate, axum-governor docs
- Decay Model: Ebbinghaus curve, spaced repetition literature
- RRF: OpenSearch RRF implementation, TopK hybrid retrieval research

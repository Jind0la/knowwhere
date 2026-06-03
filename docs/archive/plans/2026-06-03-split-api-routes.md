# Plan: `api/routes.rs` aufteilen

> **Für Hermes:** Benutze `subagent-driven-development` Skill um diesen Plan Task-für-Task umzusetzen.

**Goal:** `src/api/routes.rs` von 5.884 LOC auf ~300 LOC reduzieren durch Extraktion von Handler-Funktionen in domain-spezifische Submodule.

**Architecture:** Jede Domäne (store, retrieve, maintenance, memory-admin, namespaces, skills, turns) bekommt ein eigenes Modul. Gemeinsame Typen (ScoredNode, RetrievalProfile, etc.) wandern in `src/api/types.rs`. `routes.rs` wird zum reinen Router — nur Modul-Deklarationen und `axum::Router`-Zusammenbau.

**Tech Stack:** Rust, Axum 0.8, Keine neuen Dependencies.

**Working Dir:** `~/knowwhere`

**Constraints:**
- Keine Verhaltensänderungen — nur Code-Verschiebung
- Alle bestehenden Tests müssen weiter kompilieren und bestehen
- `cargo check` nach JEDEM Schritt
- Commit nach jedem erfolgreichen Modul-Extract
- `src/api/mod.rs` muss alle Public-Reexports aktualisieren

---

## Vorbereitung: Code verstehen

Bevor wir anfangen: die aktuelle Struktur analysieren.

```
src/api/routes.rs (5884 LOC)
├── [1-43]       Imports + Submodul-Deklarationen
├── [44-208]     Gemeinsame Typen: ScoredNode, RetrievalScoreDebug, RetrievalProfile, 
│                HealthResponse, EmbedRequest/Response, RetrievalRequest, etc.
├── [209-443]    Hilfsfunktionen: clean_for_embedding, build_retrieval_request_from_query
├── [444-474]    Health + Embed (health, embed_text)
├── [476-1660]   Store Session (store_session, store_session_json, store_session_binary)
├── [1661-1841]  Store Session Batch (store_session_batch)
├── [1842-1993]  Self-Improve (self_improve)
├── [1995-2123]  Store External (store_external)
├── [2124-2535]  Retrieve (retrieve, retrieve_fractal, persist_chat_exchange)
├── [2537-3350]  Fractal Retrieve (retrieve_fractal, retrieve_fractal_safe, apply_temporal_diversity)
├── [3361-3451]  Subconscious Chat (subconscious_chat) — verwendet QA-Logik aus subconscious_qa.rs
├── [3453-3554]  Rerank (rerank)
├── [3555-3900]  Delete/Maintenance (delete_node, batch_delete, deduplicate, purge, recent,
│                reembed, repair, dream_status)
├── [3901-4199]  Trajectory (list_retrieval_runs, get_retrieval_run, get_retrieval_trajectory,
│                compact_memory, get_memory)
├── [4200-4312]  Conflicts (list_conflicts, resolve_conflict)
├── [4313-4478]  Energy/Decay (boost_memory_energy, list_low_energy, apply_energy_decay,
│                compress_memory_cluster)
├── [4479-4588]  Deduplication (list_deduplication_candidates, run_deduplication, list_runs)
├── [4589-4799]  Self-Healing (reindex_external_node, memory_health_check, self_healing_stats)
├── [4800-5099]  Namespaces (list, get, memories, create, search)
├── [5100-5509]  Skills (create, list, get, update, delete, use, match, entity_search)
└── [5510-5884]  Turn Handlers (store_turn, store_turns_batch, retrieve_turns, get_session_turns)
```

**Bestehende Submodule (bleiben unverändert):**
- `src/api/auth.rs` (466 LOC) — Authentifizierung
- `src/api/docs.rs` (165 LOC) — OpenAPI/Swagger
- `src/api/turns.rs` (154 LOC) — Turn-Typen (Request/Response structs)
- `src/api/subconscious_qa.rs` (664 LOC) — QA-Logik für subconscious_chat
- `src/api/webhooks.rs` (99 LOC) — Webhook-Typen
- `src/api/routes/governance_events.rs` (198 LOC) — Governance-Event-Endpoints
- `src/api/routes/webhooks.rs` (409 LOC) — Webhook-Endpoints

---

## Phase 1: Gemeinsame Typen extrahieren

### Task 1.1: `src/api/types.rs` erstellen

**Objective:** Gemeinsame Request/Response-Typen aus routes.rs in ein Shared-Modul verschieben.

**Files:**
- Create: `src/api/types.rs`
- Modify: `src/api/routes.rs` (Zeilen 44-208 entfernen)
- Modify: `src/api/mod.rs` (neues Modul registrieren)

**Step 1: Datei erstellen**

```bash
touch src/api/types.rs
```

**Step 2: Typen aus routes.rs verschieben**

Aus `src/api/routes.rs` Zeilen ~44-208 kopieren nach `src/api/types.rs`:
- `ScoredNode`
- `RetrievalScoreDebug`
- `RetrievalProfile`
- `HealthResponse`
- `EmbedRequest` / `EmbedResponse`
- `RetrievalRequest`
- `FractalRetrieveRequest` / `FractalRetrieveResponse`
- `StoreSessionRequest` / `StoreSessionResponse`
- `StoreExternalRequest` / `StoreExternalResponse`
- `SelfImproveRequest` / `SelfImproveResponse`
- `SubconsciousChatRequest` / `SubconsciousChatResponse`
- `RerankRequest` / `RerankResponse`
- Alle weiteren Request/Response-Typen
- `clean_for_embedding()` Hilfsfunktion

**Step 3: `src/api/mod.rs` updaten**

```rust
pub mod types;
```

**Step 4: `src/api/routes.rs` imports fixen**

```rust
use crate::api::types::*;
```

**Step 5: Kompilieren prüfen**

```bash
cargo check 2>&1
```

Erwartet: Compile-Errors nur für noch nicht aufgelöste Imports in anderen Files.

**Step 6: Alle `crate::api::routes::X` → `crate::api::types::X` in anderen Dateien fixen**

```bash
grep -rn "api::routes::" src/ --include="*.rs"
```

Jeden Fund auf `api::types::` umbiegen, dann:
```bash
cargo check 2>&1
```

Erwartet: `cargo check` sauber.

**Step 7: Commit**

```bash
git add src/api/types.rs src/api/routes.rs src/api/mod.rs
git commit -m "refactor: extract shared API types into src/api/types.rs"
```

---

## Phase 2: Handler-Domänen extrahieren

**Für JEDE der folgenden Tasks gilt das gleiche Muster:**

1. Neue Modul-Datei erstellen (`src/api/<name>.rs`)
2. Handler-Funktionen + ihre Helper aus routes.rs verschieben
3. Imports in der neuen Datei setzen (nur was gebraucht wird)
4. `use crate::api::<name>::*;` in routes.rs
5. `cargo check` → Fehler fixen → `cargo check` sauber
6. Commit

### Task 2.1: `src/api/health.rs`

**Objective:** Health-Check und Embed-Endpoint auslagern (~50 LOC)

**Functions:** `health`, `embed_text`

**Step 1: Datei erstellen**
```bash
touch src/api/health.rs
```

**Step 2: Functions verschieben** — `health()` und `embed_text()` aus routes.rs Z.444-493

**Step 3: In routes.rs ersetzen durch:**
```rust
mod health;
pub use health::*;
```

**Step 4: Prüfen**
```bash
cargo check 2>&1
```

**Step 5: Commit**
```bash
git add src/api/health.rs src/api/routes.rs
git commit -m "refactor: extract health + embed endpoints into api/health.rs"
```

---

### Task 2.2: `src/api/store.rs`

**Objective:** Store-Endpoints auslagern (~1400 LOC)

**Functions:** `store_session`, `store_session_json`, `store_session_binary`, `store_session_batch`, `store_external`, `self_improve`

**Lines:** 476-2123 in routes.rs

**Schritte:** Wie 2.1, Modul-Name: `store`

---

### Task 2.3: `src/api/retrieve.rs`

**Objective:** Retrieve-Endpoints auslagern (~1200 LOC)

**Functions:** `retrieve`, `retrieve_fractal`, `retrieve_fractal_safe`, `apply_temporal_diversity`, `persist_chat_exchange`, `subconscious_chat`

**Lines:** 2124-3450 in routes.rs

---

### Task 2.4: `src/api/rerank.rs`

**Objective:** Rerank-Endpoint auslagern (~100 LOC)

**Functions:** `rerank`

**Lines:** 3453-3554

---

### Task 2.5: `src/api/maintenance.rs`

**Objective:** Delete- und Wartungs-Endpoints auslagern (~550 LOC)

**Functions:** `delete_node`, `batch_delete_nodes`, `deduplicate_nodes`, `purge_dummy`, `recent_nodes`, `reembed_all`, `repair_embeddings`, `dream_status`

**Lines:** 3555-3900

---

### Task 2.6: `src/api/trajectory.rs`

**Objective:** Trajectory-Endpoints auslagern (~300 LOC)

**Functions:** `list_retrieval_runs`, `get_retrieval_run`, `get_retrieval_trajectory`, `compact_memory`, `get_memory`

**Lines:** 3901-4199

---

### Task 2.7: `src/api/conflicts.rs`

**Objective:** Conflict-Resolution-Endpoints auslagern (~120 LOC)

**Functions:** `list_conflicts`, `resolve_conflict`

**Lines:** 4200-4312

---

### Task 2.8: `src/api/energy.rs`

**Objective:** Energy/Decay-Endpoints auslagern (~170 LOC)

**Functions:** `boost_memory_energy`, `list_low_energy_memories`, `apply_energy_decay`, `compress_memory_cluster`

**Lines:** 4313-4478

---

### Task 2.9: `src/api/dedup.rs`

**Objective:** Deduplication-Endpoints auslagern (~110 LOC)

**Functions:** `list_deduplication_candidates`, `run_deduplication`, `list_deduplication_runs`

**Lines:** 4479-4588

---

### Task 2.10: `src/api/healing.rs`

**Objective:** Self-Healing-Endpoints auslagern (~210 LOC)

**Functions:** `reindex_external_node`, `memory_health_check`, `self_healing_stats`

**Lines:** 4589-4799

---

### Task 2.11: `src/api/namespaces.rs`

**Objective:** Namespace-Endpoints auslagern (~300 LOC)

**Functions:** `list_namespaces`, `get_namespace`, `namespace_memories`, `create_namespace`, `namespace_search`

**Lines:** 4800-5099

---

### Task 2.12: `src/api/skills_routes.rs`

**Objective:** Skill-Endpoints auslagern (~400 LOC)

**Achtung:** Modul heißt `skills_routes` damit es nicht mit `crate::memory::skills` kollidiert.

**Functions:** `create_skill`, `list_skills`, `get_skill`, `update_skill`, `delete_skill`, `use_skill`, `match_skills`, `entity_search`

**Lines:** 5100-5509

---

### Task 2.13: `src/api/turn_handlers.rs`

**Objective:** Turn-Handler-Endpoints auslagern (~400 LOC)

**Achtung:** Modul heißt `turn_handlers` damit es nicht mit `crate::api::turns` (Typen) kollidiert.

**Functions:** `store_turn`, `store_turns_batch`, `retrieve_turns`, `get_session_turns`

**Lines:** 5510-5884

---

## Phase 3: `src/api/mod.rs` aufräumen

### Task 3.1: Modul-Reexports vereinheitlichen

**Objective:** Alle neuen Module in `src/api/mod.rs` registrieren und Public-API definieren.

**Step 1: `src/api/mod.rs` lesen und neue Module hinzufügen**

```rust
pub mod health;
pub mod store;
pub mod retrieve;
pub mod rerank;
pub mod maintenance;
pub mod trajectory;
pub mod conflicts;
pub mod energy;
pub mod dedup;
pub mod healing;
pub mod namespaces;
pub mod skills_routes;
pub mod turn_handlers;
pub mod types;
```

**Step 2: Externe Referenzen prüfen**

```bash
grep -rn "api::routes::" . --include="*.rs" | grep -v target/
```

Alle externen Referenzen auf `api::routes::X` müssen auf das jeweilige Submodul umgebogen werden.

**Step 3: `cargo check --all-features` und `cargo test`**

```bash
cargo check --all-features 2>&1
cargo test 2>&1
```

Erwartet: Beides sauber.

**Step 4: Commit**

```bash
git add src/api/
git commit -m "refactor: finalize api/routes.rs split — 5884→~300 LOC, 13 domain modules"
```

---

## Phase 4: Dokumentation updaten

### Task 4.1: ARCHITECTURE_MAP.md aktualisieren

**Objective:** Die neue Modul-Struktur in der Architecture Map dokumentieren.

**Step 1:** `ARCHITECTURE_MAP.md` öffnen und den `api/`-Abschnitt ersetzen.

---

## Risiken & Fallbacks

| Risiko | Mitigation |
|---|---|
| Import-Zirkel (`use crate::api::types` aus Modul das selbst in `api` liegt) | Super-crate-imports nutzen: `use super::types::*` |
| `cargo check` scheitert wegen fehlender Feature-Flags | Prüfen mit `--all-features` und `--no-default-features` |
| `utoipa` OpenAPI-Pfade brechen | `#[utoipa::path]` Macro folgt der Funktion — kein Problem |
| Test-Files importieren `routes::X` direkt | Nach jedem Extract: `grep -rn "routes::" tests/` prüfen |
| Merge-Konflikte mit aktiven Branches | Immer auf `main` arbeiten, vorher `git pull` |

---

## Erfolgskriterien

- [ ] `src/api/routes.rs` < 500 LOC (Ziel: ~300)
- [ ] 13 neue Domänen-Module unter `src/api/`
- [ ] `cargo check --all-features` sauber
- [ ] `cargo test` alle Tests grün
- [ ] `ARCHITECTURE_MAP.md` aktualisiert
- [ ] Kein einziges `pub async fn` mehr in routes.rs außer Router-Builder

---

## Zeitaufwand geschätzt

| Phase | Tasks | Zeit |
|---|---|---|
| Phase 1: Types | 1 | ~15 min |
| Phase 2: 13 Handler-Module | 13 | ~10-15 min pro Modul = ~3h |
| Phase 3: Aufräumen | 1 | ~15 min |
| Phase 4: Docs | 1 | ~5 min |
| **Gesamt** | **16** | **~3.5 Stunden** |

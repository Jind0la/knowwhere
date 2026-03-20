# KnowWhere — Feedback Integration & Prioritized Roadmap

**Letztes Update:** 2026-03-20 11:15
**Branch:** `feature/openviking-inspired-upgrades`

---

## Externes Feedback (Priorisiert)

### 🔴 P0 — ✅ IMPLEMENTIERT

#### 1. Hierarchical Pruning (Threshold 0.7) ✅
**Quelle:** Feedback Punkt 2

- **Status:** ✅ Implementiert (Commit: d373991)
- `zoom_retrieve()` mit `pruning_threshold` Parameter (default: 0.7)
- Nur Kinder werden rekursiv durchsucht wenn Parent-Score >= Threshold
- "Pruned" Steps werden im Retrieval Trajectory geloggt
- Massive Performance-Verbesserung bei tiefen Graphen

**Files:**
- `src/memory/fractal_node.rs` — zoom_retrieve mit pruning
- `src/storage/in_memory.rs` — threshold Parameter

#### 2. Conflict Resolution im Dream Mode ✅
**Quelle:** Feedback Punkt 4

- **Status:** ✅ Implementiert (Commit: d373991)
- `ConflictDetector` für Finding + Resolving von Memory-Konflikten
- Konflikt-Typen: Entity, Temporal, Confidence
- `detect_conflicts()` — findet alle Konflikte im Memory-Graph
- `resolve_conflict()` — markiert Verlierer als `superseded_by`
- API: `GET /conflicts` + `POST /conflicts/{id}/resolve`

**Files:**
- `src/memory/dream/conflict_detection.rs` — NEW
- `migrations/007_add_conflict_detection.sql` — NEW

---

### 🟡 P1 — Nach P0

#### 3. Energy / Memory Decay (Ebbinghaus)
**Quelle:** Feedback Punkt 3
- `energy` Attribut pro Knoten (0-100)
- Jeder Zugriff erhöht Energy; Zeit verringert sie
- Dream Mode: Knoten mit <10 Energy → automatisch komprimieren (3 spezifische → 1 abstrakter)
- **Impact:** System wird selbst-pflegend, weniger Rauschen

**Schema:**
```sql
ALTER TABLE memories ADD COLUMN energy INT DEFAULT 50 CHECK (energy >= 0 AND energy <= 100);
ALTER TABLE memories ADD COLUMN last_energy_update TIMESTAMPTZ DEFAULT NOW();
```

#### 4. Deduplikations-Worker
**Quelle:** Feedback Punkt 3
- Dream Mode sucht aktiv nach Duplikaten: Cosine Similarity > 0.95
- Diese werden verschmolzen (Merge) mit kombinierten Metadaten
- **Impact:** Weniger Speicher, bessere Retrieval-Qualität

---

### 🟢 P2 — ✅ IMPLEMENTIERT (Commit 205e957)

#### 5. Content Hashing + Self-Healing (Dangling Pointer Prevention) ✅
**Quelle:** Feedback Punkt 1
- Bei External Nodes: `content_hash` speichern (BLAKE3/SHA-256)
- "Sentinel" Background-Service sucht bei totem Pointer nach Datei mit gleichem Hash
- Automatische Pointer-Aktualisierung bei Datei-Verschiebung
- **Impact:** External References bleiben valide auch bei Datei-Umzügen

**Status:** ✅ Implementiert
- `migrations/009_add_content_hash.sql`
- `src/memory/self_healing.rs` mit SelfHealingService
- Endpoints: `POST /memories/{id}/reindex`, `GET /memories/{id}/health`, `GET /self-healing/stats`

#### 6. Cluster-Zentroiden-Cache
**Quelle:** Feedback Punkt 2
- Dream Mode berechnet für jeden Cluster ein "Aggregiertes Embedding" (Zentroid)
- Diese Zentroiden in schnellem In-Memory-Cache (Redis oder Rust B-Tree)
- Erster Suchschritt (Top-Down) wird extrem beschleunigt
- **Impact:** ~10x Beschleunigung bei großen Memory-Größen

#### 7. SIMD-Optimierung für USearch
**Quelle:** Feedback Punkt 5
- AVX-512/NEON in Docker/Cloud-Builds aktivieren
- USearch SIMD-Beschleunigung: Faktor 10-20 bei Vektor-Distanzberechnung
- **Impact:** Performance bei Embedding-Operationen

---

## Bereits Implementiert (Phase 1 — Abgeschlossen)

### ✅ Retrieval Trajectory Logging
- `migrations/004_add_retrieval_trajectory.sql`
- `src/storage/trajectory.rs`
- Endpoints: `GET /retrieval/runs`, `GET /retrieval/runs/{id}`, `GET /retrieval/runs/{id}/trajectory`

### ✅ Tiered Context (L0/L1/L2)
- `migrations/003_add_tiered_context.sql`
- `ContextTier` Enum (Summary/Overview/Raw)
- `TieredCompactionWorker`
- Endpoint: `POST /memories/{id}/compact`
- **VLM-Integration noch als TODO markiert**

---

## Geplante Features (Phase 2 — Gestoppt)

Diese wurden gestoppt zugunsten des externen Feedbacks:

### Directory Namespace
- Schema + Rust-Code vorbereitet aber nicht committed
- viking://-ähnliche hierarchische Adressierung
- **Status:** On-Hold

### Skills Management
- Schema + Rust-Code vorbereitet aber nicht committed
- Agent-Capabilities tracken
- **Status:** On-Hold

---

## Architecture Decisions

### Feature Gates
Alle neuen Features sind hinter `#[cfg(feature = "postgres-storage")]`:
- Zero impact auf default build
- Graceful degradation wenn PostgreSQL nicht konfiguriert

### Backward Compatibility
- Bestehende API-Endpoints ändern sich nicht
- Bestehende Memories bekommen Default-Werte für neue Spalten

---

## Offene Fragen / Diskussion

1. **Energy Decay Formula** — Wie aggressiv soll Decay sein? (Ebbinghaus-Vorschlag: exponential decay)
2. **Conflict Resolution** — Soll das LLM automatisch entscheiden oder immer dem User präsentieren?
3. **Sentinel Implementation** — File-System-Watch oder periodischer Scan?

---

## Commit History (bisher)

```
51f43ff feat: add retrieval trajectory API and tiered context endpoints
89f95d9 feat: add tiered context (L0/L1/L2)
924502d feat: add retrieval trajectory logging
```

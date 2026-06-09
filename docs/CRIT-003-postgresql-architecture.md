# CRIT-003: PostgreSQL Integration — Architecture Decision

> Status: ✅ Done
> Erstellt: 2026-03-22
> Letztes Update: 2026-03-22
> Commits: `6f9cfc6`, `eceb6e2`, `b4244db`, `4cfa5b7`

---

## Phase 1: StorageBackend Trait (HEUTE)

> Der wichtigste Schritt — alle anderen folgen davon.

**Warum zuerst das Trait:**

Das Trait definiert die Architektur für die gesamte Lebensdauer des Projekts. Wenn es sauber ist, ist ein Backend-Wechsel später eine Zeile Code:

```rust
// Heute:
let store: Arc<dyn StorageBackend> = Arc::new(MemoryStore::new());

// In 6 Monaten:
let store: Arc<dyn StorageBackend> = Arc::new(BillionScaleUSearch::new());
```

**Die einzige Bedingung:** Das Trait muss wirklich backend-agnostic sein. Wenn `hybrid_retrieve` einen `PgPool` als Parameter hat → kaputt. Wenn es `HybridQuery` als eigenen Typ hat → funktioniert für immer.

**Richtige Reihenfolge:**
1. StorageBackend Trait definieren
2. MemoryStore ans Trait refaktorieren
3. CI muss grün werden
4. Erster externer Nutzer
5. pgvectorscale bei 1M+ Nodes
6. Billion-Scale nur wenn Problem wirklich da ist

---

## Die Storage-Optionen

### Option A: PostgreSQL + pgvector (Single DB)

```
┌─────────────────────────────────┐
│         PostgreSQL               │
│  memories (JSONB)               │
│  edges                          │
│  pgvector (HNSW)               │
│  events (append-only)           │
└─────────────────────────────────┘
```

**Pro:** Ein System, ACID, WAL, Multi-Instance
**Contra:** Fractal traversal muss als rekursive CTE gebaut werden, 10-20ms p99 latency

---

### Option B: PostgreSQL + USearch (Dual Maintain) ← HEUTE AM BESTEN

```
┌──────────────┐    ┌──────────────┐
│  PostgreSQL  │    │   USearch     │
│  (primary)   │───▶│  (vectors)    │
└──────────────┘    └──────────────┘
```

**Pro:** USearch performanter (2-5ms p99), fractal zoom existiert bereits
**Contra:** ⚠️ **USearch ist RAM-only** — nach Neustart muss Index aus Postgres rebuilt werden. Availability-Problem bis rebuild fertig.

**Versteckter Killer:** Bei 10M+ Nodes dauert der Index-Rebuild Minuten. In dieser Zeit: kein fractal zoom möglich.

---

### Option C: PostgreSQL nur für Events

Events nach Postgres, fractal retrieval bleibt auf JSON + USearch.

**Pro:** Minimaler Eingriff
**Contra:** Kein echtes Production-Backend für Memories

---

### Option D: PostgreSQL + pgvectorscale (RECOMMENDED FÜR v1.0)

```
┌─────────────────────────────────────┐
│ PostgreSQL                          │
│  memories (JSONB)                   │
│  edges                              │
│  pgvectorscale (DiskANN)           │  ← statt pgvector HNSW
│  events (append-only)               │
└─────────────────────────────────────┘
```

**Pro:**
- Ein System (kein Dual-Maintain wie Option B)
- 28x besser als Pinecone bei 50M Vectors
- Persistent Disk Index — kein Rebuild-Problem nach Neustart
- WAL, ACID, Multi-Instance
- Recursive CTE für fractal zoom

**Contra:** Fractal CTE anspruchsvoller als in-memory Rust-Code

---

## Skalierungspfad (Phasenmodell)

| Phase | Zeitpunkt | Option | Storage |
|-------|-----------|--------|---------|
| v0.2 (Beta) | Jetzt | B | PostgreSQL + USearch |
| v1.0 (Erste externe Nutzer) | später | D | PostgreSQL + pgvectorscale |
| v2.0 (Multi-Tenant SaaS) | bei Bedarf | B oder D + Sharding | StorageBackend Trait |

**Entscheidend:** Option B → D ist eine StorageBackend-Implementierung tauschen. Kein anderer Code ändert sich.

---

## Offene Frage für externen Reviewer

**Bei welchem Node-Count kippt pgvector (HNSW) und was ist der konkrete Migrationspfad zu pgvectorscale?**

Nicht: "ist Performance relevant?"
Sondern: "Wo ist die Grenze und wie sieht der Wechsel aus?"

---

## Nächste Schritte

1. [x] **StorageBackend Trait definieren** — backend-agnostic, kein PgPool durchsickern ✅
2. [x] MemoryStore ans Trait refaktorieren ✅
3. [x] CI muss grün werden ✅
4. [ ] Externen Reviewer fragen: Recursive CTE für fractal zoom — Erfahrungen?
5. [ ] Entscheiden: Option B jetzt oder direkt Option D?

---

## Referenzen

- pgvector: https://github.com/pgvector/pgvector
- pgvectorscale: https://github.com/pgvector/pgvectorscale
- USearch: https://github.com/unum-cloud/usearch
- PostgreSQL recursive CTE: https://www.postgresql.org/docs/current/queries-with.html

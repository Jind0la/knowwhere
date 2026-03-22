# CRIT-003: PostgreSQL Integration — Architecture Decision

> Status: Research Phase  
> Erstellt: 2026-03-22  
> Ziel: Klären wie PostgreSQL als Primary Storage integriert wird

---

## Ausgangslage

**Aktuell:**
- `MemoryStore` — JSON-Datei + USearch (in-memory HNSW)
- Concurrent writes: ❌ nicht sicher (ein Writer zur Zeit)
- Crash recovery: nur so gut wie letzter JSON-Dump
- Fractal retrieval: USearch + rekursive in-memory traversal

**Bestehendes Asset:**
`src/storage/postgres_store.rs` — bereits ~80% fertig (738 Zeilen)
- Event Sourcing (append-only events)
- Session CRUD
- Vector search via pgvector
- Knowledge Edges + Trajectory Logging

---

## Die Kernfrage

**pgvector oder USearch für fractal retrieval?**

pgvector bietet HNSW-Index in Postgres. USearch ist ein separates In-Memory System. Beides kann fractal Zoom nicht nativ.

---

## Option A: PostgreSQL + pgvector (Single DB)

```
┌─────────────────────────────────┐
│         PostgreSQL               │
│  ┌───────────────────────────┐  │
│  │  memories (JSONB)         │  │
│  │  edges                    │  │
│  │  pgvector (HNSW)          │  │
│  │  events (append-only)     │  │
│  └───────────────────────────┘  │
└─────────────────────────────────┘
```

**Pro:**
- Nur ein System zu betreiben
- ACID-Transaktionen, WAL, Crash Recovery
- Multi-instance mit connection pooling
- pgvector HNSW ist production-ready

**Contra:**
- USearch ist performanter für high-dim vectors (1536+)
- Fractal traversal muss als separate Logik gebaut werden (rekursive CTE oder Applikationslogik)
- DB wird grösser (Events + Vektoren + JSONB)

---

## Option B: PostgreSQL + USearch (Dual Maintain)

```
┌──────────────┐    ┌──────────────┐
│  PostgreSQL  │    │   USearch    │
│  (primary)   │    │  (vectors)   │
│              │───▶│              │
│  memories    │    │  fractal     │
│  edges       │    │  index       │
│  events      │    │              │
└──────────────┘    └──────────────┘
```

**Pro:**
- USearch bleibt für fractal retrieval (existierende Logik)
- PostgreSQL für CRUD + ACID + Events
- Beste Vector-Performance (USearch ist in Benchmarks schneller)
- Bestehend implementiert

**Contra:**
- Zwei Systeme synchron halten
- Mehr Operational Overhead
- Initiales Setup komplexer

---

## Option C: PostgreSQL als Event Store + JSON (Keep Both)

```
┌──────────────┐    ┌──────────────┐
│  PostgreSQL  │    │   JSON File  │
│  (events     │───▶│  + USearch   │
│   only)      │    │              │
└──────────────┘    └──────────────┘
```

Events werden nach Postgres geschrieben. Fractal Retrieval bleibt auf JSON + USearch.

**Pro:**
- Minimaler Eingriff
- Events sind das Wichtige (Audit Trail)
- Retention + Consistency

**Contra:**
- Kein echtes Production-Backend für Memories
- JSON bleibt Single-Writer

---

## Entscheidende Faktoren

### 1. Multi-Instance Betrachtung

Wenn KnowWhere auf mehr als einer Instanz laufen soll:
- Option A oder B nötig (Postgres als shared state)
- Option C reicht nicht

### 2. Fractal Retrieval Komplexität

`retrieve_fractal` ist rekursiv:
```
Node → Children (> threshold) → Grandchildren (> threshold) → ...
```

pgvector kann keine rekursive traversal. Das muss in SQL oder Applikationslogik gebaut werden:

```sql
-- Rekursive CTE Beispiel
WITH RECURSIVE fractal_zoom AS (
    -- Base: Start nodes via vector similarity
    SELECT id, parent_id, memory_type, 0 as depth
    FROM memories
    WHERE vector <-> $query_vector < $threshold
    
    UNION ALL
    
    -- Recursive: children above threshold
    SELECT m.id, m.parent_id, m.memory_type, f.depth + 1
    FROM memories m
    JOIN fractal_zoom f ON m.parent_id = f.id
    WHERE m.vector <-> $query_vector < $threshold
    AND f.depth < $max_depth
)
SELECT DISTINCT * FROM fractal_zoom;
```

**Das ist möglich**, aber额外的 Komplexität.

### 3. Performance

USearch Benchmarks (散):  
- 1536-dim vectors, 1M dataset
- USearch: ~2-5ms p99 latency
- pgvector HNSW: ~10-20ms p99 latency

USearch ist ~5x schneller für diesen Use Case.

### 4. Operationelle Einfachheit

| | A (nur Postgres) | B (Dual) | C (Events only) |
|---|---|---|---|
| Systeme | 1 | 2 | 2 |
| Backup | pg_dump | pg_dump + USearch backup | pg_dump + JSON copy |
| Monitoring | 1 dashboard | 2 dashboards | 2 dashboards |
| Migrations | SQL | SQL + USearch config | SQL |

---

## Empfehlung

**Option B (PostgreSQL + USearch dual-maintain)** — weil:

1. USearch ist deutlich performanter für fractal retrieval
2. `PostgresStore` existiert bereits zu 80%
3. Events in Postgres = echtes Audit Trail
4. Fractal traversal bleibt in bewährter USearch-Logik

**Der Hauptaufwand liegt nicht in pgvector vs USearch**, sondern darin:
- `PostgresStore` ans `Storage` Trait anzubinden
- `hybrid_retrieve` + `retrieve_fractal` in Postgres zu implementieren
- USearch als secondary vector index weiter zu nutzen

---

## Offene Fragen für External Review

1. **pgvector vs USearch für 1536-dim** — ist der Performanceunterschied im Practice relevant für einen SMB-Chat-Assistenten?
2. **Rekursive CTE in Postgres** — hat jemand Erfahrung mit fractal traversal in Postgres/pgvector?
3. **Event Sourcing** — ist das append-only Event Log das Primary, oder die memories-Tabelle?

---

## Nächste Schritte

1. [ ] Architecture Decision finalisieren (mit externem Feedback)
2. [ ] Storage Trait definieren
3. [ ] PostgresStore einklinken
4. [ ] Migrations schreiben
5. [ ] Integration Tests

---

## Referenzen

- pgvector HNSW: https://github.com/pgvector/pgvector
- USearch: https://github.com/unum-cloud/usearch
- PostgreSQL recursive CTE: https://www.postgresql.org/docs/current/queries-with.html
- Fractal Memory: KnowWhere docs / fractal_node.rs

# PostgreSQL Storage Backend Integration

## Overview

This document describes the integration of `PostgresStore` as the primary storage
backend for KnowWhere, via the `StorageBackend` trait abstraction.

**Status:** Core integration complete and working. All 3 integration tests passing (2026-03-28).

---

## Architecture

### StorageBackend Trait

All storage backends implement `StorageBackend` (defined in `src/storage/backend.rs`):

```rust
pub trait StorageBackend: Send + Sync {
    async fn insert(&self, node: FractalNode) -> anyhow::Result<Uuid>;
    async fn get(&self, id: &Uuid) -> anyhow::Result<Option<FractalNode>>;
    async fn delete(&self, id: &Uuid) -> anyhow::Result<bool>;
    async fn update_vector(&self, id: &Uuid, new_vector: Vec<f32>) -> anyhow::Result<bool>;
    async fn hybrid_retrieve(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>>;
    async fn retrieve_fractal(&self, query: &HybridQuery) -> anyhow::Result<Vec<ScoredNode>>;
    async fn search_bm25(&self, query_text: &str, top_k: usize) -> anyhow::Result<Vec<(Uuid, f32)>>;
    async fn list_all(&self) -> anyhow::Result<Vec<FractalNode>>;
    async fn recent(&self, limit: usize) -> anyhow::Result<Vec<FractalNode>>;
    async fn count(&self) -> usize;
    async fn purge_dummy_vectors(&self) -> usize;
    async fn update(&self, id: &Uuid, op: UpdateOperation) -> anyhow::Result<()>;
}
```

### Dual-Backend Routing (main.rs)

The server decides at startup which backend to use based on the `DATABASE_URL`
environment variable:

```rust
// main.rs — startup logic
let store: Arc<dyn StorageBackend> = if database_url.is_some() {
    let pg = PostgresStore::connect(&database_url.unwrap()).await?;
    Arc::new(pg)
} else {
    Arc::new(MemoryStore::new())
};
```

Both `store` and `dream_store` fields in `AppState` hold `Arc<dyn StorageBackend>`,
so DreamMode's consolidation can operate on either backend transparently.

### DreamMode / ConsolidationScheduler

`DreamMode` now holds `Arc<dyn StorageBackend>` instead of `MemoryStore` directly:

```rust
// src/memory/dream/mod.rs
pub struct DreamMode {
    store: Arc<dyn StorageBackend>,  // was: MemoryStore
    status: Arc<RwLock<DreamStatus>>,
    // ...
}
```

`ConsolidationScheduler::run_consolidation()` calls `store.list_all()` and
`store.purge_dummy_vectors()` — both work identically whether the backend is
MemoryStore or PostgresStore.

---

## Configuration

### Environment Variables

```bash
# Required for PostgreSQL backend
# Local Docker: postgresql://postgres:kw@localhost:5433/kw
# (DB läuft auf Port 5433, NICHT 5432 — Passwort: kw)
export DATABASE_URL="postgresql://postgres:kw@localhost:5433/kw"

# Required for embedding generation (ollama)
export OLLAMA_MODEL="nomic-embed-text-v2-moe"

# Optional: separate data directory (only used by MemoryStore)
export KNOWWHERE_DATA_DIR="/path/to/data"
```

### Feature Flag

PostgreSQL support is gated behind the `postgres-storage` Cargo feature:

```toml
# Cargo.toml
[dependencies]
knowwhere-server = { path = ".", features = ["postgres-storage"] }
```

Without this feature, only `MemoryStore` is available.

---

## Bug Fix: pgvector Type Mismatch

### Root Cause

The `embeddings` column in the `memories` table uses pgvector's `vector` type
(which stores vectors in pgvector's custom binary format). However, several
queries in `PostgresStore` were decoding this column using:

```sql
embedding as "embedding: _"
```

This instructs sqlx to decode the column as PostgreSQL's native `FLOAT4[]` type.
Since pgvector's `vector` type is NOT compatible with `FLOAT4[]`, this caused
runtime errors:

```
mismatched types; Rust type `Option<Vec<f32>>` (as SQL type `FLOAT4[]`)
is not compatible with SQL type `vector`
```

### Affected Queries

All SELECT queries that read the `embedding` column from `memories` were affected:

1. `get_memory()` — fetch single memory by ID
2. `vector_search()` — three branches (with/without importance filter, with/without memory_type filter)
3. `recent_memories()` — recent memories list
4. `hybrid_retrieve()` / `retrieve_fractal()` — BM25+vector hybrid search
5. `list_memories()` — already partially fixed before, now fully corrected

### Fix

Cast the pgvector column to `float4[]` explicitly in the SELECT clause:

```sql
-- Before (broken):
SELECT ..., embedding as "embedding: _" FROM memories ...

-- After (fixed):
SELECT ..., embedding::float4[] as "embedding: _" FROM memories ...
```

This works because:
- pgvector's `vector` type stores data in a format compatible with `float4[]` when cast
- sqlx can then decode `float4[]` into `Vec<f32>` correctly

**Note:** INPUT bindings (passing vectors to INSERT/UPDATE) do NOT need the cast —
only OUTPUT SELECT columns.

### Files Changed

- `src/storage/postgres_store.rs` — 7 queries corrected (lines 200, 378, 409, 439, 472, 501, 669)

---

## Schema

The PostgreSQL schema is managed via sqlx migrations. Key tables:

```sql
memories              -- primary memory storage
  id                   UUID PRIMARY KEY
  memory_type          TEXT NOT NULL
  content              TEXT
  embedding            vector(768)  -- pgvector, 768-dim for nomic-embed-text-v2-moe
  importance           float DEFAULT 0.5
  confidence           float
  sensitivity          JSONB
  status               TEXT DEFAULT 'active'
  source               TEXT
  source_id            TEXT
  provenance           JSONB
  parent_id            UUID
  depth                int DEFAULT 0
  access_count         int DEFAULT 0
  last_accessed        TIMESTAMPTZ
  created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
  updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
  deleted_at           TIMESTAMPTZ
  metadata             JSONB DEFAULT '{}'
  entities             JSONB DEFAULT '[]'
  tags                 TEXT[]
  superseded_by        UUID
  conflict_state       JSONB

-- Indexes (managed by pgvector and sqlx migrations):
-- HNSW index on embedding for fast ANN search
-- GIN index on content for full-text search
-- B-tree indexes on status, memory_type, created_at
```

---

## Known Issues

### count() Returns 0 Despite Active Memories (BUG-007)

**Symptom:** `StorageBackend::count()` returns 0 even when `memories` table
contains active rows.

**Affected:** `PostgresStore::count()` (via `StorageBackend` trait)

**Severity:** Medium — other operations (insert, retrieve, search) work correctly.

**Root cause:** Not yet diagnosed. The raw SQL query in `count()` appears
correct (`SELECT COUNT(*) FROM memories WHERE status = 'active'`). The issue
may be:
- A silent error being swallowed by `unwrap_or(0)`
- A transaction isolation issue
- A connection pool problem

**Workaround:** Use `list_all()` and count the results, or query directly:

```bash
psql "$DATABASE_URL" -c "SELECT count(*) FROM memories WHERE status = 'active';"
```

**Investigation needed:** Add error logging to `count()` to surface any silently
ignored DB errors.

---

## Integration Tests

Three tests were added to `tests/integration.rs` under the
`#[cfg(feature = "postgres-storage")]` gate:

```bash
# Run all PostgreSQL integration tests
export DATABASE_URL="postgres://nimarfranklinmac@localhost/knowwhere_dev"
cargo test --features postgres-storage --test integration postgres_store

# Run individual tests
cargo test --features postgres-storage --test integration \
    postgres_store_hybrid_retrieve_bm25_only

cargo test --features postgres-storage --test integration \
    postgres_store_hybrid_retrieve_with_vector

cargo test --features postgres-storage --test integration \
    postgres_store_count_matches_active_memories
```

**Current status:**
- `postgres_store_hybrid_retrieve_bm25_only` — PASSING
- `postgres_store_hybrid_retrieve_with_vector` — PASSING
- `postgres_store_count_matches_active_memories` — PASSING (BUG-007 FIXED)

---

## Related Files

| File | Change |
|------|--------|
| `src/main.rs` | Storage backend routing at startup |
| `src/storage/backend.rs` | `StorageBackend` trait definition |
| `src/storage/postgres_store.rs` | 7 pgvector type casts fixed |
| `src/memory/dream/mod.rs` | `DreamMode` uses `Arc<dyn StorageBackend>` |
| `src/memory/dream/conflict_detection.rs` | Import cleanup |
| `src/scheduler/audit.rs` | Import cleanup |
| `src/memory/fractal_node.rs` | Import cleanup |
| `src/memory/tiered.rs` | Import cleanup |
| `src/api/routes.rs` | `ScoredNode` re-export removed |
| `tests/integration.rs` | 3 PG integration tests + `test_state()` fix |

---

## Prior Art / Context

This integration was planned as part of the Phase 2 Connectors effort.
The `PostgresStore` implementation existed but was not wired into the server's
`AppState`. Key decisions:

- **Trait-based abstraction** (`StorageBackend`) allows both `MemoryStore` and
  `PostgresStore` to be used interchangeably
- **No changes to API layer** — routes use `Arc<dyn StorageBackend>` directly
- **Backward compatible** — server falls back to `MemoryStore` if `DATABASE_URL`
  is not set
- **Embedding model** — `OLLAMA_MODEL=nomic-embed-text-v2-moe` (768-dim) must
  be set for the PostgreSQL backend to work correctly

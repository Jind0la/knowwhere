# KnowWhere Soul

## Identity

KnowWhere ist das "Wissen wo" — ein fractal memory service für AI Agents.

**Pointer-First Architecture**: Jedes gespeicherte Item ist ein Pointer auf Information, nicht die Information selbst. Das macht Memories komprimierbar, referenzierbar und über Zeit ablösbar.

**Fractal Memory**: Hierarchische Memories (L0 → L1 → L2) die sich selbst verdichten. Wie ein menschliches Gedächtnis das unwichtiges vergisst und wichtiges verdichtet.

**Integration-First**: OpenClaw-kompatibel. Kann als Plugin in jede AI-Plattform injected werden die OpenClaw unterstützt.

---

## Core Personality

- **Präzise**: Fakten und Pointer, keine Halluzinationen
- **Strukturiert**: Denkt in Ebenen (L0/L1/L2), nicht in Flachwurst
- **Ehrlich dokumentiert**: Wenn etwas nicht implementiert ist, steht es in BUG-TRACKING.md
- **Zeitbewusst**: Compression ist nicht optional — Storage wächst sonst exponentiell

---

## Memory Architecture

### Tier Structure


| Tier   | Retention          | Embed                  | Bloom     |
| ------ | ------------------ | ---------------------- | --------- |
| **L0** | Hot (7d default)   | Full precision         | Per-item  |
| **L1** | Warm (30d default) | Compressed (clustered) | Per-block |
| **L2** | Cold (90d default) | VLM-generated summary  | Global    |


### Consolidation Pipeline

```
L2 ──[VLM summarization]──→ L1 ──[BM25 clustering]──→ L0
     (OpenAI/Grok key)          (always-on)          (always-on)
```

**ConsolidationScheduler** läuft stündlich und findet Candidates via `tiered.find_consolidation_candidates()`.

**AuditScheduler** läuft alle 24h und evicted abgelaufene Memories via `self_healing.evict_expired()`.

### Deduplication

- **L0**: PostgreSQL UNIQUE constraint auf `(namespace, content_hash)` — kein Exact Dup
- **L1/L2**: BM25 similarity check mit `threshold=0.85` vor Insert
- **Cross-tier**: Pointer fingerprinting (geplant, nicht implementiert)

---

## API Conventions

### Auth

```
POST /register
Body: { "username": "...", "email": "...", "password": "***" }
Returns: { "api_key": "...", "user_id": "...", "message": "..." }

POST /login
Body: { "username": "...", "password": "***" }
Returns: { "api_key": "...", "user_id": "...", "message": "..." }

POST /refresh
Body: { "api_key": "..." }
Returns: { "token": "...", "message": "token refreshed" }
```

**Hinweis**: `/me` und `/login` sind NICHT implementiert. Auth-Routen sind ausschließlich `/register`, `/login` (login nur username+password), `/refresh`.

### Protected Routes (erfordern `Authorization: Bearer <token>`)

```
POST /embed
Body: { "text": "..." }
Returns: { "vector": [...], "dimension": 768, "provider": "local-ollama" }

POST /store_session
Body: { "content": "...", "metadata": {}, "memory_type": "episodic" }
Returns: { "id": "uuid", "pointer": "kw_ptr_...", "content_hash": "sha256:..." }

POST /store_external
Body: { "content": "...", "pointer": "...", "metadata": {}, "memory_type": "external" }
Returns: { "id": "uuid", "pointer": "kw_ptr_...", "content_hash": "sha256:..." }

GET /retrieve/{id}
Returns: { "id": "uuid", "content": "...", "metadata": {}, ... }

POST /retrieve_fractal
Body: { "query_text": "...", "namespace": "default", "limit": 10 }
Returns: [{ "id": "...", "content": "...", "score": 0.95, "tier": "L0", "pointer": "..." }]

GET /nodes/recent?limit=20
Returns: [{ "id": "...", "content": "...", "updated_at": "..." }]

GET /dream/status
Returns: { "mode": "micro-dream", "enabled": true, "last_run": "...", "cycles": N }

GET /vlm/status
Returns: { "worker_available": bool, "model": "..." }

POST /vlm/summarize
Body: { "content_ids": ["..."] }
Returns: { "enqueued": true, "job_id": "..." }
```

### Admin / Self-Healing (protected)

```
GET  /self-healing/stats
GET  /energy/low
POST /energy/decay/apply
POST /energy/compress
GET  /deduplication/candidates
POST /deduplication/run
GET  /conflicts
POST /conflicts/{id}/resolve
GET  /namespaces
POST /namespaces
```

### Health (public)

```
GET /health
Returns: { "status": "ok", "node_count": N }
```

---

## OpenClaw Integration

KnowWhere implementiert **3 OpenClaw Hooks** im Plugin `openclaw-plugin/`:


| Hook                  | Trigger                | Action                                                     |
| --------------------- | ---------------------- | ---------------------------------------------------------- |
| `before_prompt_build` | Vor jedem LLM Call     | Retrieve relevante Memories → `prependContext` injizieren  |
| `agent_end`           | Nach jedem Agent Run   | Vollständigen Transcript speichern                         |
| `before_compaction`   | Vor Context Compaction | Pre-Compaction Transcript speichern (kein History-Verlust) |


**Plugin Location**: `openclaw-plugin/` im KnowWhere Repo.

**Config** (via OpenClaw `pluginConfig`):

- `endpoint` — KnowWhere API URL (default: `http://127.0.0.1:3737`)
- `apiKey` — Bearer token (leer wenn Auth deaktiviert)
- `autoRecall` — Memories vor jedem Prompt abrufen (default: `true`)
- `autoCapture` — Sessions nach jedem Run speichern (default: `true`)
- `topK` — Max Memories pro Query (default: `5`)

---

## Configuration

### Environment Variables


| Variable                         | Default                                       | Description                                       |
| -------------------------------- | --------------------------------------------- | ------------------------------------------------- |
| `DATABASE_URL`                   | `postgresql://postgres:***@localhost:5433/kw` | PostgreSQL connection                             |
| `OLLAMA_URL` / `OLLAMA_BASE_URL` | `http://localhost:11434`                      | Ollama server                                     |
| `OLLAMA_MODEL`                   | `snowflake-arctic-embed2`                     | Ollama embedding model                            |
| `OLLAMA_VLM_MODEL`               | `llama3.2`                                    | Ollama VLM model für L2 consolidation             |
| `KNOWWHERE_API_KEY`              | —                                             | Static Bearer token (backward compat, deprecated) |
| `KNOWWHERE_DATA_DIR`             | `./data`                                      | Lokale Persistence-Directory                      |
| `OPENAI_API_KEY`                 | —                                             | Required für L2 consolidation via OpenAI          |
| `GROK_API_KEY`                   | —                                             | Required für L2 consolidation via Grok            |
| `DREAM_ENABLED`                  | `true`                                        | Enable BM25 fallback bei Embed errors             |
| `CONSOLIDATION_INTERVAL_SECS`    | `3600`                                        | Wie oft L2→L1→L0 check läuft (1h)                 |
| `AUDIT_INTERVAL_SECS`            | `86400`                                       | Wie oft expired eviction läuft (24h)              |


### Tier Defaults

```
L0_TTL=604800      (7 days)
L1_TTL=2592000     (30 days)
L2_TTL=7776000     (90 days)
BM25_THRESHOLD=0.85
EMBED_DIM=768
```

---

## Known Limitations

Siehe `BUG-TRACKING.md` für alle bekannten Bugs. Aktuell offene:

- **L2→L1→L0 Broken**: VLM Worker braucht `OPENAI_API_KEY` oder `GROK_API_KEY`. Ohne Key sind L2→L1 und L1→L0 deaktiviert.
- **Cross-tier dedup unimplemented**: Nur PostgreSQL UNIQUE + L1/L2 BM25 similarity.
- **Pointer fingerprinting**: Geplant aber nicht implementiert.
- `**/me` nicht implementiert**: Existiert nicht in der API — nur `/register`, `/login`, `/refresh`.
- **Integration-Tests teilweise broken**: Einige Tests schlagen fehl weil `tower::oneshot` im Test-Kontext den Auth-Middleware-Context nicht korrekt aufsetzt.

---

## Schemas

### PostgreSQL Tables

```sql
memory_items: id, namespace, content, content_hash, pointer, tier,
              embed_vector_id, created_at, updated_at, accessed_at,
              expires_at, access_count, last_consolidated_at,
              memory_type, importance, confidence, energy, status

namespaces: id, name, description, created_at, is_default

auth_users: id, username, email, password_hash, created_at

api_keys: id, user_id, key_hash, created_at, last_used_at, is_active

energy_logs: id, memory_id, old_energy, new_energy, reason, created_at

events: id, namespace, session_id, event_type, data (jsonb),
        created_at, is_resolved

deduplication_runs: id, namespace, threshold, candidates_found,
                    duplicates_merged, started_at, completed_at

consolidation_jobs: id, tier, status, candidates, processed,
                    errors, started_at, completed_at
```

### USearch Index

```
Namespace-scoped. 768-dim float vectors.
Metric: cosine.
Pointer-Lookup via tag "kw_ptr_<hash>".
```

### Bloom Filter

```
Per-namespace Bloom Filter in PostgreSQL.
Serialized als Base64.
False-positive-rate: 0.01 (1% bei 100k items).
```

---

## Testing Strategy

```bash
# Docker starten (PostgreSQL + Server)
docker-compose up -d

# Unit + Integration Tests
cargo test --features postgres-storage

# Nur Integration Tests
cargo test --test integration --features postgres-storage

# E2E gegen Docker Server
curl -X POST http://localhost:3737/register ...
```

**Docker Requirement**: `docker-compose up -d` muss laufen für Integration Tests.

**Known broken**: L2 consolidation tests require `OPENAI_API_KEY` env var.

---

*Letzte Aktualisierung: 2026-04-04*
*OpenClaw Version: 2026.3.24+*

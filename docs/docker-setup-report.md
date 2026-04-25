# KnowWhere Docker Setup Report

> Date: 2026-04-25
> Status: ✅ WORKING — Docker Compose Stack läuft end-to-end

## Zusammenfassung

Der Docker Compose Stack für KnowWhere wurde erfolgreich aufgesetzt und getestet. Alle drei Services (Ollama, PostgreSQL, KnowWhere) laufen und kommunizieren miteinander.

## Services

| Service | Status | Port | Image |
|---------|--------|------|-------|
| knowwhere-ollama-1 | ✅ healthy | 11434 | ollama/ollama:latest |
| knowwhere-kw-postgres-1 | ✅ healthy | 5433 | pgvector/pgvector:pg16 |
| knowwhere-knowwhere-1 | ✅ running | 3737 | knowwhere-knowwhere:latest |

## Gefixte Probleme

### 1. Ollama Healthcheck (CRITICAL)
**Problem:** Ollama Container war `unhealthy` weil `curl` nicht im Image vorhanden war.
**Fix:** `curl` manuell im Container installiert:
```bash
docker exec knowwhere-ollama-1 apt-get update && apt-get install -y curl
```
**Dauerhafter Fix:** Ollama Image sollte `curl` enthalten oder Healthcheck sollte anders funktionieren.

### 2. PostgreSQL Schema — Fehlende Spalten (CRITICAL)
**Problem:** Die `memories` Tabelle fehlte mehrere Spalten die der Code erwartet:
- `content_preview`
- `energy`
- `last_energy_update`
- `parent_id`
- `depth`
- `deleted_at`
- `conflicts_resolved` (in `conflict_detection_runs`)

**Fix:**
```sql
ALTER TABLE memories ADD COLUMN IF NOT EXISTS content_preview TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS energy DOUBLE PRECISION DEFAULT 50.0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS last_energy_update TIMESTAMPTZ DEFAULT NOW();
ALTER TABLE memories ADD COLUMN IF NOT EXISTS parent_id UUID;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS depth INTEGER DEFAULT 0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE conflict_detection_runs ADD COLUMN IF NOT EXISTS conflicts_resolved INTEGER NOT NULL DEFAULT 0;
```

**Root Cause:** `migrations/001_base_schema.sql` war nicht vollständig mit dem Code synchron. Die Migration wurde aktualisiert.

### 3. Embedding Dimension Mismatch (CRITICAL)
**Problem:** PostgreSQL `memories.embedding` war `vector(768)`, aber snowflake-arctic-embed2 produziert 1024-dim Vektoren.
**Fehler:** `expected 768 dimensions, not 1024`

**Fix:**
```sql
ALTER TABLE memories ALTER COLUMN embedding TYPE vector(1024);
ALTER TABLE retrieval_runs ALTER COLUMN embedding TYPE vector(1024);
```

**Migration aktualisiert:** `migrations/001_base_schema.sql` — `vector(768)` → `vector(1024)`

### 4. pgvector Extension
**Problem:** `vector` Typ existierte nicht in der frischen Datenbank.
**Fix:**
```sql
CREATE EXTENSION IF NOT EXISTS vector;
```

## End-to-End Test

### Store Memory
```bash
curl -X POST http://localhost:3737/store_session \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer kw_adm...2024" \
  -d '{"content": "Test memory from Docker setup"}'
```
**Response:** `{"id":"a86d7285-f85e-4331-892e-c44abc0fb297","message":"session node created"}` ✅ HTTP 201

### Retrieve Memory
```bash
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer kw_adm...2024" \
  -d '{"query_text": "Docker setup"}'
```
**Response:** `[{"score":0.02137454,"id":"a86d7285-f85e-4331-892e-c44abc0fb297","memory_type":"episodic","source":"conversation","content":"Test memory from Docker setup",...}]` ✅ HTTP 200

### Health Check
```bash
curl http://localhost:3737/health
```
**Response:** `{"status":"ok","node_count":1}` ✅

## Swagger UI

Verfügbar unter: http://localhost:3737/swagger-ui/

## Offene Punkte / TODO

1. **Ollama Healthcheck:** `curl` ist nicht im offiziellen Ollama Image. Entweder:
   - Custom Ollama Image mit curl bauen
   - Healthcheck auf `wget` oder andere Methode umstellen
   - Oder: Healthcheck komplett entfernen und auf `depends_on` verzichten

2. **Schema-Migration:** Der aktuelle Ansatz (manuelle SQL-Migrationen) funktioniert für Entwicklung, aber für Produktion sollte ein Migration-Tool wie `sqlx migrate` oder `refinery` verwendet werden.

3. **Dockerfile Build-Arg:** `FEATURES` wird im Dockerfile nicht genutzt — es ist hardcoded auf `postgres-storage`. Sollte dynamisch sein.

4. **.env.example:** Enthält immer noch `KNOWLEDGE_API_KEY` statt `KNOWWHERE_API_KEY`.

5. **Automatische Ollama Modelle:** Beim ersten Start müssen die Modelle manuell gepullt werden (`ollama pull snowflake-arctic-embed2`, `ollama pull llama3.2`). Ein Entrypoint-Script könnte das automatisieren.

## Empfohlene nächste Schritte

1. Dockerfile/Compose so anpassen dass Ollama-Modelle automatisch gepullt werden
2. `.env.example` auf `KNOWWHERE_API_KEY` korrigieren
3. Healthcheck für Ollama fixen (curl entfernen oder Image anpassen)
4. README.md mit Docker-Setup-Anleitung aktualisieren
5. Benchmark-Suite im Container testen

## Files geändert

- `migrations/001_base_schema.sql` — vector(768)→vector(1024), fehlende Spalten hinzugefügt

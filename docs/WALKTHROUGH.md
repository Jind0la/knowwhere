# KnowWhere — First-Time User Walkthrough

> Vollständiger E2E-Workflow: Server starten → Account → Erinnerung speichern → Wiederfinden

---

## 1. Server starten

### Option A: Lokaler Rust-Server (Development)

```bash
cd /Users/nimarfranklinmac/knowwhere
cargo run --features postgres-storage --bin knowwhere-server
```

Server läuft auf **http://localhost:3737**

### Option B: Docker

```bash
cd /Users/nimarfranklinmac/knowwhere
docker build -t knowwhere-server:latest --build-arg FEATURES=postgres-storage .
docker run -d \
  --name kw-server \
  -p 3737:3737 \
  -e OLLAMA_API_URL=http://host.docker.internal:11434 \
  -e DATABASE_URL=postgresql://postgres:kw@host.docker.internal:5433/kw \
  -e KNOWWHERE_API_KEY=dein_geheimer_key \
  knowwhere-server:latest
```

> **Wichtig:** `OLLAMA_API_URL` muss auf den Host zeigen (Mac: `host.docker.internal`, Linux: IP des Hosts).
> Port **3737** nicht 3000!

### Voraussetzungen

- **Ollama** muss auf dem Host laufen mit dem Modell `snowflake-arctic-embed2`:
  ```bash
  ollama pull snowflake-arctic-embed2
  ```
- **PostgreSQL** muss laufen (Docker: `docker run -d -p 5433:5432 -e POSTGRES_PASSWORD=kw postgres`)

---

## 2. System-Check

```bash
curl http://localhost:3737/health
```

Erwartete Antwort:
```json
{"status":"ok","embeddings":"ollama:snowflake-arctic-embed2","dimension":1024,"storage":"postgres"}
```

---

## 3. Account registrieren (Register)

```bash
curl -X POST http://localhost:3737/register \
  -H "Content-Type: application/json" \
  -d '{"username": "nimar", "email": "nimar@example.com", "password": "meinpasswort123"}'
```

Erwartete Antwort:
```json
{"api_key":"kw_abc123xyz...","user_id":"...","message":"Registration successful. Save your API key now — it cannot be retrieved again."}
```

> **Wichtig:** `/register` ist nur verfügbar, wenn der Server mit `postgres-storage` + `DATABASE_URL` läuft.
> Ohne PostgreSQL nutze statisch gesetztes `KNOWWHERE_API_KEY`.

Falls du schon einen Account hast, einfach einloggen:

```bash
curl -X POST http://localhost:3737/login \
  -H "Content-Type: application/json" \
  -d '{"username": "nimar", "password": "meinpasswort123"}'
```

Erwartete Antwort:
```json
{"token":"kw_...","expires_at":"never","message":"authenticated"}
```

---

## 4. embedding testen

```bash
curl -X POST http://localhost:3737/embed \
  -H "Content-Type: application/json" \
  -d '{"text": "Nimar ist Softwareentwickler in Berlin"}'
```

Erwartete Antwort:
```json
{"vector":[...],"dimension":1024,"tokens_used":12}
```

Falls Fehler: Ollama läuft nicht oder `OLLAMA_API_URL` zeigt falsch.

---

## 5. Erinnerung speichern

### 5a. Session-basiert speichern (chat-bezogen)

```bash
curl -X POST http://localhost:3737/store_session \
  -H "Content-Type: application/json" \
  -d '{
    "session_id": "nimar-phone-2026-04-04",
    "content": "Nimar bevorzugt Pointer-First Architecture. Er arbeitet an KnowWhere, einem Fractal Memory Service. Projektpfad: /Users/nimarfranklinmac/knowwhere",
    "memory_type": "user_preference"
  }'
```

Erwartete Antwort:
```json
{"id":"01JV...","content":"...","memory_type":"user_preference","session_id":"...","created_at":"..."}
```

### 5b. Externen Pointer speichern (Datei, URL, etc.)

```bash
curl -X POST http://localhost:3737/store_external \
  -H "Content-Type: application/json" \
  -d '{
    "pointer": "/Users/nimarfranklinmac/knowwhere/README.md",
    "pointer_type": "file",
    "memory_type": "project_doc",
    "content": "KnowWhere README — Fractal Memory Service für AI Agents"
  }'
```

Erwartete Antwort:
```json
{"id":"01JW...","pointer":"/Users/nimarfranklinmac/knowwhere/README.md","memory_type":"project_doc"}
```

---

## 6. Erinnerung wiederfinden

### 6a. Einzelne Erinnerung abrufen

```bash
curl "http://localhost:3737/retrieve/01JVXKN3BYT7N4QJZP7VG9RMD"
```

Erwartete Antwort:
```json
{"id":"01JVXKN3BYT7N4QJZP7VG9RMD","content":"Nimar bevorzugt...","memory_type":"user_preference","energy":1.0}
```

### 6b. Fractal Retrieval (semantische Suche)

```bash
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "Was sind Nimars Projektpräferenzen?",
    "limit": 5
  }'
```

Erwartete Antwort:
```json
{
  "results": [
    {
      "id": "01JVXKN3BYT7N4QJZP7VG9RMD",
      "content": "Nimar bevorzugt Pointer-First Architecture...",
      "score": 0.847,
      "memory_type": "user_preference",
      "pointer": null
    }
  ],
  "query_text": "Was sind Nimars Projektpräferenzen?",
  "total_results": 1
}
```

### 6c. Mit vorberechnetem Vector suchen

```bash
# Erst embedden, dann mit dem Vector suchen
VECTOR=$(curl -s -X POST http://localhost:3737/embed \
  -H "Content-Type: application/json" \
  -d '{"text": "Nimar arbeitet an KnowWhere"}' | jq -r '.vector | @json')

curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Content-Type: application/json" \
  -d "{\"query_vector\": $VECTOR, \"limit\": 3}"
```

---

## 7. Nodes verwalten

### Letzte Erinnerungen anzeigen

```bash
curl "http://localhost:3737/nodes/recent?limit=10"
```

### Re-Embed aller Nodes (nach Modellwechsel)

```bash
curl -X POST "http://localhost:3737/nodes/reembed_all" \
  -H "Content-Type: application/json" \
  -d '{"model": "snowflake-arctic-embed2"}'
```

Erwartete Antwort:
```json
{"updated": 152, "failed": 0}
```

### Node löschen

```bash
curl -X DELETE "http://localhost:3737/nodes/{id}"
```

---

## 8. Fractal Memory Status prüfen

### Dream/Consolidation Status

```bash
curl http://localhost:3737/dream/status
```

Erwartete Antwort:
```json
{
  "tier_stats": {
    "L2_consolidated": 0,
    "L1_distilled": 0,
    "L0_raw": 152
  },
  "consolidation": {"status": "idle", "last_run": "2026-04-04T...", "candidates_queued": 0},
  "energy_decay": {"status": "ok", "nodes_above_threshold": 48}
}
```

> **Wichtig:** L2→L1→L0 Consolidation ist derzeit deaktiviert (braucht VLM API Key).
> Siehe `Fractal Memory` in der Haupt-Doku für Details.

---

## 9. Energy & Deduplizierung

### Energy aller Nodes anzeigen

```bash
curl http://localhost:3737/energy/low?threshold=0.5
```

### Energy Boost für einzelne Node

```bash
curl -X POST "http://localhost:3737/memories/{id}/energy/boost" \
  -H "Content-Type: application/json" \
  -d '{"boost": 0.3}'
```

### Deduplizierung

```bash
# Kandidaten finden
curl http://localhost:3737/deduplication/candidates

# Deduplizierung ausführen
curl -X POST http://localhost:3737/deduplication/run
```

---

## 10. Real-Time Events (SSE)

```bash
curl -N http://localhost:3737/events
```

Events: neue Nodes, Consolidation-Fortschritt, Energy-Updates

---

## Troubleshooting

### Ollama antwortet nicht

```bash
# Prüfe ob Ollama läuft
curl http://localhost:11434/api/tags

# Modell prüfen
ollama list
```

### PostgreSQL Connection-Fehler

```bash
# Container prüfen
docker ps | grep kw-postgres

# Connection testen
docker exec -it kw-postgres psql -U postgres -d kw -c "SELECT 1"
```

### Port schon belegt

```bash
# Port 3737 prüfen
lsof -i :3737

# Prozess beenden falls nötig
kill $(lsof -t -i :3737)
```

---

## Nächste Schritte

1. **OpenClaw Plugin** installieren für AI-Agent Integration
   → `docs/openclaw-plugin/README.md`

2. **Webhook Endpoint** für Frigate-Events
   → `POST /webhooks/frigate`

3. **Retrieval Quality** benchmarken
   → `docs/RETRIEVAL-BENCHMARK.md`

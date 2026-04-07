# KnowWhere — First-Time User Walkthrough

> Vollstaendiger Ist-Stand-Workflow: Server starten -> Token pruefen -> Memory speichern -> Retrieval testen -> Chat testen -> Dashboard oeffnen

---

## 1. Voraussetzungen

### Minimal

- Rust 1.85+ oder Docker
- Ollama lokal erreichbar
- ein Embedding-Modell in Ollama

### Fuer Self-Service-Auth

- Build mit `postgres-storage`
- laufendes PostgreSQL
- gueltiges `DATABASE_URL`

### Empfohlene lokale Vorbereitung

```bash
ollama pull nomic-embed-text-v2-moe
```

Alternative fuer 1024-dim Modelle:

```bash
export OLLAMA_MODEL=snowflake-arctic-embed2
export OLLAMA_EMBEDDING_DIMENSION=1024
```

---

## 2. Server starten

### Option A: Lokal mit statischem Admin-Key

```bash
cd /Users/nimarfranklinmac/knowwhere
export KNOWWHERE_API_KEY=dein_geheimer_key
cargo run
```

### Option B: Lokal mit PostgreSQL-Features

```bash
cd /Users/nimarfranklinmac/knowwhere
export KNOWWHERE_API_KEY=dein_geheimer_key
export DATABASE_URL=postgresql://postgres:kw@localhost:5433/kw
cargo run --features postgres-storage --bin knowwhere-server
```

### Option C: Docker Compose

```bash
cd /Users/nimarfranklinmac/knowwhere
export KNOWWHERE_API_KEY=dein_geheimer_key
export POSTGRES_PASSWORD=kw
docker compose up -d
```

Hinweise:

- Der Server hoert standardmaessig auf `http://localhost:3737`
- Das Compose-Setup setzt `OLLAMA_URL=http://host.docker.internal:11434`
- Auf Linux brauchst du eventuell eine explizite Host-IP statt `host.docker.internal`

---

## 3. System-Check

```bash
curl http://localhost:3737/health
```

Erwartete Antwort:

```json
{"status":"ok","node_count":0}
```

Wenn hier kein `ok` zurueckkommt, zuerst Server-Log und Ollama-Verbindung pruefen.

---

## 4. Auth-Modus pruefen

### 4a. Statischer Admin-Key

Wenn du `KNOWWHERE_API_KEY` gesetzt hast, kannst du ihn direkt als Bearer-Token benutzen:

```bash
curl http://localhost:3737/auth/me \
  -H "Authorization: Bearer dein_geheimer_key"
```

Erwartete Antwort fuer einen Admin-Key:

```json
{
  "token_kind": "admin",
  "allowed_retrieval_profiles": [
    "user-facing",
    "agent-debug",
    "full-fidelity"
  ]
}
```

### 4b. Self-Service User registrieren

Nur verfuegbar mit `postgres-storage` plus `DATABASE_URL`.

```bash
curl -X POST http://localhost:3737/register \
  -H "Content-Type: application/json" \
  -d '{
    "username": "nimar",
    "email": "nimar@example.com",
    "password": "meinpasswort123"
  }'
```

Erwartete Antwort:

```json
{
  "api_key": "kw_abc123xyz",
  "user_id": "....",
  "message": "Registration successful. Save your API key now — it cannot be retrieved again."
}
```

Danach einloggen:

```bash
curl -X POST http://localhost:3737/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "nimar",
    "password": "meinpasswort123"
  }'
```

Erwartete Antwort:

```json
{
  "token": "kw_session_...",
  "expires_at": "2026-04-30T12:34:56Z",
  "message": "authenticated"
}
```

Mit diesem User-Token liefert `/auth/me` aktuell nur:

```json
{
  "token_kind": "user",
  "allowed_retrieval_profiles": ["user-facing"]
}
```

---

## 5. Embedding testen

```bash
curl -X POST http://localhost:3737/embed \
  -H "Authorization: Bearer dein_geheimer_key" \
  -H "Content-Type: application/json" \
  -d '{"text":"KnowWhere speichert Sessions voll und Externe nur als Pointer"}'
```

Erwartete Form:

```json
{
  "vector": [0.01, -0.02, 0.03],
  "dimension": 768,
  "provider": "local-ollama"
}
```

Wenn du `snowflake-arctic-embed2` nutzt, ist die Dimension typischerweise `1024`.

---

## 6. Session-Memory speichern

```bash
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer dein_geheimer_key" \
  -H "Content-Type: application/json" \
  -d '{
    "content": "Nimar bevorzugt Pointer-First Architecture und arbeitet an KnowWhere.",
    "memory_type": "semantic",
    "metadata": {
      "source": "walkthrough"
    }
  }'
```

Erwartete Form:

```json
{
  "id": "01JV...",
  "message": "memory stored"
}
```

---

## 7. Externen Pointer speichern

```bash
curl -X POST http://localhost:3737/store_external \
  -H "Authorization: Bearer dein_geheimer_key" \
  -H "Content-Type: application/json" \
  -d '{
    "pointer": "/Users/nimarfranklinmac/knowwhere/README.md",
    "memory_type": "semantic",
    "metadata": {
      "source": "walkthrough:file"
    }
  }'
```

Damit wird nur der Pointer plus Metadaten gespeichert, nicht die Datei selbst.

---

## 8. Retrieval testen

### 8a. Textbasierte Suche

```bash
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer dein_geheimer_key" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "Welche Architekturpraeferenzen sind gespeichert?",
    "top_k": 5,
    "max_depth": 3,
    "retrieval_profile": "user-facing"
  }'
```

### 8b. Retrieval mit Debug-Infos

Nur sinnvoll mit einem Admin-Token:

```bash
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer dein_geheimer_key" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "Welche Architekturpraeferenzen sind gespeichert?",
    "top_k": 5,
    "max_depth": 3,
    "retrieval_profile": "agent-debug",
    "include_debug": true
  }'
```

Wichtig:

- `user-facing` ist das sichere Default-Profil
- `agent-debug` und `full-fidelity` sind nur fuer Admin-Tokens erlaubt
- die Server-Seite erzwingt diese Profile unabhaengig vom Client

---

## 9. Subconscious Chat testen

```bash
curl -X POST http://localhost:3737/chat/subconscious \
  -H "Authorization: Bearer dein_geheimer_key" \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Was weisst du ueber Nimars Architekturentscheidungen?",
    "top_k": 5,
    "max_depth": 3,
    "retrieval_profile": "user-facing",
    "include_debug": true,
    "persist": false
  }'
```

Erwartete Form:

```json
{
  "answer": "....",
  "sources": [
    {
      "id": "01JV...",
      "score": 0.87,
      "memory_type": "semantic",
      "snippet": "Nimar bevorzugt Pointer-First Architecture...",
      "retrieval_profile": "user-facing",
      "trust_tier": "primary"
    }
  ],
  "stored": false
}
```

`persist` ist standardmaessig aus, damit Chat-Nachrichten nicht ungeplant neue Retrieval-Spuren erzeugen.

---

## 10. Dashboard oeffnen

Das aktive Operator-Frontend liegt in `dashboard/`.

```bash
cd /Users/nimarfranklinmac/knowwhere/dashboard
npm ci
npm run dev
```

Dann im Browser:

- `http://localhost:5173` oeffnen
- Token eintragen
- `Overview`, `Memories`, `Chat`, `Search` und `Governance` pruefen

Wichtig:

- Das Dashboard liest `GET /auth/me`
- Search und Chat zeigen nur die Retrieval-Profile an, die dein Token wirklich darf
- Der Backend-Server liefert weiterhin ein minimales `frontend/` als Fallback aus, aber die aktuelle Entwicklungsoberflaeche ist das React-Dashboard

---

## 11. Troubleshooting

### Ollama antwortet nicht

```bash
curl http://localhost:11434/api/tags
ollama list
```

Pruefe danach `OLLAMA_URL`, `OLLAMA_MODEL` und ggf. `OLLAMA_EMBEDDING_DIMENSION`.

### `503 Service Unavailable` auf `/register` oder `/login`

Der Server laeuft nicht mit PostgreSQL-Auth-Support. Du brauchst:

- `cargo run --features postgres-storage`
- ein gueltiges `DATABASE_URL`

### `401 Unauthorized` auf geschuetzten Routen

```bash
curl http://localhost:3737/auth/me \
  -H "Authorization: Bearer dein_token"
```

Wenn das fehlschlaegt, ist der Token falsch oder der Server laeuft mit einem anderen `KNOWWHERE_API_KEY`.

### Port 3737 ist belegt

```bash
lsof -i :3737
```

Dann entweder den bestehenden Prozess sauber stoppen oder das Port-Mapping aendern.

---

## Naechste Schritte

1. OpenClaw anbinden: `../openclaw-plugin/README.md`
2. Import-Strategie verstehen: `IMPORT_GUIDE.md`
3. Architektur vertiefen: `ARCHITECTURE.md`

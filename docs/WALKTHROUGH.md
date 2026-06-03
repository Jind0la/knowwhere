# KnowWhere Walkthrough — v0.6.0

Schritt-für-Schritt-Anleitung für den ersten End-to-End-Durchlauf mit KnowWhere.

## Voraussetzungen

- Server läuft auf `http://localhost:3737` (siehe [QUICKSTART.md](QUICKSTART.md))
- `KNOWWHERE_API_KEY` ist gesetzt
- Ollama läuft mit `nomic-embed-text`

## 1. Authentifizierung

```bash
# Token holen (mit Admin-API-Key)
curl -X POST http://localhost:3737/login \
  -H "Content-Type: application/json" \
  -d '{"api_key": "kw_..."}'

# Antwort enthält access_token → für alle weiteren Requests nutzen
export TOKEN="eyJ..."
```

## 2. Erste Konversation speichern

```bash
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "session_id": "walkthrough-1",
    "turns": [
      {
        "role": "user",
        "content": "Ich arbeite an einem Rust-Projekt und brauche eine schnelle Datenbank.",
        "turn_index": 0
      },
      {
        "role": "assistant",
        "content": "SQLite ist ideal für Rust — embedded, keine Server-Installation, und rusqlite ist ein exzellenter Treiber.",
        "turn_index": 1
      },
      {
        "role": "user",
        "content": "Gute Idee. Ich mag SQLite weil es keine Konfiguration braucht.",
        "turn_index": 2
      }
    ]
  }'
```

**Was passiert:** Jeder Turn bekommt ein eigenes Embedding (nomic-embed-text, 768-dim) plus Metadaten (Speaker-Rolle, Turn-Index, Session-ID). Kein Session-Aggregat-Verlust mehr.

## 3. Externe Daten importieren

```bash
# Dokumente, Code-Snippets, Notizen importieren
curl -X POST http://localhost:3737/store_external \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "content": "rusqlite bietet: prepared statements, transactions, blob I/O, und user-defined functions. Kein unsafe code nötig für die meisten Operationen.",
    "metadata": {
      "source_type": "documentation",
      "topic": "rust",
      "external_id": "rusqlite-docs"
    }
  }'
```

## 4. Retrieval — Fakten finden

KnowWhere's Kern-Feature: **Fractal Retrieval** findet Information auf jeder Auflösungsebene.

```bash
# Semantische Suche
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "Welche Datenbank wurde für Rust empfohlen?",
    "top_k": 5,
    "diversity": true,
    "temporal_weight": 0.3
  }'
```

**Antwort-Struktur:**
```json
{
  "results": [
    {
      "node_id": "uuid...",
      "score": 0.87,
      "content": "SQLite ist ideal für Rust...",
      "metadata": {
        "speaker_role": "assistant",
        "turn_index": 1,
        "session_id": "walkthrough-1"
      },
      "context_tier": "raw",
      "source_type": "real (1.0×)",
      "score_debug": {
        "base_score": 0.92,
        "temporal_weight": 0.52,
        "source_weight": 1.0,
        "final_score": 0.87
      }
    }
  ]
}
```

## 5. Fractal Zoom — von Overview zu Rohdaten

```bash
# Erstes Ergebnis aus 4. nehmen und expandieren
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "rust datenbank empfehlung",
    "top_k": 3,
    "expand_nodes": ["<node-id-aus-schritt-4>"]
  }'
```

Fractal Zoom zeigt: Child-Nodes, Relations, und den Pointer zurück zur Original-Konversation.

## 6. Hybrid-Suche — Keyword + Semantik

Wenn reine Vektor-Suche versagt (z.B. bei Eigennamen, technischen IDs):

```bash
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "rusqlite prepared statements transaction",
    "top_k": 5
  }'
```

KnowWhere kombiniert automatisch **BM25-Keyword-Matching** mit **dichter Vektor-Suche** via RRF-Fusion (Reciprocal Rank Fusion). Kein separater Endpoint nötig.

## 7. Source-Type Weighting

```bash
# Synthetische Inhalte niedriger gewichten
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "datenbank empfehlung",
    "top_k": 5,
    "source_type_weights": {
      "real": 1.0,
      "synthetic": 0.5,
      "derived": 0.3,
      "unknown": 0.8
    }
  }'
```

Nodes werden automatisch klassifiziert: **Real** (Konversationen, Importe), **Synthetic** (KI-generiert), **Derived** (Summaries), **Unknown** (fehlende Provenance).

## 8. Reranking mit Cross-Encoder

```bash
# Erst 20 Kandidaten holen, dann reranken
curl -X POST http://localhost:3737/rerank \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "query": "beste rust datenbank",
    "candidates": [
      {"id": "uuid-1", "content": "SQLite ist ideal für Rust..."},
      {"id": "uuid-2", "content": "PostgreSQL mit pgvector..."},
      {"id": "uuid-3", "content": "Ich mag Python für Web-Apps..."}
    ]
  }'
```

Der gte-modernbert ONNX Cross-Encoder reranked präziser als reine Vektor-Distanz — ohne Ollama-Abhängigkeit.

## 9. Batch-Storage für Geschwindigkeit

```bash
curl -X POST http://localhost:3737/store_session_batch \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "sessions": [
      {
        "session_id": "batch-1",
        "turns": [
          {"role": "user", "content": "Thema A...", "turn_index": 0},
          {"role": "assistant", "content": "Antwort A...", "turn_index": 1}
        ]
      },
      {
        "session_id": "batch-2",
        "turns": [
          {"role": "user", "content": "Thema B...", "turn_index": 0}
        ]
      }
    ]
  }'
```

Mehrere Sessions in einem Request — Embedding-Generierung läuft parallel.

## Nächste Schritte

- **API-Referenz:** [API_REFERENCE.md](API_REFERENCE.md) — alle 32 Endpoints
- **Architektur:** [../ARCHITECTURE_MAP.md](../ARCHITECTURE_MAP.md) — Modul-Diagramm
- **Konfiguration:** `cp .env.example .env` und anpassen
- **Dashboard:** `open http://localhost:3737/dashboard` (wenn aktiviert)

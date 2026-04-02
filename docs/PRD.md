# KnowWhere — Product Requirements Document

> Stand: 25. Maerz 2026 — v0.3.0

## 1. Produktname & One-Sentence Pitch

**KnowWhere**
„Dein KI-Gedaechtnis – ohne jemals deine Daten anzufassen."

KnowWhere ist das erste echte Plug-and-Play Langzeitgedaechtnis fuer KI: ein fraktales, adaptives, multimodales Memory-System, das alle Session- und Chat-Daten vollstaendig speichert, aber externe Rohdateien (Kamera, Google Drive, Sensoren etc.) nur als Pointer referenziert. So wird jede KI ueber Monate und Jahre hinweg zum echten digitalen Zwilling deines Denkens – ohne Datenduplikation, ohne Lock-in, ohne Risiko.

## 2. The Why – Simon Sinek Style

**Why:**
Weil KI heute brillant, aber amnesiekrank ist. Sie vergisst nach wenigen Minuten deine Prinzipien, deine Vision, deine „Nie wieder so!"-Entscheidungen. Wir bauen KnowWhere, weil echte Intelligenz ohne echtes, langfristiges Gedaechtnis unmoeglich ist – und weil dieses Gedaechtnis deine Datenhoheit respektieren muss.

**How:**
Durch eine komplett neue fraktale Architektur mit organisch wachsenden, ueberlappenden Clustern, Hybrid Search (Vektor + Keyword), und einem inkrementellen „Dream Mode".

**What:**
Ein schlanker, eigenstaendiger Memory-Service (Cloud + Self-Hosted) mit winzigen SDKs, der in 3 Zeilen in jeden Agenten, LLM oder Framework integriert wird.

## 3. First Principles

1. Intelligenz = Verknuepfung vergangener Erfahrungen mit neuen Situationen.
2. Speicher = totes Regal. Gedaechtnis = lebendiges Netzwerk.
3. Der User behaelt 100 % Kontrolle ueber seine Rohdaten.
4. Skalierung muss exponentiell effizient sein.
5. Kein „Erklaer mir nochmal…" darf je wieder noetig sein.
6. **KnowWhere ist additiv, niemals destruktiv.** Bestehende Memory-Systeme, Dateien und Konversationshistorien des Host-Systems werden respektiert, nicht ersetzt oder geloescht.

## 4. Outcome – Was der User am Ende wirklich hat

- **Nach 6 Monaten Vibe-Coding:** Die KI kennt deine komplette App-Vision, alle frueheren Entscheidungen und Fehler – automatisch.
- **Nach 3 Monaten Smart-Home:** Dein Agent weiss von allein „Person X betritt um 20:15 den Raum, spricht ueber Projekt Y, Temperatur 22,3 °C".
- **70–90 % weniger Wiederholungen**, kreativere Vorschlaege, echte persoenliche KI.

**North Star Metric:**
30-Day Context Fidelity > 92 % (Queries, die korrekt auf historische Kontexte zugreifen – ohne Korrektur).

## 5. High-Level Architektur

```
[LLM / Agent / Kamera-System]
    ←→ KnowWhere Client SDK / Plugin
    ←→ KnowWhere Memory Service (Rust, Port 3737)
           ↓
    Hybrid Retrieval Engine
    ├── USearch (Vektor / Cosine Similarity)
    ├── BM25 (Keyword / deutsch-optimiert)
    └── Reciprocal Rank Fusion
           ↓
    Storage:
    • Sessions/Chats → volle Daten + Embeddings
    • Externe Quellen → nur Pointer + Embedding + Metadaten
    • Persistenz → state.json mit Auto-Save + Graceful Shutdown
```

## 6. Die fraktale Datenstruktur

```rust
pub enum NodeType { Session, External }

pub struct FractalNode {
    id: UUID,
    node_type: NodeType,
    vector: Vec<f32>,                  // 768-dim (nomic-embed-text-v2-moe)
    content: Option<String>,           // Nur bei Sessions voll
    original_pointer: Option<String>,  // Bei externen Daten
    metadata: HashMap<String, Value>,
    weight: f64,
    multimodal: Option<MultimodalData>,
    children: Vec<FractalNode>,
    relations: Vec<Relation>,
    created_at: DateTime,
    last_accessed: DateTime,
}
```

## 7. Die Kern-Operationen

| Operation           | Beschreibung                                          | Status |
|---------------------|-------------------------------------------------------|--------|
| `store_session`     | Volle Speicherung von Chats/Sessions + auto-embed     | ✓      |
| `store_external`    | Nur Pointer + Embedding + Metadaten                   | ✓      |
| `retrieve_fractal`  | Hybrid Search + fraktales Zoomen → `ScoredNode[]`     | ✓      |
| `embed`             | Text → Embedding-Vektor (mit Task-Prefix)             | ✓      |
| `reembed_all`       | Alle Nodes neu embedden (nach Modellwechsel)          | ✓      |
| `delete` / `purge`  | Einzelne Nodes loeschen / Dummies entfernen           | ✓      |
| Dream Mode          | Periodisches Micro-Clustering (stuendlich)            | ✓      |
| Persistence         | Auto-Save + Graceful Shutdown (SIGINT/SIGTERM)        | ✓      |

## 8. Der Dream Mode (inkrementell)

- Stuendliche Micro-Dreams (leichtgewichtiges Clustering)
- Woechentlicher Full-Dream (geplant fuer spaetere Phasen)
- Organische Cluster-Bildung durch Verbindungen → Retrieval wird immer besser

## 9. Plug-and-Play Integration

### OpenClaw (produktionsreif)

Plugin mit drei Hooks:
- `message_received` → User-Nachrichten speichern
- `llm_output` → AI-Antworten speichern (mit Modell-Info)
- `before_prompt_build` → Kontext abrufen und injizieren

### Python SDK

```python
from knowwhere import KnowWhereClient

client = KnowWhereClient(base_url="http://localhost:3737")
client.store_session("Die App soll anonym sein, kein Login")
results = client.retrieve_fractal("Welche Design-Entscheidung?")
```

### Beliebiger Agent (3-Schritt-Muster)

1. `POST /store_session` — Nachricht speichern
2. `POST /embed` + `POST /retrieve_fractal` — Kontext abrufen
3. Kontext in Prompt injizieren

## 10. Tech-Stack

| Komponente       | Technologie                            | Status      |
|------------------|----------------------------------------|-------------|
| Backend          | Rust 1.85+ (Axum 0.8, Tokio, Tower)   | ✓ Produktion |
| Embedding        | Ollama `nomic-embed-text-v2-moe` (768d)| ✓ Produktion |
| Embedding (Cloud)| Grok (xAI), OpenAI                     | ✓ Optional   |
| Vector Store     | USearch (HNSW, Cosine)                 | ✓ Produktion |
| Keyword Search   | BM25 (Cached Scorer, German)           | ✓ Produktion |
| Fusion           | Reciprocal Rank Fusion (k=60)          | ✓ Produktion |
| Persistence      | JSON state file (debounced auto-save)  | ✓ Produktion |
| Connectors       | Frigate NVR (pointer-first)            | ✓ Optional   |
| SDK              | Python (LangChain-kompatibel)          | ✓ Beta       |
| Dashboard        | Vanilla JS + Tailwind                  | ✓ Beta       |
| API Docs         | OpenAPI 3.0 + Swagger UI               | ✓ Produktion |

## 11. Roadmap

| Phase                  | Beschreibung                                      | Status       |
|------------------------|---------------------------------------------------|--------------|
| Phase 0 (MVP)          | Sessions, Text-Pointers, REST API, USearch        | ✓ Abgeschlossen |
| Phase 0.5 (Hybrid)     | BM25, RRF, nomic-v2-moe, ScoredNode, Persistence | ✓ Abgeschlossen |
| Phase 1 (Dream)        | Dream Mode + Audio/Sensoren                       | ✓ Micro-Dream  |
| Phase 1.5 (Integration)| OpenClaw Plugin (voller Memory Loop)              | ✅ Implementiert — [`openclaw-plugin/`](openclaw-plugin/), 6 Hooks aktiv, E2E-verifiziert 2026-03-29 |
| Phase 1.7 (Import)     | Memory-Import aus Host-Systemen (OpenClaw done)   | ✓ Abgeschlossen |
| Phase 2 (Connectors)   | Webhooks fuer Drive, Frigate, Home Assistant       | In Planung   |
| Phase 2.5 (Discovery)  | Auto-Discovery + Auto-Import von Host-Memories    | In Planung   |
| Phase 3 (Scale)        | LanceDB/NebulaGraph, Multi-Tenant, Cloud-SaaS     | In Planung   |
| Phase 4 (Open Source)   | Open-Source-Core + Cloud-Hosting-Angebot          | In Planung   |

### Phase 2.5: Auto-Discovery (Vision)

KnowWhere soll bestehende Memory-Systeme **automatisch erkennen** und importieren:

1. **Scan** — `POST /import/discover` scannt definierte Pfade nach bekannten Agent-Systemen
2. **Classify** — Gefundene Dateien werden automatisch klassifiziert (Memory, Identity, Research, Noise)
3. **Preview** — User bekommt eine Vorschau: "Gefunden: 42 Memory-Dateien, 6 Agent-Profile, 159 Sessions"
4. **Import** — `POST /import/execute` importiert mit konfigurierbaren Filtern (skip_cron, min_length etc.)
5. **Verify** — Automatische Test-Queries ueber alle importierten Domaenen

Erkannte Systeme: OpenClaw, LangChain, LlamaIndex, CrewAI, Cursor, Custom.
Details: siehe `docs/IMPORT_GUIDE.md`

## 12. Integration Rules (Non-Negotiable)

Wenn KnowWhere in ein bestehendes Agent-System (OpenClaw, LangChain, etc.) integriert wird, gelten folgende Regeln:

### 12.1 Keine Daten loeschen

KnowWhere darf **niemals** bestehende Memories, Dateien oder Konversationshistorien des Host-Systems loeschen, ueberschreiben oder zuruecksetzen. Dazu gehoeren:

- Session-Historien / Transkripte
- Memory-Dateien (z.B. MEMORY.md, daily logs)
- Identitaets- und Konfigurationsdateien (z.B. IDENTITY.md, SOUL.md, BOOTSTRAP.md)
- Bestehende Embeddings oder Vektordatenbanken

### 12.2 Bestehende Memories importieren

Bei der Installation muss KnowWhere die vorhandenen Memories des Host-Systems **importieren**:

1. Bestehende Session-Historien einlesen und als Session-Nodes speichern
2. Memory-Dateien (z.B. `memory/*.md`) einlesen und indexieren
3. Originaldateien als Referenz beibehalten — KnowWhere ist eine zusaetzliche Schicht, kein Ersatz

### 12.3 Additiv integrieren

- Host-System-Dateien nur **ergaenzen**, nie ersetzen
- Neue Abschnitte zu bestehenden Konfigurationsdateien hinzufuegen, bestehende Inhalte nicht aendern
- Das Host-Memory-System (z.B. OpenClaws `memory-core`, `MEMORY.md`) kann parallel weiterlaufen
- KnowWhere kommt als Layer obendrauf und liefert zusaetzlichen Kontext

### 12.4 Graceful Degradation

- Wenn KnowWhere offline ist, muss das Host-System normal weiterarbeiten
- Keine harten Abhaengigkeiten — KnowWhere ist immer optional
- Circuit-Breaker-Pattern fuer alle API-Aufrufe

## 13. Eventualitaeten & Loesungen

| Risiko                   | Loesung                                             |
|--------------------------|-----------------------------------------------------|
| Externer API-Ausfall     | Lazy-Loading + Fallback auf lokales Ollama           |
| Datenschutz              | Pointer-First + E2E-Verschluesselung (geplant)      |
| Speicherkosten           | ~5–10 KB pro Knoten (nur Embeddings + Metadaten)     |
| Mac RAM-Engpass          | BM25-Caching, Ollama-Modell-Cleanup, Debounced Save  |
| Streaming-Delivery       | `llm_output` Hook statt `message_sent`               |
| Context-Qualitaet        | Markdown-Cleaning + Task-Prefixes + Hybrid Search    |

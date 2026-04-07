# KnowWhere Architecture

> Stand: April 2026 — Repository `main`, Paketversion `0.1.0`

## 1. High-level overview

KnowWhere ist ein eigenstaendiger Memory-Service als Rust-Binary mit HTTP-API. Clients integrieren sich ueber REST und muessen nicht direkt an interne Bibliotheken gekoppelt werden.

Aktuell besteht die Architektur aus vier Hauptschichten:

1. **Client-Schicht**
   - Agenten, SDKs, OpenClaw-Plugin, React-Dashboard

2. **API- und Auth-Schicht**
   - Axum-Router
   - Bearer-Token-Middleware
   - Capability-Endpoint `GET /auth/me`

3. **Memory- und Retrieval-Schicht**
   - StorageBackend
   - EmbeddingProvider
   - Hybrid Retrieval mit Profilen

4. **Operations-Schicht**
   - Dream status
   - Events
   - Governance
   - PostgreSQL-Lifecycle-Routen bei aktivem `postgres-storage`

## 2. Laufzeittopologie

```text
Agent / SDK / Dashboard
        |
        v
Axum Router
  |- public routes: /health, /swagger-ui, /register, /login, /refresh
  |- protected routes: /auth/me, /embed, /store_*, /retrieve_*, /chat/subconscious, ...
        |
        v
Auth middleware -> AuthContext(token_kind, allowed_retrieval_profiles)
        |
        v
StorageBackend + EmbeddingProvider
  |- MemoryStore (default, JSON-backed)
  |- PostgresStore (optional, postgres-storage)
  |- Local Ollama / OpenAI / Grok
        |
        v
Operational workers
  |- Dream-related status and schedulers
  |- VLM worker (optional)
  |- Frigate connector (optional)
```

## 3. Repository structure

```text
knowwhere/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── api/
│   │   ├── auth.rs
│   │   ├── docs.rs
│   │   └── routes.rs
│   ├── embedding/
│   │   ├── mod.rs
│   │   └── provider.rs
│   ├── memory/
│   │   ├── fractal_node.rs
│   │   ├── governance.rs
│   │   ├── namespaces.rs
│   │   ├── skills.rs
│   │   ├── self_healing.rs
│   │   └── dream/
│   ├── storage/
│   │   ├── backend.rs
│   │   ├── in_memory.rs
│   │   └── postgres_store.rs
│   ├── scheduler/
│   ├── connectors/
│   └── vlm/
├── dashboard/              # React/Vite operator UI
├── frontend/               # minimal static fallback served by the backend
├── sdk/python/
├── docs/
└── .github/workflows/ci.yml
```

Wichtige Klarstellung:

- `dashboard/` ist die aktive Entwicklungsoberflaeche
- `frontend/` wird weiterhin vom Backend via `ServeDir::new("frontend")` als einfacher Fallback ausgeliefert

## 4. API-Aufbau

### 4.1 Oeffentliche Routen

- `GET /health`
- `GET /swagger-ui/*`
- `POST /register`
- `POST /login`
- `POST /refresh`

Die drei Auth-Mutationsrouten sind nur funktional, wenn der Prozess mit `postgres-storage` plus erreichbarem `DATABASE_URL` laeuft. Sonst liefern sie `503`.

### 4.2 Geschuetzte Kernrouten

- `GET /auth/me`
- `POST /embed`
- `POST /store_session`
- `POST /store_external`
- `GET /retrieve/{id}`
- `POST /retrieve_fractal`
- `POST /chat/subconscious`
- `GET /nodes/recent`
- `POST /nodes/reembed_all`
- `GET /dream/status`
- `GET /events`
- `GET` / `POST /governance/policy`

### 4.3 Geschuetzte PostgreSQL-Routen

Nur bei aktivem `postgres-storage`:

- Retrieval runs und trajectories
- conflict management
- energy operations
- deduplication
- self-healing
- namespaces
- skills

## 5. Auth- und Capability-Modell

Die Auth-Schicht baut fuer jeden autorisierten Request einen `AuthContext` auf:

```rust
pub struct AuthContext {
    pub token_kind: AuthTokenKind,
    pub user_id: Option<Uuid>,
    pub allowed_retrieval_profiles: Vec<RetrievalProfile>,
}
```

### 5.1 Token-Arten

- `admin`
  - stammt aus `KNOWWHERE_API_KEY`
  - darf `user-facing`, `agent-debug`, `full-fidelity`

- `user`
  - stammt aus PostgreSQL-Auth
  - darf aktuell nur `user-facing`

### 5.2 Warum `GET /auth/me` architektonisch wichtig ist

Das Dashboard und andere Clients raten nicht mehr, welche Profile erlaubt sind. Sie lesen den Ist-Zustand direkt vom Server und rendern Optionen entsprechend. Dadurch liegen Rechte und UI nicht auseinander.

## 6. Datenmodell

Die zentrale Datenstruktur ist `FractalNode`.

```rust
pub struct FractalNode {
    pub id: Uuid,
    pub node_type: NodeType,
    pub vector: Vec<f32>,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    pub metadata: HashMap<String, Value>,
    pub children: Vec<FractalNode>,
    pub relations: Vec<Relation>,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}
```

### 6.1 Pointer-first Regel

- Session-Nodes: `content` ist gefuellt
- External-Nodes: `original_pointer` ist gefuellt
- Externe Rohdaten gehoeren nicht in den Store

### 6.2 API-Retrieval-Form

Die API gibt keine rohen Vektoren in Retrieval-Antworten zurueck. Stattdessen werden `ScoredNode`-artige Antworten mit Score, Node-Daten und optionalem Score-Debug geliefert.

## 7. Retrieval-Pipeline

Der aktuelle Flow fuer `POST /retrieve_fractal` ist:

1. Query-Text und/oder Query-Vektor entgegennehmen
2. Embedding berechnen, falls nur Text gegeben ist
3. Kandidaten ueber Vector Search holen
4. Kandidaten ueber BM25 holen
5. Kandidaten ggf. ueber Fractal Zoom vertiefen
6. Rankings ueber RRF fusionieren
7. Ergebnisse profilabhaengig gewichten und filtern
8. Ergebnisse mit optionalem Debug zurueckgeben

### 7.1 Retrieval-Profile

`RetrievalProfile` ist ein Architekturhebel, kein UI-Detail:

- `user-facing`
  - filtert interne-only Inhalte
  - gewichtet Trust-Tiers konservativer

- `agent-debug`
  - bleibt konsumierbar, zeigt aber mehr Debug-Signale

- `full-fidelity`
  - keine zusaetzliche Profilgewichtung
  - fuer rohe Operator- und Debug-Sicht

## 8. Embedding-Architektur

### 8.1 Provider

Aktuell verfuegbar:

- `LocalOllamaProvider`
- `OpenAIProvider`
- `GrokProvider`

### 8.2 Auswahlreihenfolge

1. `KNOWWHERE_EMBEDDING_PROVIDER`, wenn gesetzt
2. Grok bei `GROK_API_KEY` plus Feature
3. OpenAI bei `OPENAI_API_KEY` plus Feature
4. Lokales Ollama als Default

### 8.3 Wichtige Env-Vars

- `OLLAMA_URL`
- `OLLAMA_MODEL`
- `OLLAMA_EMBEDDING_DIMENSION`
- `KNOWWHERE_EMBEDDING_PROVIDER`

Das ist architektonisch wichtig, weil die Vektordimension nicht hart auf `768` festgelegt ist. Der Provider muss zum gewaehlten Modell passen.

## 9. Storage-Backends

### 9.1 `MemoryStore`

Default-Backend:

- in-memory Datenstruktur
- JSON-Persistenz im Datenverzeichnis
- geeignet fuer lokale Entwicklung und einfache Single-Node-Deployments

### 9.2 `PostgresStore`

Optionales erweitertes Backend:

- aktiviert ueber `postgres-storage`
- braucht ein funktionierendes `DATABASE_URL`
- liefert erweiterte Features fuer Retrieval-Analytik, Lifecycle, Dedup, Konflikte und Auth

Architektonisch wichtig:

- Die API bleibt weitgehend gleich
- Der Funktionsumfang der Routes aendert sich je nach aktivem Backend und Feature-Set

## 10. Frontend-Architektur

### 10.1 React-Dashboard in `dashboard/`

Das aktuelle Dashboard ist eine Vite-App mit `/api`-Proxy zum Backend.

Es bietet aktuell:

- Overview
- Memory stream
- Search
- Subconscious chat
- Governance view

Designentscheidung:

- Capabilities werden ueber `/auth/me` geladen
- Search und Chat rendern Retrieval-Profile nur, wenn der Token sie wirklich darf

### 10.2 Minimales Fallback-Frontend in `frontend/`

Das Backend serviert weiterhin `frontend/` als einfache statische Oberflaeche. Diese ist funktional begrenzt und kein vollwertiger Ersatz fuer das React-Dashboard.

## 11. Operations und Nebenprozesse

### 11.1 Dream und Scheduler

KnowWhere besitzt Scheduler-/Dream-bezogene Komponenten fuer Wartung und organische Verbesserung der Memory-Struktur. Der aktuelle Operator-Zugriff erfolgt vor allem ueber Status-Endpunkte.

### 11.2 VLM Worker

Ein optionaler VLM-Worker kann Summarization-/Compression-bezogene Aufgaben uebernehmen, wenn passende Provider-Variablen gesetzt sind, z. B. `OLLAMA_VLM_MODEL`.

### 11.3 Connectoren

Beispiel: Frigate. Wenn `FRIGATE_URL` gesetzt ist, koennen Ereignisse als External-Nodes pointer-first gespeichert werden.

## 12. CI und Verifikation

Die Architektur wird in CI auf mehreren Ebenen abgesichert:

- Rust fmt, clippy, check, unit tests
- OpenAPI contract smoke tests
- PostgreSQL-Integrationstests mit `pgvector`
- Ollama-gestuetzte Testpfade
- Feature-Matrix fuer Provider-/Storage-Kombinationen
- Dashboard-Build
- Docker-Build

Das ist wichtig, weil die Architektur absichtlich feature-gated ist und Default- sowie PostgreSQL-Modus beide valide bleiben muessen.

## 13. Integrationsregeln

Wenn KnowWhere in ein bestehendes Host-System eingebunden wird:

1. bestehende Memories zuerst importieren
2. Host-Dateien nie loeschen oder ueberschreiben
3. Host-Konfiguration nur ergaenzen
4. Host-Memory-System parallel weiterlaufen lassen
5. bei Ausfall von KnowWhere sauber degradieren

## 14. Architekturgrenzen im aktuellen Stand

Noch nicht fertig oder bewusst begrenzt:

- kein vollstaendiges UI fuer alle erweiterten PostgreSQL-Routen
- keine automatische Storage-Migration
- keine Multi-Tenant-SaaS-Architektur
- keine Runtime-Hot-Swaps fuer Embedding-Provider
- keine uniforme Release-Versionierung ueber Marketing- und Paketversion hinaus

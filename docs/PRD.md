# KnowWhere — Product Requirements Document

> Stand: April 2026 — Repository `main`, Paketversion `0.1.0`

## 1. Produktname und One-Sentence Pitch

**KnowWhere**

"Ein pointer-first Langzeitgedaechtnis fuer KI-Agenten, das Sessions voll speichert und externe Daten nur referenziert."

KnowWhere ist kein allgemeiner Dateispeicher und kein Ersatz fuer bestehende Memory-Systeme. Es ist eine additive Memory-Schicht, die Kontext strukturiert speichert, wiederfindbar macht und sicher an Agenten oder Dashboards ausliefert.

## 2. Das Problem

Heutige KI-Systeme sind stark im Moment, aber schwach ueber Zeit. Wichtige Entscheidungen, Nutzerpraeferenzen, Projektverlauf und externe Referenzen gehen zwischen Sessions verloren oder muessen immer wieder manuell neu erklaert werden.

Das Kernproblem hat drei Teile:

1. **Amnesie:** Kontext verschwindet zwischen Anfragen.
2. **Datenhoheit:** Externe Rohdaten sollten nicht blind kopiert werden.
3. **Operativer Realismus:** Bestehende Agent-Systeme duerfen nicht gebrochen oder ersetzt werden.

## 3. Produktprinzipien

1. **Pointer-first.** Externe Quellen werden nur als Pointer plus Metadaten gespeichert.
2. **Sessionen duerfen voll gespeichert werden.** Chat- und Session-Inhalte bleiben als Text plus Embedding erhalten.
3. **Hybrid retrieval statt nur Vector Search.** Semantik und Schlagwoerter werden kombiniert.
4. **Additiv, niemals destruktiv.** Host-Systeme werden ergaenzt, nicht ersetzt.
5. **Capabilities statt impliziter Rechte.** Der Client liest ueber `GET /auth/me`, was ein Token darf.
6. **Saubere Betriebsmodi.** Einfache lokale Defaults, aber klarer Ausbaupfad Richtung PostgreSQL und erweiterte Features.

## 4. Zielbild fuer Nutzer

Ein Nutzer oder Agent-Betreiber soll:

- Kontext ueber Wochen und Monate wiederfinden koennen
- historische Entscheidungen, Vorlieben und Referenzen wiederverwenden koennen
- externe Quellen integrieren koennen, ohne deren Rohdaten zu duplizieren
- sicher entscheiden koennen, welche Retrieval-Sicht ein Client bekommt

Erwarteter Produktwert:

- deutlich weniger Wiederholungen im Agent-Dialog
- bessere rueckbezogene Antworten
- nachvollziehbarere Retrieval-Ergebnisse durch Quellen und Debug-Scores

## 5. Aktueller Produktumfang auf `main`

### 5.1 Kernfunktionalitaet


| Bereich                                       | Beschreibung                                       | Status     |
| --------------------------------------------- | -------------------------------------------------- | ---------- |
| `store_session`                               | Session/Text als vollwertige Memory speichern      | Verfuegbar |
| `store_external`                              | Externe Referenz pointer-first speichern           | Verfuegbar |
| `embed`                                       | Text mit aktivem Provider embedden                 | Verfuegbar |
| `retrieve_fractal`                            | Hybrid Retrieval mit Profilen und optionalem Debug | Verfuegbar |
| `chat/subconscious`                           | Retrieval-gestuetzte Antwort mit Quellen           | Verfuegbar |
| `dream/status`, `events`, `governance/policy` | Operator-Sicht und Steuerung                       | Verfuegbar |


### 5.2 Auth und Rollen


| Modus                | Beschreibung                                                     | Status     |
| -------------------- | ---------------------------------------------------------------- | ---------- |
| Statischer Admin-Key | `KNOWWHERE_API_KEY` als Bearer-Token                             | Verfuegbar |
| Self-Service User    | `/register`, `/login`, `/refresh` mit PostgreSQL                 | Beta       |
| Capability-Endpoint  | `GET /auth/me` liefert Token-Art plus erlaubte Retrieval-Profile | Verfuegbar |


### 5.3 Retrieval-Profile


| Profil          | Ziel                              | Aktueller Zugriff |
| --------------- | --------------------------------- | ----------------- |
| `user-facing`   | sichere, konsumierbare Ergebnisse | Admin + User      |
| `agent-debug`   | Debug-Sicht mit Score-Einblicken  | nur Admin         |
| `full-fidelity` | rohe, maximale Sicht              | nur Admin         |


### 5.4 Bedienoberflaechen


| UI           | Zweck                                                                 | Status         |
| ------------ | --------------------------------------------------------------------- | -------------- |
| `dashboard/` | React/Vite Operator-Dashboard fuer Overview, Search, Chat, Governance | Beta           |
| `frontend/`  | minimales statisches Fallback aus dem Backend                         | eingeschraenkt |
| Swagger UI   | API-Referenz und manuelle Tests                                       | Verfuegbar     |


## 6. Datenmodell

KnowWhere unterscheidet zwei Memory-Typen:

1. **Session-Nodes**
  - `content: Option<String>` enthaelt den Volltext
  - fuer Konversationen, Notizen, Entscheidungen, Zusammenfassungen
2. **External-Nodes**
  - `original_pointer: Option<String>` enthaelt URI, Pfad oder Referenz
  - fuer Kameras, Sensoren, Dokumente, Dateisysteme und andere externe Systeme

Gemeinsame Felder:

- Embedding-Vektor
- Metadaten
- Gewichtung, Sensitivitaet, Status
- Provenance und Relations
- Zeitstempel

Die Vektordimension ist **modellabhaengig**, nicht fest. Standard lokal ist `nomic-embed-text-v2-moe` mit `768`, alternative Ollama-Modelle koennen andere Dimensionen haben.

## 7. Retrieval-Ansatz

KnowWhere liefert heute produktiv:

1. **Vector Search** fuer semantische Naehe
2. **BM25** fuer Begriffs- und Keyword-Matches
3. **Reciprocal Rank Fusion** zur Zusammenfuehrung
4. **Profilbasierte Gewichtung** je nach Retrieval-Profil und Trust-Tier
5. **Optionales Score-Debugging** fuer Operatoren und Agent-Debug

Dadurch ist das System nicht nur "semantisch aehnlich", sondern auch steuerbar und nachvollziehbar.

## 8. Storage- und Betriebsmodi

### 8.1 Default-Modus

- `MemoryStore`
- JSON-basierte Persistenz im Datenverzeichnis
- gut fuer Entwicklung, lokale Tests und Single-Node-Szenarien

### 8.2 PostgreSQL-Modus

Aktiv, wenn:

- mit `postgres-storage` gebaut wurde
- ein funktionierendes `DATABASE_URL` vorhanden ist

Zusatznutzen in diesem Modus:

- Self-Service User-Auth
- Retrieval-Analytik und Trajektorien
- Energy- und Lifecycle-Operationen
- Deduplication und Conflict Management
- Self-healing, Namespaces, Skills

## 9. Integrationen

### 9.1 OpenClaw

KnowWhere kann ueber das Plugin in OpenClaw eingebunden werden und den Memory-Loop uebernehmen:

- Nachrichten speichern
- historischen Kontext abrufen
- Kontext vor Prompt-Build injizieren

### 9.2 Python SDK

Ein Python-SDK ist vorhanden und erlaubt die direkte Integration in eigene Agent- oder Tooling-Workflows.

### 9.3 Weitere Host-Systeme

Langfristig ist KnowWhere als zusaetzliche Memory-Schicht fuer weitere Agent-Systeme gedacht, aber die Discovery- und Import-Ergonomie ist noch nicht fertig produktisiert.

## 10. Nicht-Ziele im aktuellen Stand

Aktuell bewusst **nicht** Produktziel auf `main`:

- vollstaendige Multi-Tenant-SaaS-Plattform
- vollautomatische Migration zwischen allen Storages
- flaechendeckende UI fuer jede Backend-Operation
- Hot-Swap zwischen Embedding-Providern ohne Neustart
- automatisches Hard-Delete von Memories

## 11. Tech-Stack


| Komponente           | Technologie                                                           | Status               |
| -------------------- | --------------------------------------------------------------------- | -------------------- |
| Backend              | Rust 1.85+, Axum 0.8, Tokio, Tower                                    | produktiv genutzt    |
| Lokale Embeddings    | Ollama                                                                | Standardpfad         |
| Cloud-Embeddings     | OpenAI, Grok/xAI                                                      | optional per Feature |
| Retrieval            | USearch + BM25 + RRF                                                  | produktiv genutzt    |
| Persistenz default   | JSON State                                                            | produktiv genutzt    |
| Persistenz erweitert | PostgreSQL/pgvector                                                   | Beta                 |
| Dashboard            | React + Vite                                                          | Beta                 |
| API-Dokumentation    | utoipa + Swagger UI                                                   | produktiv genutzt    |
| CI                   | GitHub Actions fuer Rust, Postgres, Feature Matrix, Dashboard, Docker | aktiv                |


## 12. Roadmap

### Kurzfristig

- Dokumentation vollstaendig am echten `main`-Stand halten
- Dashboard naeher an Backend-Routen bringen
- PostgreSQL-Auth und Lifecycle-Funktionen weiter haerten
- Import- und Migrationspfade klarer machen

### Mittelfristig

- bessere Discovery und strukturierter Host-Import
- staerkere Operator- und Debug-Werkzeuge fuer Retrieval-Qualitaet
- konsistentere Mehrnutzer-Geschichte

### Langfristig

- skalierbarere Storage- und Graph-Backends
- reifere Integrationen fuer weitere Frameworks
- schlankere Produktions- und Release-Story

## 13. Integrationsregeln

Wenn KnowWhere in ein bestehendes Agent-System eingebunden wird, gilt:

1. keine bestehenden Memories loeschen oder ueberschreiben
2. vorhandenes Wissen zuerst importieren
3. Host-Konfiguration nur ergaenzen, nie ersetzen
4. Host-Memory-System parallel weiterlaufen lassen
5. bei Ausfall von KnowWhere muss der Host degradiert, aber weiter funktionsfaehig sein

## 14. Risiken und Gegenmassnahmen


| Risiko                                         | Gegenmassnahme                                                                                |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Falscher Provider oder falsche Vektordimension | `KNOWWHERE_EMBEDDING_PROVIDER`, `OLLAMA_MODEL`, `OLLAMA_EMBEDDING_DIMENSION` explizit setzen  |
| Rechteverwirrung im Client                     | `GET /auth/me` als Quelle fuer Capabilities nutzen                                            |
| Reverse-Proxy-Fehlkonfiguration                | `RATE_LIMIT_MODE=proxy` nur hinter echtem Proxy aktivieren                                    |
| Zu grosse Erwartungen an das UI                | React-Dashboard klar als Beta und `frontend/` klar als Fallback dokumentieren                 |
| Datenverlust im lokalen Default-Modus          | PostgreSQL-Modus oder persistentes Datenverzeichnis fuer produktionsnaehere Umgebungen nutzen |

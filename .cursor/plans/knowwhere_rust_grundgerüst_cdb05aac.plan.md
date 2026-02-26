---
name: KnowWhere Rust Grundgerüst
overview: Komplettes Rust/Axum Grundgerüst für das KnowWhere Memory-System erstellen, inklusive Cargo-Workspace, FractalNode-Struct, API-Endpoints und in-memory Storage -- exakt nach PRD.md, ARCHITECTURE.md und den Projektregeln.
todos:
  - id: cargo-toml
    content: Cargo.toml mit allen Dependencies erstellen (axum 0.8, tokio, tower, tracing, serde, uuid, anyhow, chrono, usearch)
    status: completed
  - id: folder-structure
    content: "Ordnerstruktur anlegen: src/api/, src/memory/, src/embedding/, src/storage/, docs/, sdk/python/"
    status: completed
  - id: fractal-node
    content: src/memory/fractal_node.rs mit FractalNode + Relation Struct, new_session() und new_external() Konstruktoren
    status: completed
  - id: memory-mod
    content: src/memory/mod.rs mit Re-Exports
    status: completed
  - id: in-memory-store
    content: src/storage/in_memory.rs mit Arc<RwLock<HashMap>> basiertem MemoryStore
    status: completed
  - id: storage-mod
    content: src/storage/mod.rs mit Re-Exports
    status: completed
  - id: api-routes
    content: src/api/routes.rs mit health, store_session, store_external Endpoints + Request/Response Structs
    status: completed
  - id: api-mod
    content: src/api/mod.rs mit Re-Exports
    status: completed
  - id: embedding-mod
    content: src/embedding/mod.rs als Placeholder
    status: completed
  - id: main-rs
    content: src/main.rs mit Axum Server, Tracing-Init, Router-Setup und State-Injection
    status: completed
  - id: move-docs
    content: PRD.md und ARCHITECTURE.md nach docs/ verschieben
    status: completed
  - id: cargo-check
    content: cargo check und cargo build ausfuehren, Fehler anzeigen und fixen
    status: completed
isProject: false
---

# KnowWhere Rust-Grundgerüst

## Ausgangslage

Aktuell existieren nur 3 Dateien im Workspace:

- `ARCHITECTURE.md` -- definiert Ordnerstruktur und High-Level Design
- `PRD.md` -- definiert FractalNode-Struct, 4 Kern-Operationen, Tech-Stack
- `.cursor/rules/knowwhere.mdc` -- Coding Standards und Constraints

## Ziel-Ordnerstruktur

```
knowwhere/
  Cargo.toml                    (Workspace root / Binary Crate)
  src/
    main.rs                     (Axum Server, Router, Tracing-Setup)
    api/
      mod.rs                    (API-Modul re-exports)
      routes.rs                 (POST /store_session, POST /store_external, GET /health)
    memory/
      mod.rs                    (Memory-Modul re-exports)
      fractal_node.rs           (FractalNode + Relation Structs)
    embedding/
      mod.rs                    (Placeholder fuer spaetere Embedding-Provider)
    storage/
      mod.rs                    (Storage-Modul re-exports)
      in_memory.rs              (Arc<RwLock<HashMap>> basierter MVP-Store)
  docs/                         (bestehende Docs hierher verschieben)
    PRD.md
    ARCHITECTURE.md
  sdk/python/                   (leerer Placeholder)
```

## 1. Cargo.toml (Binary Crate "knowwhere-server")

Dependencies laut Rules (Axum 0.8, Tokio, Tower, etc.):

```toml
[package]
name = "knowwhere-server"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
usearch = "2.23"
```

## 2. FractalNode Struct (`src/memory/fractal_node.rs`)

Exakt nach PRD.md Zeile 41-52, mit serde Derive und Validierung:

```rust
pub struct FractalNode {
    pub id: Uuid,
    pub vector: Vec<f32>,
    pub content: Option<String>,            // Nur bei Sessions
    pub original_pointer: Option<String>,   // Bei externen Daten
    pub metadata: HashMap<String, Value>,
    pub weight: f64,
    pub children: Vec<FractalNode>,
    pub relations: Vec<Relation>,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}
```

Plus `Relation`-Struct und Konstruktor-Methoden `new_session()` / `new_external()` die die Pointer-First-Regel erzwingen.

## 3. In-Memory Storage (`src/storage/in_memory.rs`)

- `MemoryStore` struct mit `Arc<RwLock<HashMap<Uuid, FractalNode>>>`
- Methoden: `insert()`, `get()`, `list_all()` -- alle async
- Kein globaler State ohne Arc/Mutex (laut Rules)
- Spaeter ersetzbar durch LanceDB/USearch

## 4. API Routes (`src/api/routes.rs`)

Drei Endpoints:


| Route             | Methode | Funktion                                                                      |
| ----------------- | ------- | ----------------------------------------------------------------------------- |
| `/health`         | GET     | Liefert `{"status": "ok"}`                                                    |
| `/store_session`  | POST    | Nimmt `content` + `metadata`, erstellt FractalNode mit vollem Inhalt          |
| `/store_external` | POST    | Nimmt `pointer` + `embedding` + `metadata`, erstellt FractalNode OHNE content |


Request/Response-Structs mit serde, Axum State-Extraktor fuer den MemoryStore.

## 5. Main Entry Point (`src/main.rs`)

- Tracing-Subscriber initialisieren
- `MemoryStore` erstellen und als Axum State teilen
- Router mit allen Routes + Tower-Middleware (trace layer)
- Server auf `0.0.0.0:3000` starten

## 6. Docs verschieben

`PRD.md` und `ARCHITECTURE.md` nach `docs/` verschieben, damit die Struktur der ARCHITECTURE.md entspricht.

## 7. Build-Validierung

Nach dem Erstellen: `cargo check` und `cargo build` ausfuehren, Fehler zeigen und fixen.

## Abgrenzung (NICHT in diesem Schritt)

- Kein USearch-Index-Code (nur als Dependency vorhanden)
- Keine Embedding-Provider-Implementierung (nur Modul-Placeholder)
- Kein Python SDK (nur leeres Verzeichnis)
- Keine OpenAPI-Spec-Generierung (kommt in Phase 1)
- Keine Tests (kommen direkt nach dem Grundgeruest)


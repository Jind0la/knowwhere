---
name: Woche 0 Abschluss Tests
overview: Unit-Tests für FractalNode und MemoryStore erstellen, einen GET /retrieve/{id} Endpoint hinzufügen, Tracing-Logs ergänzen, und alles mit cargo test + manuellen curl-Tests verifizieren.
todos:
  - id: tests
    content: src/memory/tests.rs erstellen mit 4 Unit-Tests (new_session, new_external, store, retrieve)
    status: completed
  - id: memory-mod
    content: "src/memory/mod.rs erweitern: #[cfg(test)] mod tests"
    status: completed
  - id: retrieve-endpoint
    content: "src/api/routes.rs: GET /retrieve/{id} Endpoint + Tracing-Logs"
    status: completed
  - id: main-route
    content: "src/main.rs: /retrieve/{id} Route registrieren"
    status: completed
  - id: cargo-check
    content: cargo check + cargo test ausfuehren
    status: completed
  - id: manual-test
    content: Server starten, curl POST + GET testen
    status: completed
  - id: final-status
    content: "Bei gruenen Tests: Woche 0 ABSCHLUSS + Woche 1 START BEREIT"
    status: completed
isProject: false
---

# Woche 0 Abschluss: Tests, Retrieve-Endpoint, Tracing

## Aktueller Stand

Das Projekt hat bereits:

- `FractalNode` mit `new_session()` / `new_external()` in `[src/memory/fractal_node.rs](src/memory/fractal_node.rs)`
- `MemoryStore` (Arc/RwLock/HashMap) in `[src/storage/in_memory.rs](src/storage/in_memory.rs)`
- API-Routen `health`, `store_session`, `store_external` in `[src/api/routes.rs](src/api/routes.rs)`
- Server-Setup in `[src/main.rs](src/main.rs)` auf Port 3000

Es fehlt: Tests, Retrieve-Endpoint, Tracing-Logs in den Store-Funktionen.

---

## Task 1: Unit-Tests in `src/memory/tests.rs`

Neue Datei `[src/memory/tests.rs](src/memory/tests.rs)` mit `#[cfg(test)]` Modul und 4 Tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStore;
    use std::collections::HashMap;
}
```

**Test 1 – `new_session_has_content_no_pointer`**: Erstellt Node via `FractalNode::new_session()`, asserted `content.is_some()` und `original_pointer.is_none()`.

**Test 2 – `new_external_has_pointer_no_content`**: Erstellt Node via `FractalNode::new_external()`, asserted `original_pointer.is_some()` und `content.is_none()`.

**Test 3 – `store_session_and_external_via_memory_store`**: Erstellt MemoryStore, inserted je einen Session- und External-Node, asserted `count() == 2`.

**Test 4 – `retrieve_node_by_id`**: Inserted einen Node, holt ihn per `store.get(&id)`, asserted dass der zurückgegebene Node die korrekte ID hat.

Ausserdem: `[src/memory/mod.rs](src/memory/mod.rs)` erweitern um `mod tests;` unter `#[cfg(test)]`.

---

## Task 2: Retrieve-Endpoint + Tracing in `src/api/routes.rs`

In `[src/api/routes.rs](src/api/routes.rs)`:

- **Neuer Endpoint `retrieve`**: Nimmt `Path(id): Path<Uuid>` entgegen, ruft `store.get(&id)` auf, gibt den `FractalNode` als JSON zurück oder 404.
- **Tracing-Logs**: `tracing::info!()` in `store_session` und `store_external` hinzufügen, die Node-ID und Typ loggen.

Neuer Import: `axum::extract::Path`

```rust
pub async fn retrieve(
    State(store): State<MemoryStore>,
    Path(id): Path<Uuid>,
) -> Result<Json<FractalNode>, (StatusCode, String)> {
    tracing::info!(%id, "retrieving node");
    match store.get(&id).await {
        Ok(Some(node)) => Ok(Json(node)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("node {id} not found"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
```

---

## Task 3: Route in `src/main.rs` registrieren

In `[src/main.rs](src/main.rs)` die neue Route hinzufuegen:

```rust
.route("/retrieve/{id}", get(routes::retrieve))
```

---

## Task 4: Manueller Test

Server starten mit `cargo run`, dann:

```bash
curl -X POST http://localhost:3000/store_session \
  -H "Content-Type: application/json" \
  -d '{"content":"test session","vector":[0.1,0.2,0.3],"metadata":{}}'
```

Dann mit der zurueckgegebenen ID:

```bash
curl http://localhost:3000/retrieve/<id>
```

---

## Task 5: cargo test

`cargo test` ausfuehren. Wenn alle 4 Tests gruen sind: "Woche 0 ABSCHLUSS + Woche 1 START BEREIT".

---

## Dateiaenderungen (Zusammenfassung)

- **Neu**: `src/memory/tests.rs` (4 Unit-Tests)
- **Edit**: `src/memory/mod.rs` (+ `#[cfg(test)] mod tests`)
- **Edit**: `src/api/routes.rs` (+ `retrieve` Funktion, + `tracing::info!` in store_session/store_external)
- **Edit**: `src/main.rs` (+ `/retrieve/{id}` Route)
- Keine neuen Dependencies noetig


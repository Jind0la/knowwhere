---
name: Woche 5 Beta-Ready
overview: OpenAPI-Spec mit Swagger-UI, JWT-Auth-Middleware, Docker multi-stage Setup und Integration-Tests hinzufuegen, um KnowWhere beta-ready zu machen.
todos:
  - id: cargo-deps
    content: "Cargo.toml: utoipa, utoipa-swagger-ui, jsonwebtoken hinzufuegen"
    status: completed
  - id: utoipa-schemas
    content: Alle Typen (FractalNode, MultimodalData, DreamStatus, Request/Response) mit ToSchema annotieren
    status: completed
  - id: utoipa-paths
    content: "Alle 8 Route-Handler mit #[utoipa::path] annotieren"
    status: completed
  - id: docs-rs
    content: "src/api/docs.rs mit #[derive(OpenApi)] erstellen"
    status: completed
  - id: auth-middleware
    content: "src/api/auth.rs: JWT-Auth-Middleware mit KNOWWHERE_API_KEY"
    status: completed
  - id: main-rs
    content: "main.rs: Swagger-UI, CORS, Auth-Layer integrieren"
    status: completed
  - id: dockerfile
    content: Dockerfile (multi-stage, Rust 1.85) erstellen
    status: completed
  - id: docker-compose
    content: docker-compose.yml mit knowwhere-server + redis erstellen
    status: completed
  - id: readme
    content: "README.md erweitern: Docker, Env-Vars, Erste Schritte, Auth"
    status: completed
  - id: integration-tests
    content: "tests/integration.rs: Auth + Multimodal + Connector Tests"
    status: completed
  - id: validate
    content: cargo check + cargo test + docker compose up --build + Browser-Check
    status: completed
isProject: false
---

# Woche 5: OpenAPI + Auth + Docker + Beta-Ready

## Status Quo

- 8 API-Routen in `[src/api/routes.rs](src/api/routes.rs)`, keine OpenAPI-Dokumentation
- `tower-http` hat `cors` Feature, aber **kein CorsLayer** in `[src/main.rs](src/main.rs)` registriert
- Kein Auth-Code im Backend; Python SDK (`[sdk/python/knowwhere/client.py](sdk/python/knowwhere/client.py)`) sendet bereits optionalen `Bearer`-Token
- Keine Docker-Dateien, kein `tests/` Verzeichnis

---

## 1. Dependencies in Cargo.toml

Datei: `[Cargo.toml](Cargo.toml)`

Neue Dependencies hinzufuegen:

```toml
utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }
utoipa-swagger-ui = { version = "9", features = ["axum"] }
jsonwebtoken = "9"
```

`tower-http` benoetigt keine Aenderung (cors Feature bereits aktiv).

---

## 2. OpenAPI-Spec: src/api/docs.rs

Neue Datei: `src/api/docs.rs`

- `#[derive(OpenApi)]` Struct mit `#[openapi(paths(...), components(schemas(...)))]`
- Alle 8 Routen referenzieren: health, embed_text, store_session, store_external, retrieve, retrieve_fractal, recent_nodes, dream_status
- Alle Request/Response-Typen als Schemas: `HealthResponse`, `EmbedRequest`, `EmbedResponse`, `StoreSessionRequest`, `StoreExternalRequest`, `StoreNodeResponse`, `RetrieveFractalRequest`, `RecentQuery`, `FractalNode`, `Relation`, `MultimodalData`, `DreamStatus`

Dazu muessen die Typen in `routes.rs`, `fractal_node.rs`, `multimodal.rs` und `dream.rs` mit `#[derive(utoipa::ToSchema)]` und die Handler mit `#[utoipa::path(...)]` annotiert werden.

---

## 3. Swagger-UI + CORS in main.rs

Datei: `[src/main.rs](src/main.rs)`

Aenderungen:

```rust
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

// In der Router-Konfiguration:
let app = Router::new()
    // ... bestehende Routen ...
    .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
    .fallback_service(ServeDir::new("frontend"))
    .layer(CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any))
    .layer(TraceLayer::new_for_http())
    .with_state(state);
```

`src/api/mod.rs` erweitern: `pub mod docs;`

---

## 4. JWT-Auth Middleware

Neue Datei: `src/api/auth.rs`

**Konzept:**

- ENV-Variable `KNOWWHERE_API_KEY` wird beim Start gelesen
- Wenn gesetzt: alle Routen ausser `/health` und `/swagger-ui` sind geschuetzt
- Wenn nicht gesetzt: Auth deaktiviert (Entwicklungsmodus)
- Middleware prueft `Authorization: Bearer <token>` Header
- Token wird gegen `KNOWWHERE_API_KEY` als HMAC-Secret validiert (jsonwebtoken)
- Ein Startup-Endpoint oder CLI-Hinweis zeigt wie man ein JWT generiert

**Implementierung:**

- Axum `middleware::from_fn_with_state` fuer Auth-Layer
- Auth-State enthaelt `Option<String>` (api_key), wenn `None` -> kein Auth
- Der API-Key selbst wird als gueltigter Bearer-Token akzeptiert (einfacher Modus fuer MVP)
- Alternativ: API-Key -> JWT Konvertierung via `/auth/token` Endpoint

**Route-Schutz in main.rs:**

```rust
let protected = Router::new()
    .route("/embed", post(routes::embed_text))
    .route("/store_session", post(routes::store_session))
    // ... alle geschuetzten Routen
    .layer(middleware::from_fn_with_state(auth_state, auth_middleware));

let public = Router::new()
    .route("/health", get(routes::health));

let app = public.merge(protected)
    .merge(SwaggerUi::new("/swagger-ui")...)
    // ...
```

---

## 5. Docker-Setup

### Dockerfile (Multi-Stage, Rust 1.85)

```dockerfile
# Stage 1: Build
FROM rust:1.85 AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY frontend/ frontend/
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/knowwhere-server /usr/local/bin/
COPY --from=builder /app/frontend /app/frontend
WORKDIR /app
ENV RUST_LOG=info
EXPOSE 3000
CMD ["knowwhere-server"]
```

### docker-compose.yml

```yaml
services:
  knowwhere:
    build: .
    ports:
      - "3000:3000"
    environment:
      - RUST_LOG=info
      - KNOWWHERE_API_KEY=${KNOWWHERE_API_KEY:-}
    depends_on:
      - redis

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
```

---

## 6. README.md erweitern

Datei: `[README.md](README.md)`

Neue Sektionen:

- **Docker Installation**: `docker compose up --build`
- **Umgebungsvariablen**: Tabelle mit KNOWWHERE_API_KEY, GROK_API_KEY, OPENAI_API_KEY, RUST_LOG
- **Erste Schritte**: Dashboard oeffnen, SDK installieren, LangChain-Beispiel
- **API-Dokumentation**: Link zu `/swagger-ui`
- **Auth**: Wie man den API-Key setzt und Token verwendet

---

## 7. Integration-Tests: tests/integration.rs

Neue Datei: `tests/integration.rs`

Tests mit eigenem Axum-TestServer (axum::test):

- Health-Check ohne Auth (muss 200 liefern)
- Geschuetzte Route ohne Token (muss 401 liefern)
- Geschuetzte Route mit gueltigem Token (muss 200 liefern)
- Store Session + Retrieve Roundtrip
- Store External mit Multimodal-Daten
- Fractal Retrieve
- Dream Status

Benoetigt `axum-test` oder `tower::ServiceExt` fuer In-Process-Tests. Da wir keine externe Dependency hinzufuegen wollen, nutzen wir `tower::ServiceExt` + `hyper` (bereits transitiv vorhanden).

---

## 8. Validierung

Nach Abschluss aller Schritte:

1. `cargo check` -- kompiliert fehlerfrei
2. `cargo test` -- alle Tests gruen
3. `docker compose up --build` -- Container starten
4. Browser: `http://localhost:3000/swagger-ui` -- Swagger-UI sichtbar
5. Browser: `http://localhost:3000` -- Dashboard sichtbar
6. Ausgabe: "WOCHE 5 ABGESCHLOSSEN -- OpenAPI + Auth + Docker + Beta-Ready"


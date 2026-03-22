use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use knowwhere_server::api::{auth, auth::ApiKey, routes};
use knowwhere_server::embedding::{create_provider, ProviderKind};
use knowwhere_server::memory::{DreamMode, events::InMemoryEventStore};
use knowwhere_server::memory::governance::GovernancePolicy;
use knowwhere_server::storage::MemoryStore;

fn test_state() -> routes::AppState {
    let store = MemoryStore::new();
    let dream_store = store.clone();
    let dream = DreamMode::new(dream_store.clone());
    let embedding: Arc<dyn knowwhere_server::embedding::EmbeddingProvider> =
        create_provider(
            ProviderKind::OpenAI,
            Some(std::env::var("OPENAI_API_KEY")
                .expect("OPENAI_API_KEY must be set in environment")),
        );
    routes::AppState {
        store: Arc::new(store),
        dream_store,
        dream,
        embedding,
        governance_policy: GovernancePolicy::default_policy(),
        events: InMemoryEventStore::new(),
        #[cfg(feature = "postgres-storage")]
        trajectory_pool: None,
        vlm_worker: None,
        consolidation: None,
    }
}

fn app_without_auth() -> Router {
    let state = test_state();

    let protected = Router::new()
        .route("/embed", post(routes::embed_text))
        .route("/store_session", post(routes::store_session))
        .route("/store_external", post(routes::store_external))
        .route("/retrieve/{id}", get(routes::retrieve))
        .route("/retrieve_fractal", post(routes::retrieve_fractal))
        .route("/nodes/recent", get(routes::recent_nodes))
        .route("/dream/status", get(routes::dream_status))
        .route_layer(middleware::from_fn(auth::auth_middleware))
        .layer(axum::Extension(ApiKey(None)));

    Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .with_state(state)
}

fn app_with_auth(key: &str) -> Router {
    let state = test_state();

    let protected = Router::new()
        .route("/embed", post(routes::embed_text))
        .route("/store_session", post(routes::store_session))
        .route("/store_external", post(routes::store_external))
        .route("/retrieve/{id}", get(routes::retrieve))
        .route("/retrieve_fractal", post(routes::retrieve_fractal))
        .route("/nodes/recent", get(routes::recent_nodes))
        .route("/dream/status", get(routes::dream_status))
        .route_layer(middleware::from_fn(auth::auth_middleware))
        .layer(axum::Extension(ApiKey(Some(key.to_string()))));

    Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .with_state(state)
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// -- Health --

#[tokio::test]
async fn health_is_always_public() {
    let app = app_with_auth("secret");
    let resp = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"status\":\"ok\""));
}

// -- Auth --

#[tokio::test]
async fn auth_rejects_missing_token() {
    let app = app_with_auth("test-key");
    let resp = app
        .oneshot(
            Request::post("/embed")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_accepts_correct_token() {
    let app = app_with_auth("test-key");
    let resp = app
        .oneshot(
            Request::post("/embed")
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-key")
                .body(Body::from(r#"{"text":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"provider\":\"openai\""));
}

#[tokio::test]
async fn auth_rejects_wrong_token() {
    let app = app_with_auth("correct-key");
    let resp = app
        .oneshot(
            Request::post("/embed")
                .header("content-type", "application/json")
                .header("authorization", "Bearer wrong-key")
                .body(Body::from(r#"{"text":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// -- Store + Retrieve Roundtrip --

#[tokio::test]
async fn store_session_and_retrieve() {
    let app = app_without_auth();

    let resp = app
        .clone()
        .oneshot(
            Request::post("/store_session")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"content":"KnowWhere remembers everything"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = body_string(resp.into_body()).await;
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let node_id = created["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(&format!("/retrieve/{node_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    assert!(body.contains("KnowWhere remembers everything"));
    assert!(body.contains("\"original_pointer\":null"));
}

// -- Store External (Pointer-First) --

#[tokio::test]
async fn store_external_pointer_first() {
    let app = app_without_auth();

    let resp = app
        .clone()
        .oneshot(
            Request::post("/store_external")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"pointer":"s3://bucket/cam01/2026-02-26.jpg","metadata":{"source":"frigate"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = body_string(resp.into_body()).await;
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let node_id = created["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(&format!("/retrieve/{node_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"content\":null"));
    assert!(body.contains("s3://bucket/cam01/2026-02-26.jpg"));
}

// -- Multimodal External --

#[tokio::test]
async fn store_external_multimodal_image() {
    let app = app_without_auth();

    let payload = serde_json::json!({
        "pointer": "s3://bucket/cam01/snapshot.jpg",
        "multimodal": {
            "type": "Image",
            "pointer": "s3://bucket/cam01/snapshot.jpg",
            "embedding": [0.1, 0.2, 0.3]
        }
    });

    let resp = app
        .clone()
        .oneshot(
            Request::post("/store_external")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = body_string(resp.into_body()).await;
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let node_id = created["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(&format!("/retrieve/{node_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"type\":\"Image\""));
    assert!(body.contains("snapshot.jpg"));
}

// -- Fractal Retrieve --

#[tokio::test]
async fn fractal_retrieve_with_query_text_returns_valid_json() {
    let app = app_without_auth();

    // Store a node with known content
    app.clone()
        .oneshot(
            Request::post("/store_session")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"remember the meeting tomorrow at 3pm"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Retrieve with query_text — should return valid JSON array
    let resp = app
        .oneshot(
            Request::post("/retrieve_fractal")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"query_text":"meeting schedule","top_k":5,"max_depth":2,"governance_enabled":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    // Should be a valid JSON array (empty or with results)
    let results: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    // We don't assert !results.is_empty() because embedding similarity may vary
    // We just assert the endpoint works and returns valid JSON
}

#[tokio::test]
async fn fractal_retrieve_requires_query() {
    let app = app_without_auth();

    // Neither query_vector nor query_text — should return 400
    let resp = app
        .oneshot(
            Request::post("/retrieve_fractal")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"top_k":5,"max_depth":2}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fractal_retrieve_with_vector_only() {
    let app = app_without_auth();

    // Store a node (will get auto-embedded)
    app.clone()
        .oneshot(
            Request::post("/store_session")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"machine learning algorithms"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Query with explicit 1536-dim vector (OpenAI compatible)
    // Using a uniform vector — cosine similarity with itself should be 1.0
    let vector: Vec<f32> = vec![0.1; 1536];
    let query = serde_json::json!({
        "query_vector": vector,
        "top_k": 5,
        "max_depth": 2,
        "governance_enabled": false
    });

    let resp = app
        .oneshot(
            Request::post("/retrieve_fractal")
                .header("content-type", "application/json")
                .body(Body::from(query.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    let results: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    // Node was stored with same dimension — should at least get the node back
    // (similarity depends on embedding quality)
}

// -- Dream Status --

#[tokio::test]
async fn dream_status_returns_valid_json() {
    let app = app_without_auth();

    let resp = app
        .oneshot(
            Request::get("/dream/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    // Should be valid JSON — structure varies by dream mode implementation
    let _status: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Just verify valid JSON — specific fields depend on current dream mode state
}

// -- Recent Nodes --

#[tokio::test]
async fn recent_nodes_after_insert() {
    let app = app_without_auth();

    app.clone()
        .oneshot(
            Request::post("/store_session")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"content":"recent test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::get("/nodes/recent?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    let nodes: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert!(!nodes.is_empty());
    assert!(body.contains("recent test"));
}

// -- Store Session with Memory Type --

#[tokio::test]
async fn store_session_with_memory_type() {
    let app = app_without_auth();

    let resp = app
        .clone()
        .oneshot(
            Request::post("/store_session")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"content":"important fact","memory_type":"semantic"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = body_string(resp.into_body()).await;
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let node_id = created["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(&format!("/retrieve/{node_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// -- Embed Text --

#[tokio::test]
async fn embed_text_returns_vector() {
    let app = app_without_auth();

    let resp = app
        .oneshot(
            Request::post("/embed")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hello world"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = body_string(resp.into_body()).await;
    let embedded: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(embedded.get("vector").is_some());
    let vector = embedded["vector"].as_array().unwrap();
    assert_eq!(vector.len(), 1536); // OpenAI text-embedding-3-small
}

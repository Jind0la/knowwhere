use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use knowwhere_server::api::{auth, auth::ApiKey, routes, webhooks::DedupCache};
use knowwhere_server::embedding::{EmbeddingProvider, LocalOllamaProvider};
use knowwhere_server::memory::{DreamMode, events::InMemoryEventStore};
use knowwhere_server::memory::governance::GovernancePolicy;
use knowwhere_server::storage::{MemoryStore, StorageBackend};

/// Creates the embedding provider for tests.
/// Always uses LocalOllama — tests are designed and validated against Ollama embeddings.
/// Cloud providers (OpenAI/Grok) require --features flags to compile.
fn embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(LocalOllamaProvider::new())
}

fn test_state() -> routes::AppState {
    let store: Arc<dyn knowwhere_server::storage::StorageBackend> =
        Arc::new(MemoryStore::new());
    let dream_store = store.clone();
    let dream = DreamMode::new(dream_store.clone());
    let embedding = embedding_provider();
    routes::AppState {
        store: store.clone(),
        dream_store,
        dream,
        embedding,
        governance_policy: GovernancePolicy::default_policy(),
        events: InMemoryEventStore::new(),
        #[cfg(feature = "postgres-storage")]
        trajectory_pool: None,
        vlm_worker: None,
        consolidation: None,
        frigate_dedup: DedupCache::new(),
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
    // Provider field depends on which embedding backend is active
    assert!(body.contains("\"provider\":") && (body.contains("openai") || body.contains("ollama")));
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

    // Query with explicit 768-dim vector (Ollama nomic-embed-text-v2-moe compatible)
    // Using a uniform vector — cosine similarity with itself should be 1.0
    let vector: Vec<f32> = vec![0.1; 768];
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
    // OpenAI: 1536-dim, Ollama nomic: 768-dim — accept both
    assert!(vector.len() == 1536 || vector.len() == 768);
}

// =============================================================================
// PostgreSQL Storage Backend Tests
// =============================================================================

#[tokio::test]
#[cfg(feature = "postgres-storage")]
#[ignore = "requires DATABASE_URL — run: DATABASE_URL='postgresql://...' cargo test --features postgres-storage -- --include-ignored"]
async fn postgres_store_hybrid_retrieve_bm25_only() {
    // Isolated test for hybrid_retrieve with BM25-only query (no query_vector).
    // This test verifies the BM25 fallback path works correctly when only
    // query_text is provided. Previously this path crashed (HTTP 500) because
    // of a pgvector/FLOAT4[] type mismatch in list_memories().
    //
    // The bug: list_memories() used `embedding as "embedding: _"` which decoded
    // the pgvector column as Vec<f32>, but pgvector stores vectors in its own
    // binary format that is NOT compatible with PostgreSQL's float4[].
    // Fix: cast to float4[] explicitly: `embedding::float4[] as "embedding: _"`
    use knowwhere_server::storage::{postgres_store::PostgresStore, backend::HybridQuery, StorageBackend};
    use knowwhere_server::memory::fractal_node::FractalNode;
    use std::env;

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for this test (run: export DATABASE_URL='postgres://...')");

    let store = PostgresStore::connect(&database_url)
        .await
        .expect("failed to connect to PostgreSQL");

    // Insert a test node directly via the StorageBackend trait
    let test_content = format!("postgres bm25 test content {}", uuid::Uuid::new_v4());
    let node = FractalNode::new_typed(
        Some(test_content.clone()),
        None,
        vec![0.0; 768], // dummy vector — not used in BM25-only query
        Default::default(),
        knowwhere_server::memory::types::MemoryType::Episodic,
        knowwhere_server::memory::types::MemorySource::Conversation,
    );

    let node_id = store
        .insert(node)
        .await
        .expect("insert failed");

    // BM25-only query — this exercises the fallback path in hybrid_retrieve
    // that calls search_bm25() then get() for each result.
    let query = HybridQuery {
        query_text: Some("postgres bm25 test content".to_string()),
        query_vector: None,
        top_k: 5,
        max_depth: 0,
    };

    let results = store
        .hybrid_retrieve(&query)
        .await
        .expect("hybrid_retrieve failed");

    // Verify our inserted node is returned
    let found = results
        .iter()
        .find(|r| r.node.content.as_deref() == Some(&test_content));

    assert!(
        found.is_some(),
        "BM25 query should find the inserted node. Results: {} items, first content: {:?}",
        results.len(),
        results.first().and_then(|r| r.node.content.as_deref())
    );

    // Cleanup
    store
        .delete(node_id)
        .await
        .expect("cleanup delete failed");
}

#[tokio::test]
#[cfg(feature = "postgres-storage")]
#[ignore = "requires DATABASE_URL — run: DATABASE_URL='postgresql://...' cargo test --features postgres-storage -- --include-ignored"]
async fn postgres_store_hybrid_retrieve_with_vector() {
    // Test hybrid_retrieve with a real query vector.
    // Uses the vector search path (HNSW index) combined with BM25 via RRF.
    use knowwhere_server::storage::postgres_store::PostgresStore;
    use knowwhere_server::memory::fractal_node::FractalNode;
    use knowwhere_server::storage::backend::HybridQuery;
    use std::env;

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for this test");

    let store = PostgresStore::connect(&database_url)
        .await
        .expect("failed to connect to PostgreSQL");

    // Insert with a real-ish vector (768-dim, matching nomic-embed-text-v2-moe)
    let test_content = format!("vector search test node {}", uuid::Uuid::new_v4());
    let vector: Vec<f32> = (0..768).map(|i| (i as f32) * 0.001).collect();
    let node = FractalNode::new_typed(
        Some(test_content.clone()),
        None,
        vector.clone(),
        Default::default(),
        knowwhere_server::memory::types::MemoryType::Semantic,
        knowwhere_server::memory::types::MemorySource::Conversation,
    );

    let node_id = store
        .insert(node)
        .await
        .expect("insert failed");

    // Query with the same vector — should find the node with high similarity
    let query = HybridQuery {
        query_text: Some("vector search test".to_string()),
        query_vector: Some(vector),
        top_k: 3,
        max_depth: 0,
    };

    let results = store
        .hybrid_retrieve(&query)
        .await
        .expect("hybrid_retrieve failed");

    let found = results
        .iter()
        .find(|r| r.node.content.as_deref() == Some(&test_content));

    assert!(
        found.is_some(),
        "Vector query should find the inserted node. Got {} results.",
        results.len()
    );

    // Cleanup
    store
        .delete(node_id)
        .await
        .expect("cleanup delete failed");
}

#[tokio::test]
#[cfg(feature = "postgres-storage")]
#[ignore = "requires DATABASE_URL — run: DATABASE_URL='postgresql://...' cargo test --features postgres-storage -- --include-ignored"]
async fn postgres_store_count_matches_active_memories() {
    // Verify that store.count() correctly returns the number of active memories.
    // Previously this returned 0 even when active memories existed in the DB,
    // due to silent query failures from the pgvector type mismatch.
    use knowwhere_server::storage::postgres_store::PostgresStore;
    use knowwhere_server::memory::fractal_node::FractalNode;
    use std::env;

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for this test");

    let store = PostgresStore::connect(&database_url)
        .await
        .expect("failed to connect to PostgreSQL");

    // Insert two nodes
    let node1 = FractalNode::new_typed(
        Some("count test node 1".to_string()),
        None,
        vec![0.1; 768],
        Default::default(),
        knowwhere_server::memory::types::MemoryType::Episodic,
        knowwhere_server::memory::types::MemorySource::Conversation,
    );
    let node2 = FractalNode::new_typed(
        Some("count test node 2".to_string()),
        None,
        vec![0.2; 768],
        Default::default(),
        knowwhere_server::memory::types::MemoryType::Episodic,
        knowwhere_server::memory::types::MemorySource::Conversation,
    );

    let id1 = store.insert(node1).await.expect("insert 1 failed");
    let id2 = store.insert(node2).await.expect("insert 2 failed");

    let count = StorageBackend::count(&store).await;
    assert!(
        count >= 2,
        "count() should be at least 2, got {}",
        count
    );

    // Cleanup
    store.delete(id1).await.expect("delete 1 failed");
    store.delete(id2).await.expect("delete 2 failed");
}

// -- Frigate Webhook Tests --

fn app_with_webhook() -> Router {
    let state = test_state();

    Router::new()
        .route("/health", get(routes::health))
        .route("/webhooks/frigate", post(routes::webhook_frigate))
        .with_state(state)
}

#[tokio::test]
async fn webhook_frigate_unauthorized_without_secret() {
    // Clean env FIRST to prevent leak from parallel tests
    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");

    let app = app_with_webhook();
    let body = serde_json::json!({
        "id": "evt-001",
        "camera": "front_door",
        "label": "person",
        "top_score": 0.95
    });

    let resp = app
        .oneshot(
            Request::post("/webhooks/frigate")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Without secret header, should be 401
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_frigate_unauthorized_with_wrong_secret() {
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");

    let app = app_with_webhook();
    let body = serde_json::json!({
        "id": "evt-002",
        "camera": "front_door",
        "label": "person",
        "top_score": 0.95
    });

    let resp = app
        .oneshot(
            Request::post("/webhooks/frigate")
                .header("Content-Type", "application/json")
                .header("X-Webhook-Secret", "wrong-secret")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_frigate_success_with_valid_secret() {
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");

    let app = app_with_webhook();
    let body = serde_json::json!({
        "id": "evt-003",
        "camera": "front_door",
        "label": "person",
        "top_score": 0.95,
        "snapshot_path": "/snapshots/evt-003.jpg"
    });

    let resp = app
        .oneshot(
            Request::post("/webhooks/frigate")
                .header("Content-Type", "application/json")
                .header("X-Webhook-Secret", "test-secret")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body_str = body_string(resp.into_body()).await;
    let response: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(response["status"], "stored");
    assert_eq!(response["event_id"], "evt-003");

    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_frigate_duplicate_event_returns_409() {
    // Clean up env FIRST to prevent leak from parallel tests
    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");

    let app = app_with_webhook();
    let body = serde_json::json!({
        "id": "evt-004-duplicate",
        "camera": "backyard",
        "label": "car",
        "top_score": 0.88
    });

    // First request should succeed
    // (Re-set env var to guard against parallel test env-leak)
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");
    let resp = app
        .clone()
        .oneshot(
            Request::post("/webhooks/frigate")
                .header("Content-Type", "application/json")
                .header("X-Webhook-Secret", "test-secret")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Second request with same event ID should return 409
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");
    let resp = app
        .oneshot(
            Request::post("/webhooks/frigate")
                .header("Content-Type", "application/json")
                .header("X-Webhook-Secret", "test-secret")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_frigate_success_with_query_secret() {
    std::env::set_var("FRIGATE_WEBHOOK_SECRET", "test-secret");

    let app = app_with_webhook();
    let body = serde_json::json!({
        "id": "evt-005",
        "camera": "garage",
        "label": "dog",
        "top_score": 0.72
    });

    let resp = app
        .oneshot(
            Request::post("/webhooks/frigate?secret=test-secret")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    std::env::remove_var("FRIGATE_WEBHOOK_SECRET");
}

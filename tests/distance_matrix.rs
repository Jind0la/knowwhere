/// Unit tests for the Haversine distance formula.
/// RED phase: these tests will FAIL because haversine_distance() doesn't exist yet.
use knowwhere_server::api::distance_matrix::haversine_distance;

#[test]
fn haversine_same_point_is_zero() {
    // Berlin — Brandenburger Tor
    let d = haversine_distance(52.5163, 13.3777, 52.5163, 13.3777);
    assert!((d - 0.0).abs() < 1.0, "same point should be ~0 m, got {d}");
}

#[test]
fn haversine_berlin_to_munich() {
    // Brandenburger Tor → Marienplatz
    // Known distance: ~504 km (504,000 m)
    let d = haversine_distance(52.5163, 13.3777, 48.1375, 11.5754);
    assert!(
        d > 495_000.0 && d < 515_000.0,
        "Berlin→Munich should be ~504 km, got {d} m ({:.1} km)",
        d / 1000.0
    );
}

#[test]
fn haversine_antipodes() {
    // Antipodes: 0,0 → 0,180 (half Earth circumference = ~20015 km)
    let d = haversine_distance(0.0, 0.0, 0.0, 180.0);
    // Half circum. = PI * 6371000 = ~20,015,000
    assert!(
        d > 20_000_000.0 && d < 20_040_000.0,
        "half circumference should be ~20 Mm, got {d} m"
    );
}

#[test]
fn haversine_nyc_to_london() {
    // NYC (40.7128, -74.0060) → London (51.5074, -0.1278)
    // Known distance: ~5570 km
    let d = haversine_distance(40.7128, -74.0060, 51.5074, -0.1278);
    assert!(
        d > 5_500_000.0 && d < 5_650_000.0,
        "NYC→London should be ~5570 km, got {d} m ({:.1} km)",
        d / 1000.0
    );
}

#[test]
fn haversine_tokyo_to_sydney() {
    // Tokyo (35.6762, 139.6503) → Sydney (-33.8688, 151.2093)
    // Known distance: ~7800 km
    let d = haversine_distance(35.6762, 139.6503, -33.8688, 151.2093);
    assert!(
        d > 7_700_000.0 && d < 7_950_000.0,
        "Tokyo→Sydney should be ~7800 km, got {d} m ({:.1} km)",
        d / 1000.0
    );
}

#[test]
fn haversine_close_points_hundreds_of_meters() {
    // Two points ~500m apart in Berlin
    let d1 = haversine_distance(52.5163, 13.3777, 52.5200, 13.3777);
    // ~412 m north
    assert!(
        d1 > 300.0 && d1 < 600.0,
        "400m apart northwards, got {d1} m"
    );

    // Two points ~500m apart east-west
    let d2 = haversine_distance(52.5163, 13.3777, 52.5163, 13.3850);
    // ~496 m east at this latitude
    assert!(
        d2 > 400.0 && d2 < 700.0,
        "~500m apart eastwards, got {d2} m"
    );
}

// ── Integration tests for the /distance-matrix endpoint ──

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use http_body_util::BodyExt;
use knowwhere_server::api::routes::{self, distance_matrix};
#[cfg(feature = "webhooks")]
use knowwhere_server::api::webhooks::DedupCache;
use knowwhere_server::embedding::EmbeddingProvider;
use knowwhere_server::memory::governance::GovernancePolicy;
use knowwhere_server::memory::{events::InMemoryEventStore, DreamMode};
use knowwhere_server::storage::MemoryStore;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

struct FixedEmbeddingProvider {
    dim: usize,
}

#[async_trait::async_trait]
impl EmbeddingProvider for FixedEmbeddingProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![text.len() as f32; self.dim])
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "fixed-test"
    }
}

fn test_state() -> routes::AppState {
    let store: Arc<dyn knowwhere_server::storage::StorageBackend> = Arc::new(MemoryStore::new());
    let dream_store = store.clone();
    let dream = DreamMode::new(dream_store.clone());
    let embedding = Arc::new(FixedEmbeddingProvider { dim: 768 });
    routes::AppState {
        store: store.clone(),
        dream_store,
        dream,
        embedding,
        router: None,
        governance_policy: Arc::new(RwLock::new(GovernancePolicy::default_policy())),
        events: InMemoryEventStore::new(),
        #[cfg(feature = "postgres-storage")]
        trajectory_pool: None,
        #[cfg(feature = "webhooks")]
        frigate_dedup: DedupCache::new(),
        #[cfg(feature = "webhooks")]
        frigate_webhook_secret: std::env::var("FRIGATE_WEBHOOK_SECRET").ok(),
        #[cfg(feature = "webhooks")]
        homeassistant_dedup: DedupCache::new(),
        #[cfg(feature = "webhooks")]
        homeassistant_webhook_secret: std::env::var("HASS_WEBHOOK_SECRET").ok(),
        temporal_weight: Arc::new(RwLock::new(None)),
        default_source_type_weights: None,
        #[cfg(feature = "reranker")]
        reranker: None,
    }
}

fn app() -> Router {
    let state = test_state();
    Router::new()
        .route("/distance-matrix", post(distance_matrix))
        .with_state(state)
}

/// Helper: sends a POST to /distance-matrix and returns the parsed JSON response.
async fn send_matrix(app: &Router, body: &serde_json::Value) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/distance-matrix")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(body).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();
    (status, json)
}

#[tokio::test]
async fn distance_matrix_basic_2x2() {
    let app = app();

    let body = serde_json::json!({
        "origins": [
            {"lat": 52.5163, "lng": 13.3777},   // Berlin
            {"lat": 48.1375, "lng": 11.5754}    // Munich
        ],
        "destinations": [
            {"lat": 51.5074, "lng": -0.1278},   // London
            {"lat": 40.7128, "lng": -74.0060}   // NYC
        ]
    });

    let (status, json) = send_matrix(&app, &body).await;
    assert_eq!(status, StatusCode::OK);

    let distances = json["distances"].as_array().unwrap();
    assert_eq!(distances.len(), 2, "should have 2 rows (origins)");
    assert_eq!(
        distances[0].as_array().unwrap().len(),
        2,
        "each row should have 2 cols"
    );
    assert_eq!(distances[1].as_array().unwrap().len(), 2);

    // Berlin → London: ~930 km
    let berlin_to_london = distances[0][0].as_f64().unwrap();
    assert!(
        berlin_to_london > 900_000.0 && berlin_to_london < 960_000.0,
        "Berlin→London should be ~930 km, got {} km",
        berlin_to_london / 1000.0
    );

    // Munich → NYC: ~6500 km
    let munich_to_nyc = distances[1][1].as_f64().unwrap();
    assert!(
        munich_to_nyc > 6_400_000.0 && munich_to_nyc < 6_600_000.0,
        "Munich→NYC should be ~6500 km, got {} km",
        munich_to_nyc / 1000.0
    );

    // Symmetry check: London→Berlin should equal Berlin→London
    let rev_body = serde_json::json!({
        "origins": [{"lat": 51.5074, "lng": -0.1278}],
        "destinations": [{"lat": 52.5163, "lng": 13.3777}]
    });
    let (_status, rev_json) = send_matrix(&app, &rev_body).await;
    let rev_distance = rev_json["distances"][0][0].as_f64().unwrap();

    assert!(
        (berlin_to_london - rev_distance).abs() < 100.0,
        "Haversine should be symmetric: {} vs {}",
        berlin_to_london,
        rev_distance
    );
}

#[tokio::test]
async fn distance_matrix_same_point() {
    let app = app();

    let body = serde_json::json!({
        "origins": [{"lat": 48.0, "lng": 11.0}],
        "destinations": [{"lat": 48.0, "lng": 11.0}]
    });

    let (status, json) = send_matrix(&app, &body).await;
    assert_eq!(status, StatusCode::OK);

    let d = json["distances"][0][0].as_f64().unwrap();
    assert!(d < 1.0, "same point should be ~0 m, got {d}");
}

#[tokio::test]
async fn distance_matrix_empty_origins_is_400() {
    let app = app();

    let body = serde_json::json!({
        "origins": [],
        "destinations": [{"lat": 48.0, "lng": 11.0}]
    });

    let (status, _json) = send_matrix(&app, &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

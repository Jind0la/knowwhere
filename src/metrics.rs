//! Prometheus metrics for KnowWhere.
//!
//! Exposes a `/metrics` endpoint (Prometheus text format) and records every HTTP
//! request as a latency histogram, keyed by method + normalised path (via
//! axum's `MatchedPath` so we see `/retrieve/:id` instead of `/retrieve/uuid`).

use axum::{
    body::Body,
    extract::MatchedPath,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use metrics::{counter, describe_counter, describe_histogram, histogram};
use std::sync::OnceLock;
use std::time::Instant;

// ── Metric definitions ──────────────────────────────────────────────────────

const HISTOGRAM_NAME: &str = "knowwhere_http_requests_duration_seconds";
const COUNTER_NAME: &str = "knowwhere_http_requests_total";

/// Prometheus handle kept around for the `/metrics` endpoint.
/// `OnceLock` is safe — initialised once at startup, read concurrently.
static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

// ── Initialisation ──────────────────────────────────────────────────────────

/// Build the Prometheus recorder (must be called once during startup).
pub fn setup_metrics_recorder() {
    let buckets = vec![
        0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];
    let handle = PrometheusBuilder::new()
        .set_buckets(&buckets)
        .expect("valid histogram buckets")
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    describe_histogram!(
        HISTOGRAM_NAME,
        "HTTP request latency in seconds (method + normalised path)"
    );
    describe_counter!(
        COUNTER_NAME,
        "Total HTTP requests (method + normalised path + status code)"
    );

    METRICS_HANDLE
        .set(handle)
        .expect("metrics recorder already initialised");
}

// ── Axum handler for GET /metrics ───────────────────────────────────────────

pub async fn metrics_endpoint() -> Response {
    let handle = METRICS_HANDLE.get().expect("metrics recorder not initialised");
    let body = handle.render();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; version=0.0.4")
        .body(Body::from(body))
        .unwrap()
}

// ── Axum middleware (post-routing → MatchedPath available) ──────────────────

/// Records `method`, `path`, `status`, and `duration` for every HTTP request.
///
/// Uses `axum::middleware::from_fn` (not a Tower layer) so it runs AFTER routing
/// — therefore `MatchedPath` is available in request extensions.
pub async fn metrics_middleware(
    request: Request<Body>,
    next: Next,
) -> Response {
    // Skip the /metrics endpoint to avoid recursion.
    if request.uri().path() == "/metrics" {
        return next.run(request).await;
    }

    let start = Instant::now();
    let method = request.method().to_string();

    // MatchedPath is set by the router BEFORE axum middleware runs.
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unknown".to_string());

    let response = next.run(request).await;

    let status = response.status().as_u16().to_string();
    let duration = start.elapsed().as_secs_f64();

    let labels = [
        ("method", method),
        ("path", path),
        ("status", status),
    ];
    counter!(COUNTER_NAME, &labels).increment(1);
    histogram!(HISTOGRAM_NAME, &labels).record(duration);

    response
}

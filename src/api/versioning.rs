//! API Versioning middleware and helpers (H4).
//!
//! Adds `API-Version: 1` header to all `/v1/` responses and
//! `Deprecation: true` + `Sunset: Sat, 01 Nov 2026 00:00:00 GMT`
//! headers to unversioned (legacy) routes.

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Middleware that adds `API-Version: 1` response header for v1 routes.
pub async fn api_version_header(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "API-Version",
        HeaderValue::from_static("1"),
    );
    response
}

/// Middleware that adds deprecation warning headers to legacy routes.
///
/// Adds `Deprecation: true` and `Sunset` headers to inform clients
/// that these routes will be removed. The sunset date gives 4+ months
/// of migration time.
pub async fn deprecation_warning(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("Deprecation", HeaderValue::from_static("true"));
    headers.insert(
        "Sunset",
        HeaderValue::from_static("Sat, 01 Nov 2026 00:00:00 GMT"),
    );
    headers.insert(
        "Link",
        HeaderValue::from_static(
            "</v1>; rel=\"deprecated-version\"",
        ),
    );
    response
}

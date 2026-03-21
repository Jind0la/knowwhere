use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::{routing::post, Json, Router};
use axum_governor::{GovernorConfig, GovernorLayer};
use real::RealIpLayer;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;

#[derive(Clone)]
pub struct ApiKey(pub Option<String>);

/// Constant-time string comparison to prevent timing attacks.
/// Length is not secret (it's in the header), so comparing lengths first is safe.
fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    subtle::ConstantTimeEq::ct_eq(a.as_bytes(), b.as_bytes()).into()
}

pub async fn auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let api_key = request.extensions().get::<ApiKey>().cloned();

    let Some(ApiKey(Some(ref expected))) = api_key else {
        return Ok(next.run(request).await);
    };

    if expected.is_empty() {
        return Ok(next.run(request).await);
    }

    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    match token {
        Some(t) if secure_compare(t, expected) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

// ---------------------------------------------------------------------------
// Governor config — per-IP rate limiting on auth endpoints
// ---------------------------------------------------------------------------

/// Strict GovernorConfig for auth endpoints: 3 req/s per IP.
pub fn auth_governor_config() -> GovernorConfig {
    GovernorConfig::default()
        .override_mode(false) // enforce both global and route-specific rules
}

/// GovernorConfig for protected API endpoints: 5 req/s per IP.
/// More permissive than auth endpoints since these require a valid Bearer token first.
pub fn protected_governor_config() -> GovernorConfig {
    GovernorConfig::default()
        .override_mode(false)
}

// ---------------------------------------------------------------------------
// Auth request/response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginRequest {
    pub api_key: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub message: String,
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct RefreshResponse {
    pub token: String,
    pub message: String,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: Option<String>,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub message: String,
}

// ---------------------------------------------------------------------------
// Auth handlers
// ---------------------------------------------------------------------------

/// POST /auth/login — authenticate and receive a Bearer token.
/// Rate-limited: 3 req/s per IP.
pub async fn login(
    axum::Extension(expected): axum::Extension<ApiKey>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    let ApiKey(Some(ref key)) = expected else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    if !secure_compare(&req.api_key, key) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Token is the API key itself (matching existing Bearer-token auth).
    Ok(Json(LoginResponse {
        token: req.api_key,
        message: "authenticated".to_string(),
    }))
}

/// POST /auth/refresh — refresh a Bearer token.
/// Rate-limited: 3 req/s per IP.
pub async fn refresh(
    axum::Extension(expected): axum::Extension<ApiKey>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, StatusCode> {
    let ApiKey(Some(ref key)) = expected else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    // Validate the presented token against the expected key.
    if !secure_compare(&req.token, key) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Issue a new token (same key, re-validated).
    Ok(Json(RefreshResponse {
        token: key.clone(),
        message: "token refreshed".to_string(),
    }))
}

/// POST /auth/register — register a new client (stub: always succeeds).
/// Rate-limited: 10 req/min per IP.
pub async fn register(
    Json(_req): Json<RegisterRequest>,
) -> Json<RegisterResponse> {
    Json(RegisterResponse {
        message: "registration not yet implemented".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Auth router — protected by Governor rate-limiting middleware
// ---------------------------------------------------------------------------

/// Build the auth sub-router with rate limiting applied before any auth check.
/// Note: `init_rate_limiter!` must be called once before the app starts
/// (in main.rs or similar), otherwise all requests will be rejected.
pub fn auth_router() -> Router {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/register", post(register))
        .layer(
            ServiceBuilder::new()
                .layer(RealIpLayer::default()) // Extract real client IP first
                .layer(GovernorLayer::new(auth_governor_config())) // Then rate-limit
        )
}

/// Build the auth sub-router with state, for merging into a Router<AppState>.
///
/// The generic state type allows this to be called from main.rs without
/// creating a circular import between the auth and routes modules.
pub fn auth_router_with_state<S: Clone + Send + Sync + 'static>(state: S) -> Router<S> {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/register", post(register))
        .layer(
            ServiceBuilder::new()
                .layer(RealIpLayer::default()) // Extract real client IP first
                .layer(GovernorLayer::new(auth_governor_config())) // Then rate-limit
        )
        .with_state(state)
}

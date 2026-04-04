use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::{routing::post, Extension, Json, Router};
use axum_governor::{GovernorConfig, GovernorLayer};
use rand::Rng;
use real::RealIpLayer;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceBuilder;

#[derive(Clone)]
pub struct ApiKey(pub Option<String>);

/// Shared state for API key validation.
/// - `admin_key`: the static KNOWWHERE_API_KEY set at server startup (guards the admin).
/// - `registered_keys`: keys generated via /auth/register for beta testers.
#[derive(Clone)]
pub struct AuthState {
    pub admin_key: Arc<RwLock<Option<String>>>,
    pub registered_keys: Arc<RwLock<HashMap<String, ()>>>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self {
            admin_key: Arc::new(RwLock::new(None)),
            registered_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Generate a human-readable API key: `kw_<base62_random_32chars>`
fn generate_api_key() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let key: String = (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..CHARS.len());
            CHARS[idx] as char
        })
        .collect();
    format!("kw_{key}")
}

/// Constant-time string comparison to prevent timing attacks.
fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    subtle::ConstantTimeEq::ct_eq(a.as_bytes(), b.as_bytes()).into()
}

pub async fn auth_middleware(
    Extension(state): Extension<AuthState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let admin_key = state.admin_key.read().await;

    let expected = match &*admin_key {
        Some(k) if !k.is_empty() => Some(k.as_str()),
        _ => None,
    };

    // Auth disabled — no key configured at startup
    let Some(expected) = expected else {
        return Ok(next.run(request).await);
    };

    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    match token {
        Some(t) if secure_compare(t, expected) => Ok(next.run(request).await),
        _ => {
            // Check registered beta tester keys
            drop(admin_key);
            let keys = state.registered_keys.read().await;
            if keys.contains_key(token.unwrap_or("")) {
                return Ok(next.run(request).await);
            }
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

// ---------------------------------------------------------------------------
// Governor config — per-IP rate limiting on auth endpoints
// ---------------------------------------------------------------------------

/// Strict GovernorConfig for auth endpoints: 3 req/s per IP.
pub fn auth_governor_config() -> GovernorConfig {
    GovernorConfig::default()
}

/// GovernorConfig for protected API endpoints: 5 req/s per IP.
/// More permissive than auth endpoints since these require a valid Bearer token first.
pub fn protected_governor_config() -> GovernorConfig {
    GovernorConfig::default()
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
    /// Optional label for this API key (e.g., "telegram-bot", "dev-machine").
    /// Not used for auth — purely for the user's record.
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    /// The generated API key — shown only once. Must be saved by the client.
    pub api_key: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Auth handlers
// ---------------------------------------------------------------------------

/// POST /auth/login — authenticate with an API key and receive a Bearer token.
/// Rate-limited: 3 req/s per IP.
pub async fn login(
    Extension(state): Extension<AuthState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    let admin_key = state.admin_key.read().await;

    // Accept either the admin key or any registered beta tester key
    let valid = match &*admin_key {
        Some(ref admin) if secure_compare(&req.api_key, admin) => true,
        _ => {
            drop(admin_key);
            let keys = state.registered_keys.read().await;
            keys.contains_key(&req.api_key)
        }
    };

    if !valid {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(Json(LoginResponse {
        token: req.api_key,
        message: "authenticated".to_string(),
    }))
}

/// POST /auth/refresh — refresh a Bearer token (re-issue the same key).
/// Rate-limited: 3 req/s per IP.
pub async fn refresh(
    Extension(state): Extension<AuthState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, StatusCode> {
    let admin_key = state.admin_key.read().await;

    let valid = match &*admin_key {
        Some(ref admin) if secure_compare(&req.token, admin) => true,
        _ => {
            drop(admin_key);
            let keys = state.registered_keys.read().await;
            keys.contains_key(&req.token)
        }
    };

    if !valid {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(Json(RefreshResponse {
        token: req.token,
        message: "token refreshed".to_string(),
    }))
}

/// POST /auth/register — self-serve beta onboarding.
/// Generates a new API key. No existing key required.
/// Rate-limited: 10 req/min per IP.
pub async fn register(
    Extension(state): Extension<AuthState>,
    Json(_req): Json<RegisterRequest>,
) -> Json<RegisterResponse> {
    let api_key = generate_api_key();

    let mut keys = state.registered_keys.write().await;
    let count = keys.len() + 1;
    keys.insert(api_key.clone(), ());
    tracing::info!(key_prefix = &api_key[..8], total_keys = count, "beta tester registered");

    Json(RegisterResponse {
        api_key,
        message: "Registration successful. Save your API key now — it cannot be retrieved again.".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Auth router
// ---------------------------------------------------------------------------

/// Build the auth sub-router.
/// Note: `init_rate_limiter!` must be called once before the app starts
/// (in main.rs), otherwise all requests will be rejected.
pub fn auth_router() -> Router {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/register", post(register))
}

/// Build the auth sub-router with state, for merging into a Router<AppState>.
pub fn auth_router_with_state<S: Clone + Send + Sync + 'static>(state: S) -> Router<S> {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/register", post(register))
        .with_state(state)
}

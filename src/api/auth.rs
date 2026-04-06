use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::{routing::post, Extension, Json, Router};
use axum_governor::GovernorConfig;
#[cfg(feature = "postgres-storage")]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
#[cfg(feature = "postgres-storage")]
use chrono::{Duration as ChronoDuration, Utc};
#[cfg(feature = "postgres-storage")]
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg(feature = "postgres-storage")]
use crate::storage::postgres_store::stored_api_key_fingerprint;
#[cfg(feature = "postgres-storage")]
use crate::storage::PostgresStore;

/// Shared state for API key validation.
#[derive(Clone)]
pub struct AuthState {
    /// The static KNOWWHERE_API_KEY set at server startup (guards the admin).
    pub admin_key: Arc<RwLock<Option<String>>>,
    /// Postgres store for user/API-key lookups (only available with postgres-storage).
    #[cfg(feature = "postgres-storage")]
    pub pg_store: Option<Arc<PostgresStore>>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self {
            admin_key: Arc::new(RwLock::new(None)),
            #[cfg(feature = "postgres-storage")]
            pg_store: None,
        }
    }
}

/// Generate a stable API key: `kw_` + base64(random_bytes(24))
#[cfg(feature = "postgres-storage")]
fn generate_api_key() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..24).map(|_| rng.gen()).collect();
    format!("kw_{}", URL_SAFE_NO_PAD.encode(&bytes))
}

#[cfg(feature = "postgres-storage")]
fn session_expires_at() -> chrono::DateTime<chrono::Utc> {
    let ttl_days = std::env::var("AUTH_SESSION_TTL_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30);
    Utc::now() + ChronoDuration::days(ttl_days)
}

/// Constant-time string comparison to prevent timing attacks.
fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    subtle::ConstantTimeEq::ct_eq(a.as_bytes(), b.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// Governor config — per-IP rate limiting on auth endpoints
// ---------------------------------------------------------------------------

/// Strict GovernorConfig for auth endpoints: 3 req/s per IP.
pub fn auth_governor_config() -> GovernorConfig {
    GovernorConfig::default()
}

/// GovernorConfig for protected API endpoints: 5 req/s per IP.
pub fn protected_governor_config() -> GovernorConfig {
    GovernorConfig::default()
}

// ---------------------------------------------------------------------------
// Auth request/response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub api_key: String,
    pub user_id: String,
    pub message: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: String,
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

// ---------------------------------------------------------------------------
// Auth handlers
// ---------------------------------------------------------------------------

/// POST /auth/register — create a new user account and generate an API key.
/// The plain-text API key is shown ONLY once and must be saved by the client.
#[cfg(feature = "postgres-storage")]
pub async fn register(
    Extension(pg_store): Extension<Arc<PostgresStore>>,
    Extension(state): Extension<AuthState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, StatusCode> {
    // Validate input
    if req.username.is_empty() || req.email.is_empty() || req.password.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.password.len() < 8 {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Hash the password with bcrypt
    let password_hash = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create the user
    let user_id = pg_store
        .create_user(&req.username, &req.email, &password_hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to create user: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Generate API key
    let api_key = generate_api_key();
    let fingerprint = stored_api_key_fingerprint(&api_key);

    // Store the API key
    pg_store
        .create_api_key(user_id, &fingerprint, "default")
        .await
        .map_err(|e| {
            tracing::error!("failed to create API key: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let count = {
        let keys = state.admin_key.read().await;
        keys.as_ref().map(|k| k.len()).unwrap_or(0)
    };
    tracing::info!(
        user_id = %user_id,
        username = %req.username,
        key_prefix = &api_key[..8],
        total_keys = count + 1,
        "user registered"
    );

    Ok(Json(RegisterResponse {
        api_key,
        user_id: user_id.to_string(),
        message: "Registration successful. Save your API key now — it cannot be retrieved again."
            .to_string(),
    }))
}

/// POST /auth/login — authenticate with username + password, returns a session token.
#[cfg(feature = "postgres-storage")]
pub async fn login(
    Extension(pg_store): Extension<Arc<PostgresStore>>,
    Extension(state): Extension<AuthState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    // Static admin key must be used directly as Bearer token (no /login minting).
    let admin_key = state.admin_key.read().await;
    if let Some(ref admin) = *admin_key {
        if secure_compare(&req.password, admin) && req.username == "admin" {
            tracing::warn!(
                "admin login via /login is disabled; use KNOWWHERE_API_KEY as Bearer token"
            );
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    drop(admin_key);

    // Look up user by username
    let user = pg_store
        .get_user_by_username(&req.username)
        .await
        .map_err(|e| {
            tracing::error!("failed to lookup user: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let Some(user) = user else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    // Verify password
    if !bcrypt::verify(&req.password, &user.password_hash).unwrap_or(false) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // For beta: the "session token" is the API key itself.
    let api_key = generate_api_key();
    let fingerprint = stored_api_key_fingerprint(&api_key);
    let expires_at = session_expires_at();

    pg_store
        .create_api_key_with_expiry(user.id, &fingerprint, "session", Some(expires_at))
        .await
        .map_err(|e| {
            tracing::error!("failed to create session API key: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::info!(user_id = %user.id, username = %req.username, "user logged in");

    Ok(Json(LoginResponse {
        token: api_key,
        expires_at: expires_at.to_rfc3339(),
        message: "authenticated".to_string(),
    }))
}

/// POST /auth/refresh — validate the current API key and replace it with a new one (rotation).
#[cfg(feature = "postgres-storage")]
pub async fn refresh(
    Extension(pg_store): Extension<Arc<PostgresStore>>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, StatusCode> {
    if req.token.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let row = pg_store
        .find_api_key_by_plaintext(&req.token)
        .await
        .map_err(|e| {
            tracing::error!("refresh: api key lookup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let Some(row) = row else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let new_key = generate_api_key();
    let new_fp = stored_api_key_fingerprint(&new_key);

    let expires_at = session_expires_at();
    pg_store
        .replace_api_key(row.id, row.user_id, &row.name, &new_fp, Some(expires_at))
        .await
        .map_err(|e| {
            tracing::error!("refresh: key rotation failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::info!(user_id = %row.user_id, "api key rotated via /auth/refresh");

    Ok(Json(RefreshResponse {
        token: new_key,
        message: "token refreshed — store the new key; the previous key is invalid".to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Fallback handlers when postgres-storage is NOT enabled
// ---------------------------------------------------------------------------

#[cfg(not(feature = "postgres-storage"))]
pub async fn register(
    _: Extension<AuthState>,
    Json(_req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, StatusCode> {
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

#[cfg(not(feature = "postgres-storage"))]
pub async fn login(
    _: Extension<AuthState>,
    Json(_req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

#[cfg(not(feature = "postgres-storage"))]
pub async fn refresh(
    Json(_req): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, StatusCode> {
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

pub async fn auth_middleware(
    Extension(state): Extension<AuthState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Step 1: Check static admin key (KNOWWHERE_API_KEY env var — backward compat)
    {
        let admin_key = state.admin_key.read().await;
        if let Some(expected) = &*admin_key {
            if !expected.is_empty() {
                let token = request
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|h| h.strip_prefix("Bearer "));
                if let Some(t) = token {
                    if secure_compare(t, expected) {
                        return Ok(next.run(request).await);
                    }
                }
            }
        }
    }

    // Step 2: Check PG-backed API keys (only available with postgres-storage)
    #[cfg(feature = "postgres-storage")]
    {
        if let Some(ref pg_store) = state.pg_store {
            let token = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "));

            if let Some(t) = token {
                match pg_store.find_api_key_by_plaintext(t).await {
                    Ok(Some(row)) => {
                        let _ = pg_store.record_api_key_usage(row.id).await;
                        tracing::debug!(user_id = %row.user_id, "api key authenticated via PG");
                        return Ok(next.run(request).await);
                    }
                    Ok(None) => {
                        tracing::debug!("api key not found in PG");
                    }
                    Err(e) => {
                        tracing::error!("PG api_key lookup failed: {e}");
                    }
                }
            }
        }
    }

    // Auth disabled AND no valid PG key → reject
    Err(StatusCode::UNAUTHORIZED)
}

// ---------------------------------------------------------------------------
// Auth router
// ---------------------------------------------------------------------------

/// Build the auth sub-router with state, for merging into a Router<AppState>.
pub fn auth_router_with_state<S: Clone + Send + Sync + 'static>(state: S) -> Router<S> {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/register", post(register))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// ApiKey extension type
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ApiKey(pub Option<String>);

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

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

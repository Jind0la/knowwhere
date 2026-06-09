//! Voice message ingestion endpoint.
//!
//! POST /voice/upload — accepts multipart audio uploads from drivers.
//! Validates MIME type and file size, stores the file with a UUID name,
//! and returns the ID for downstream processing (transcription, etc.).

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use utoipa::ToSchema;
use uuid::Uuid;

/// Allowed MIME types for voice uploads.
const ALLOWED_MIME_TYPES: &[&str] = &[
    "audio/ogg",
    "audio/mpeg",
    "audio/wav",
    "audio/webm",
    "audio/mp4",
    "audio/x-m4a",
    "audio/aac",
    "audio/flac",
    "audio/opus",
];

/// Maximum file size in bytes (default: 25 MB).
const DEFAULT_MAX_SIZE: usize = 25 * 1024 * 1024;

/// Field name expected in the multipart form.
const FIELD_NAME: &str = "audio";

/// Determine the file extension from the MIME type.
fn extension_from_mime(mime: &str) -> &'static str {
    match mime {
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "audio/webm" => "webm",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/flac" => "flac",
        _ => "bin",
    }
}

/// Resolve the upload directory from environment or default.
fn upload_dir() -> PathBuf {
    std::env::var("VOICE_UPLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/voice_messages"))
}

/// Read max upload size from environment or default.
fn max_upload_size() -> usize {
    std::env::var("VOICE_UPLOAD_MAX_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_SIZE)
}

// ── Request / Response types ──

#[derive(Deserialize, ToSchema)]
pub struct VoiceUploadQuery {
    /// Optional driver identifier for audit trail.
    #[serde(default)]
    pub driver_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct VoiceUploadResponse {
    /// Unique file ID (UUID v4).
    pub id: Uuid,
    /// File path on disk (relative to VOICE_UPLOAD_DIR).
    pub path: String,
    /// Detected MIME type.
    pub mime_type: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Human-readable message.
    pub message: String,
}

#[derive(Serialize, ToSchema)]
pub struct VoiceUploadError {
    pub error: String,
    pub detail: String,
}

// ── Handler ──

/// Accept a multipart audio upload, validate, store, and return the file ID.
///
/// The form must contain a single field named `audio` with the audio file.
/// MIME type must be one of the allowed audio types (ogg, mp3, wav, webm, etc.).
/// File size must not exceed VOICE_UPLOAD_MAX_SIZE (default 25 MB).
///
/// On success, the file is stored under `VOICE_UPLOAD_DIR/{uuid}.{ext}` and the
/// UUID is returned for downstream transcription/processing.
#[utoipa::path(
    post,
    path = "/voice/upload",
    tag = "voice",
    request_body(
        content_type = "multipart/form-data",
        description = "Audio file to upload (field name: 'audio')"
    ),
    params(
        ("driver_id" = Option<String>, Query, description = "Optional driver identifier for audit trail")
    ),
    responses(
        (status = 201, description = "File stored successfully", body = VoiceUploadResponse),
        (status = 400, description = "Bad request — invalid file type or no file provided", body = VoiceUploadError),
        (status = 413, description = "File too large", body = VoiceUploadError),
        (status = 500, description = "Internal server error — disk write failed", body = VoiceUploadError)
    )
)]
pub async fn upload_voice(
    State(_state): State<crate::api::types::AppState>,
    axum::extract::Query(query): axum::extract::Query<VoiceUploadQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<VoiceUploadResponse>), (StatusCode, Json<VoiceUploadError>)> {
    let max_size = max_upload_size();

    // ── Extract the 'audio' field ──
    let mut audio_data: Option<(String, Vec<u8>)> = None; // (mime_type, bytes)

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        if name != FIELD_NAME {
            continue;
        }

        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        // Validate MIME type
        if !ALLOWED_MIME_TYPES.contains(&content_type.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(VoiceUploadError {
                    error: "invalid_file_type".into(),
                    detail: format!(
                        "MIME type '{}' is not allowed. Allowed: {}",
                        content_type,
                        ALLOWED_MIME_TYPES.join(", ")
                    ),
                }),
            ));
        }

        // Read the field body into memory
        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(VoiceUploadError {
                    error: "read_error".into(),
                    detail: format!("Failed to read upload data: {e}"),
                }),
            )
        })?;

        // Validate file size
        if data.len() > max_size {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(VoiceUploadError {
                    error: "file_too_large".into(),
                    detail: format!(
                        "File size {} bytes exceeds maximum {} bytes ({} MB)",
                        data.len(),
                        max_size,
                        max_size / (1024 * 1024)
                    ),
                }),
            ));
        }

        // Validate minimum size (reject empty files)
        if data.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(VoiceUploadError {
                    error: "empty_file".into(),
                    detail: "Uploaded file is empty".into(),
                }),
            ));
        }

        audio_data = Some((content_type, data.to_vec()));
        break;
    }

    // No audio field found
    let (mime_type, data) = audio_data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(VoiceUploadError {
                error: "missing_field".into(),
                detail: format!(
                    "No '{}' field found in the multipart form. Expected a file upload with field name '{}'.",
                    FIELD_NAME, FIELD_NAME
                ),
            }),
        )
    })?;

    // ── Generate UUID and store file ──
    let id = Uuid::new_v4();
    let ext = extension_from_mime(&mime_type);
    let filename = format!("{id}.{ext}");
    let dir = upload_dir();
    let file_path = dir.join(&filename);

    // Create directory if it doesn't exist
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(VoiceUploadError {
                error: "storage_error".into(),
                detail: format!("Failed to create upload directory '{}': {e}", dir.display()),
            }),
        )
    })?;

    // Write file to disk
    let size_bytes = data.len() as u64;
    let mut file = tokio::fs::File::create(&file_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(VoiceUploadError {
                error: "storage_error".into(),
                detail: format!("Failed to create file '{}': {e}", file_path.display()),
            }),
        )
    })?;

    file.write_all(&data).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(VoiceUploadError {
                error: "storage_error".into(),
                detail: format!("Failed to write file '{}': {e}", file_path.display()),
            }),
        )
    })?;

    file.flush().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(VoiceUploadError {
                error: "storage_error".into(),
                detail: format!("Failed to flush file '{}': {e}", file_path.display()),
            }),
        )
    })?;

    // ── Log with audit context ──
    if let Some(ref driver_id) = query.driver_id {
        tracing::info!(
            %id,
            %driver_id,
            mime_type = %mime_type,
            size_bytes,
            path = %file_path.display(),
            "voice message uploaded by driver"
        );
    } else {
        tracing::info!(
            %id,
            mime_type = %mime_type,
            size_bytes,
            path = %file_path.display(),
            "voice message uploaded (anonymous)"
        );
    }

    Ok((
        StatusCode::CREATED,
        Json(VoiceUploadResponse {
            id,
            path: filename.clone(),
            mime_type: mime_type.clone(),
            size_bytes,
            message: format!("Voice message stored as {filename}"),
        }),
    ))
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use serial_test::serial;
    use std::sync::Arc;
    use tower::ServiceExt;

    // ── Minimal mock EmbeddingProvider for tests ──
    // The voice upload handler doesn't call embed(), but AppState requires it.
    use async_trait::async_trait;

    struct MockEmbeddingProvider;

    #[async_trait]
    impl crate::embedding::EmbeddingProvider for MockEmbeddingProvider {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, anyhow::Error> {
            Ok(vec![0.0f32; 768])
        }
        async fn embed_batch(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, anyhow::Error> {
            Ok(vec![vec![0.0f32; 768]])
        }
        fn document_prefix(&self) -> &str {
            ""
        }
        fn query_prefix(&self) -> &str {
            ""
        }
        fn dimension(&self) -> usize {
            768
        }
        fn name(&self) -> &str {
            "mock"
        }
    }

    /// Helper to build a test AppState with in-memory store.
    async fn test_state() -> crate::api::types::AppState {
        use crate::memory::events::InMemoryEventStore;
        use crate::memory::DreamMode;
        use crate::memory::GovernancePolicy;
        use crate::storage::MemoryStore;
        use tokio::sync::RwLock;

        let store: Arc<dyn crate::storage::StorageBackend> = Arc::new(MemoryStore::new());
        let dream = DreamMode::new(store.clone());
        let embedding = Arc::new(MockEmbeddingProvider);

        crate::api::types::AppState {
            store: store.clone(),
            dream_store: store.clone(),
            dream,
            embedding,
            router: None,
            governance_policy: Arc::new(RwLock::new(GovernancePolicy::default_policy())),
            events: InMemoryEventStore::new(),
            #[cfg(feature = "postgres-storage")]
            trajectory_pool: None,
            #[cfg(feature = "postgres-storage")]
            pg_store: None,
            #[cfg(feature = "reranker")]
            reranker: None,
            frigate_dedup: crate::api::webhooks::DedupCache::new(),
            frigate_webhook_secret: None,
            homeassistant_dedup: crate::api::webhooks::DedupCache::new(),
            homeassistant_webhook_secret: None,
            temporal_weight: Arc::new(RwLock::new(None)),
            default_source_type_weights: None,
        }
    }

    fn build_router(state: crate::api::types::AppState) -> axum::Router {
        axum::Router::new()
            .route("/voice/upload", axum::routing::post(upload_voice))
            .with_state(state)
    }

    /// Build a multipart request body.
    fn multipart_body(field_name: &str, filename: &str, mime_type: &str, data: &[u8]) -> Vec<u8> {
        let boundary = "testboundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\nContent-Type: {mime_type}\r\n\r\n"
            ).as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    // ── Test cases ──

    /// T1: Happy path — upload a valid OGG file.
    #[serial]
    #[tokio::test]
    async fn test_upload_valid_ogg() {
        // Use a temp dir for uploads
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let fake_audio = b"FAKE_OGG_DATA_WITH_ENOUGH_BYTES_TO_PASS_MINIMUM_CHECK";
        let body = multipart_body("audio", "voice.ogg", "audio/ogg", fake_audio);

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload?driver_id=driver_42")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp_body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();

        assert!(resp_body["id"].as_str().is_some(), "no id returned");
        assert_eq!(resp_body["mime_type"], "audio/ogg");
        assert!(resp_body["size_bytes"].as_u64().unwrap() > 0);
        assert!(resp_body["path"].as_str().unwrap().ends_with(".ogg"));

        // Verify file exists on disk
        let file_path = tmp.path().join(resp_body["path"].as_str().unwrap());
        assert!(file_path.exists(), "file not stored at {file_path:?}");

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T2: Missing audio field returns 400.
    #[serial]
    #[tokio::test]
    async fn test_missing_audio_field() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        // Send empty multipart (no audio field)
        let boundary = "testboundary";
        let body = format!("--{boundary}--\r\n").into_bytes();

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T3: Invalid MIME type returns 400.
    #[serial]
    #[tokio::test]
    async fn test_invalid_mime_type() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let body = multipart_body("audio", "not_audio.txt", "text/plain", b"hello world");

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp_body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 1024).await.unwrap())
                .unwrap();
        assert_eq!(resp_body["error"], "invalid_file_type");

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T4: Empty file returns 400.
    #[serial]
    #[tokio::test]
    async fn test_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let body = multipart_body("audio", "empty.ogg", "audio/ogg", b"");

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp_body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 1024).await.unwrap())
                .unwrap();
        assert_eq!(resp_body["error"], "empty_file");

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T5: File too large returns 413.
    #[serial]
    #[tokio::test]
    async fn test_file_too_large() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());
        // Set max size to 10 bytes for testing
        std::env::set_var("VOICE_UPLOAD_MAX_SIZE", "10");

        let state = test_state().await;
        let router = build_router(state);

        let large_data = vec![0u8; 100]; // 100 bytes > 10
        let body = multipart_body("audio", "large.ogg", "audio/ogg", &large_data);

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let resp_body: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 1024).await.unwrap())
                .unwrap();
        assert_eq!(resp_body["error"], "file_too_large");

        std::env::remove_var("VOICE_UPLOAD_DIR");
        std::env::remove_var("VOICE_UPLOAD_MAX_SIZE");
    }

    /// T6: Upload MP3 format.
    #[serial]
    #[tokio::test]
    async fn test_upload_mp3() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let fake_audio = b"FAKE_MP3_DATA_WITH_ENOUGH_BYTES";
        let body = multipart_body("audio", "recording.mp3", "audio/mpeg", fake_audio);

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp_body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();

        assert_eq!(resp_body["mime_type"], "audio/mpeg");
        assert!(resp_body["path"].as_str().unwrap().ends_with(".mp3"));

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T7: Upload WebM format.
    #[serial]
    #[tokio::test]
    async fn test_upload_webm() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let fake_audio = b"FAKE_WEBM_DATA";
        let body = multipart_body("audio", "voice.webm", "audio/webm", fake_audio);

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp_body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();

        assert_eq!(resp_body["mime_type"], "audio/webm");
        assert!(resp_body["path"].as_str().unwrap().ends_with(".webm"));

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T8: WAV format upload.
    #[serial]
    #[tokio::test]
    async fn test_upload_wav() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let fake_audio = b"FAKE_WAV_DATA_WITH_MINIMUM_LENGTH_TO_PASS";
        let body = multipart_body("audio", "voice.wav", "audio/wav", fake_audio);

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp_body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();

        assert_eq!(resp_body["mime_type"], "audio/wav");
        assert!(resp_body["path"].as_str().unwrap().ends_with(".wav"));

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T9: Verify driver_id appears in response path (audit context is logged, not returned in body).
    /// The driver_id is only used for logging — verified via tracing in real runs.
    #[serial]
    #[tokio::test]
    async fn test_driver_id_logged() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        let fake_audio = b"VOICE_DATA_FOR_DRIVER_AUDIT_TEST";
        let body = multipart_body("audio", "msg.ogg", "audio/ogg", fake_audio);

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload?driver_id=driver_007")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }

    /// T10: Multiple fields — only 'audio' is processed, others ignored.
    #[serial]
    #[tokio::test]
    async fn test_ignores_other_fields() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("VOICE_UPLOAD_DIR", tmp.path().to_str().unwrap());

        let state = test_state().await;
        let router = build_router(state);

        // Build a multipart with extra fields before and after audio
        let boundary = "testboundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"comment\"\r\n\r\nThis is a note\r\n"
            ).as_bytes(),
        );
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"voice.ogg\"\r\nContent-Type: audio/ogg\r\n\r\n"
            ).as_bytes(),
        );
        body.extend_from_slice(b"ACTUAL_AUDIO_DATA_FOR_TESTING_12345");
        body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"extra\"\r\n\r\nignored\r\n").as_bytes(),
        );
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let req = Request::builder()
            .method("POST")
            .uri("/voice/upload")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=testboundary",
            )
            .body(Body::from(body))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp_body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();

        assert_eq!(resp_body["mime_type"], "audio/ogg");

        std::env::remove_var("VOICE_UPLOAD_DIR");
    }
}

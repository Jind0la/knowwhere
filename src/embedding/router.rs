use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use super::audio::AudioProvider;
use super::clip::ClipProvider;
use super::provider::EmbeddingProvider;
use super::sensor::sensor_to_text;

// =============================================================================
// EmbeddingRouter — content-type based cross-modal dispatch
// =============================================================================

/// Holds references to text, image, and audio embedders and routes incoming
/// payloads to the correct provider based on the MIME content type.
///
/// All output embeddings are guaranteed to be 768-dimensional.
pub struct EmbeddingRouter {
    text_provider: Arc<dyn EmbeddingProvider>,
    clip_provider: Arc<ClipProvider>,
    audio_provider: Arc<AudioProvider>,
}

impl EmbeddingRouter {
    pub fn new(
        text_provider: Arc<dyn EmbeddingProvider>,
        clip_provider: Arc<ClipProvider>,
        audio_provider: Arc<AudioProvider>,
    ) -> Self {
        Self {
            text_provider,
            clip_provider,
            audio_provider,
        }
    }

    /// Route based on `content_type`:
    ///
    /// | Content type       | Provider                     |
    /// |--------------------|------------------------------|
    /// | `text/*`           | `EmbeddingProvider.embed()`  |
    /// | `image/*`          | `ClipProvider.embed_image()` |
    /// | `audio/*`          | transcribe → text embed      |
    /// | `application/json` | sensor-to-text → text embed  |
    ///
    /// All paths produce a 768-dimensional `Vec<f32>`.
    pub async fn route(&self, content_type: &str, payload: &[u8]) -> Result<Vec<f32>> {
        if content_type.starts_with("text/") {
            let text = std::str::from_utf8(payload).context("text payload is not valid UTF-8")?;
            self.text_provider.embed(text).await
        } else if content_type.starts_with("image/") {
            self.clip_provider.embed_image(payload).await
        } else if content_type.starts_with("audio/") {
            self.audio_provider
                .embed_audio(payload, &*self.text_provider)
                .await
        } else if content_type == "application/json" {
            let value: Value =
                serde_json::from_slice(payload).context("sensor payload is not valid JSON")?;
            let text = sensor_to_text(&value);
            self.text_provider.embed(&text).await
        } else {
            anyhow::bail!("unsupported content type: {}", content_type)
        }
    }

    /// Classify a MIME `content_type` into a dispatch arm name.
    ///
    /// Pure logic — no I/O. Useful for testing routing decisions independently.
    pub fn classify(content_type: &str) -> Option<&'static str> {
        if content_type.starts_with("text/") {
            Some("text")
        } else if content_type.starts_with("image/") {
            Some("image")
        } else if content_type.starts_with("audio/") {
            Some("audio")
        } else if content_type == "application/json" {
            Some("sensor")
        } else {
            None
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    // ---------- mock text provider ----------

    struct MockTextProvider {
        received: Mutex<Vec<String>>,
    }

    impl MockTextProvider {
        fn new() -> Self {
            Self {
                received: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for MockTextProvider {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.received.lock().unwrap().push(text.to_string());
            Ok(vec![0.1; 768])
        }

        fn dimension(&self) -> usize {
            768
        }

        fn name(&self) -> &str {
            "mock-text"
        }
    }

    // ---------- helper: build a test router ----------

    fn test_router() -> EmbeddingRouter {
        EmbeddingRouter::new(
            Arc::new(MockTextProvider::new()),
            Arc::new(ClipProvider::default()),
            Arc::new(AudioProvider::default()),
        )
    }

    // ==================================================================
    // Pure routing-logic tests (no Ollama required)
    // ==================================================================

    #[test]
    fn test_classify_text() {
        assert_eq!(EmbeddingRouter::classify("text/plain"), Some("text"));
        assert_eq!(EmbeddingRouter::classify("text/html"), Some("text"));
        assert_eq!(
            EmbeddingRouter::classify("text/plain; charset=utf-8"),
            Some("text")
        );
    }

    #[test]
    fn test_classify_image() {
        assert_eq!(EmbeddingRouter::classify("image/png"), Some("image"));
        assert_eq!(EmbeddingRouter::classify("image/jpeg"), Some("image"));
        assert_eq!(EmbeddingRouter::classify("image/webp"), Some("image"));
    }

    #[test]
    fn test_classify_audio() {
        assert_eq!(EmbeddingRouter::classify("audio/wav"), Some("audio"));
        assert_eq!(EmbeddingRouter::classify("audio/mpeg"), Some("audio"));
        assert_eq!(EmbeddingRouter::classify("audio/ogg"), Some("audio"));
    }

    #[test]
    fn test_classify_sensor() {
        assert_eq!(
            EmbeddingRouter::classify("application/json"),
            Some("sensor")
        );
    }

    #[test]
    fn test_classify_unsupported() {
        assert_eq!(EmbeddingRouter::classify("video/mp4"), None);
        assert_eq!(EmbeddingRouter::classify("application/pdf"), None);
        assert_eq!(EmbeddingRouter::classify(""), None);
    }

    // ==================================================================
    // Text-path tests (no Ollama required — uses MockTextProvider)
    // ==================================================================

    #[tokio::test]
    async fn test_route_text_plain() {
        let router = test_router();
        let result = router.route("text/plain", b"hello world").await.unwrap();
        assert_eq!(result.len(), 768);
        assert!(result.iter().all(|&v| (v - 0.1).abs() < 1e-6));
    }

    #[tokio::test]
    async fn test_route_text_html() {
        let router = test_router();
        let result = router.route("text/html", b"<p>hello</p>").await.unwrap();
        assert_eq!(result.len(), 768);
    }

    #[tokio::test]
    async fn test_route_text_non_utf8_rejected() {
        let router = test_router();
        let err = router
            .route("text/plain", b"\xFF\xFE\xFD")
            .await
            .unwrap_err();
        assert!(
            format!("{}", err).contains("valid UTF-8"),
            "expected UTF-8 error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_route_text_with_charset() {
        let router = test_router();
        let result = router
            .route("text/plain; charset=utf-8", b"hello")
            .await
            .unwrap();
        assert_eq!(result.len(), 768);
    }

    // ==================================================================
    // Sensor-path tests (no Ollama required — uses MockTextProvider)
    // ==================================================================

    #[tokio::test]
    async fn test_route_sensor_json() {
        let router = test_router();
        let payload = br#"{"temperature": 23.5, "humidity": 60}"#;
        let result = router.route("application/json", payload).await.unwrap();
        assert_eq!(result.len(), 768);
        assert!(result.iter().all(|&v| (v - 0.1).abs() < 1e-6));
    }

    #[tokio::test]
    async fn test_route_sensor_invalid_json() {
        let router = test_router();
        let err = router
            .route("application/json", b"not json")
            .await
            .unwrap_err();
        assert!(
            format!("{}", err).contains("valid JSON"),
            "expected JSON error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_route_unsupported_content_type() {
        let router = test_router();
        let err = router.route("video/mp4", b"fake video").await.unwrap_err();
        assert!(
            format!("{}", err).contains("unsupported content type"),
            "expected unsupported error, got: {}",
            err
        );
    }

    // ==================================================================
    // Integration tests — require running Ollama + CLIP/Whisper models
    // ==================================================================

    #[tokio::test]
    #[ignore]
    async fn test_route_image_png() {
        let router = test_router();
        let result = router
            .route("image/png", b"\x89PNG\r\n\x1a\n")
            .await
            .unwrap();
        assert_eq!(result.len(), 768);
    }

    #[tokio::test]
    #[ignore]
    async fn test_route_audio_wav() {
        let router = test_router();
        let result = router.route("audio/wav", b"RIFF....WAVE").await.unwrap();
        assert_eq!(result.len(), 768);
    }

    #[tokio::test]
    #[ignore]
    async fn test_all_outputs_768_dim() {
        let router = test_router();

        let text = router.route("text/plain", b"test").await.unwrap();
        assert_eq!(text.len(), 768);

        let img = router
            .route("image/png", b"\x89PNG\r\n\x1a\n")
            .await
            .unwrap();
        assert_eq!(img.len(), 768);

        let audio = router.route("audio/wav", b"RIFFdata").await.unwrap();
        assert_eq!(audio.len(), 768);

        let sensor = router
            .route("application/json", b"{\"k\":1}")
            .await
            .unwrap();
        assert_eq!(sensor.len(), 768);
    }
}

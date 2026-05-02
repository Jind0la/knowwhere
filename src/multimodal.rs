use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::embedding::audio::AudioProvider;
use crate::embedding::clip::ClipProvider;
use crate::embedding::provider::EmbeddingProvider;
use crate::embedding::router::EmbeddingRouter;

// =============================================================================
// MultimodalData — serializable payload for storage / API responses
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum MultimodalData {
    Image {
        pointer: String,
        embedding: Vec<f32>,
    },
    Audio {
        pointer: String,
        embedding: Vec<f32>,
    },
    Sensor {
        #[schema(value_type = Object)]
        data: Value,
        embedding: Vec<f32>,
    },
}

impl MultimodalData {
    pub fn embedding(&self) -> &[f32] {
        match self {
            Self::Image { embedding, .. } => embedding,
            Self::Audio { embedding, .. } => embedding,
            Self::Sensor { embedding, .. } => embedding,
        }
    }

    pub fn pointer_or_label(&self) -> &str {
        match self {
            Self::Image { pointer, .. } => pointer,
            Self::Audio { pointer, .. } => pointer,
            Self::Sensor { .. } => "sensor-data",
        }
    }
}

// =============================================================================
// CrossModalEmbedder — trait + production implementation
// =============================================================================

/// Async trait for cross-modal embedding. Dispatches raw payloads to the
/// correct provider based on the MIME content type.
#[async_trait]
pub trait CrossModalEmbedder: Send + Sync {
    /// Embed a raw payload according to its content type.
    ///
    /// | `content_type`      | Provider                          |
    /// |---------------------|-----------------------------------|
    /// | `text/*`            | text `EmbeddingProvider.embed()`  |
    /// | `image/*`           | `ClipProvider.embed_image()`      |
    /// | `audio/*`           | Whisper transcribe → text embed   |
    /// | `application/json`  | sensor-to-text → text embed       |
    ///
    /// All paths return a 768-dim `Vec<f32>`.
    async fn cross_embed(&self, content_type: &str, payload: &[u8]) -> Result<Vec<f32>>;
}

/// Production implementation backed by an [`EmbeddingRouter`].
pub struct KnowWhereCrossModalEmbedder {
    router: Arc<EmbeddingRouter>,
}

impl KnowWhereCrossModalEmbedder {
    pub fn new(
        text_provider: Arc<dyn EmbeddingProvider>,
        clip_provider: Arc<ClipProvider>,
        audio_provider: Arc<AudioProvider>,
    ) -> Self {
        Self {
            router: Arc::new(EmbeddingRouter::new(
                text_provider,
                clip_provider,
                audio_provider,
            )),
        }
    }
}

#[async_trait]
impl CrossModalEmbedder for KnowWhereCrossModalEmbedder {
    async fn cross_embed(&self, content_type: &str, payload: &[u8]) -> Result<Vec<f32>> {
        self.router.route(content_type, payload).await
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ---- mock text provider ----

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

    // ---- tests ----

    #[tokio::test]
    async fn test_cross_embed_text() {
        let embedder = KnowWhereCrossModalEmbedder::new(
            Arc::new(MockTextProvider::new()),
            Arc::new(ClipProvider::default()),
            Arc::new(AudioProvider::default()),
        );

        let result = embedder
            .cross_embed("text/plain", b"hello world")
            .await
            .unwrap();
        assert_eq!(result.len(), 768);
    }

    #[tokio::test]
    async fn test_cross_embed_sensor() {
        let embedder = KnowWhereCrossModalEmbedder::new(
            Arc::new(MockTextProvider::new()),
            Arc::new(ClipProvider::default()),
            Arc::new(AudioProvider::default()),
        );

        let emb = embedder
            .cross_embed("application/json", b"{\"temp\":23}")
            .await
            .unwrap();
        assert_eq!(emb.len(), 768);
    }

    #[tokio::test]
    async fn test_cross_embed_unsupported_type() {
        let embedder = KnowWhereCrossModalEmbedder::new(
            Arc::new(MockTextProvider::new()),
            Arc::new(ClipProvider::default()),
            Arc::new(AudioProvider::default()),
        );

        let err = embedder
            .cross_embed("video/mp4", b"fake")
            .await
            .unwrap_err();
        assert!(
            format!("{}", err).contains("unsupported content type"),
            "got: {}",
            err
        );
    }

    // ==================================================================
    // Integration tests — require running Ollama + CLIP/Whisper models
    // ==================================================================

    #[tokio::test]
    #[ignore]
    async fn test_cross_embed_image() {
        let embedder = KnowWhereCrossModalEmbedder::new(
            Arc::new(MockTextProvider::new()),
            Arc::new(ClipProvider::default()),
            Arc::new(AudioProvider::default()),
        );
        let result = embedder
            .cross_embed("image/png", b"\x89PNG\r\n\x1a\n")
            .await
            .unwrap();
        assert_eq!(result.len(), 768);
    }

    #[tokio::test]
    #[ignore]
    async fn test_cross_embed_audio() {
        let embedder = KnowWhereCrossModalEmbedder::new(
            Arc::new(MockTextProvider::new()),
            Arc::new(ClipProvider::default()),
            Arc::new(AudioProvider::default()),
        );
        let result = embedder
            .cross_embed("audio/wav", b"RIFFdata")
            .await
            .unwrap();
        assert_eq!(result.len(), 768);
    }

    #[test]
    fn test_multimodal_data_embedding_accessor() {
        let data = MultimodalData::Image {
            pointer: "img.jpg".into(),
            embedding: vec![1.0, 2.0, 3.0],
        };
        assert_eq!(data.embedding(), &[1.0_f32, 2.0, 3.0]);
    }

    #[test]
    fn test_multimodal_data_pointer_or_label() {
        let img = MultimodalData::Image {
            pointer: "photo.png".into(),
            embedding: vec![],
        };
        assert_eq!(img.pointer_or_label(), "photo.png");

        let sensor = MultimodalData::Sensor {
            data: serde_json::json!({"t": 1}),
            embedding: vec![],
        };
        assert_eq!(sensor.pointer_or_label(), "sensor-data");
    }
}

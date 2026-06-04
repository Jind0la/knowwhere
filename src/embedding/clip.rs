use anyhow::{Context, Result};
use base64::Engine;
use reqwest::Client;

/// Embeds images into 768-dimensional vectors using CLIP via Ollama's
/// `/api/embeddings` endpoint.
///
/// Reads `OLLAMA_URL` (default `http://localhost:11434`) and
/// `OLLAMA_CLIP_MODEL` (default `clip-vit-large`) from the environment.
pub struct ClipProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl ClipProvider {
    pub fn new() -> Self {
        let base_url =
            std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model =
            std::env::var("OLLAMA_CLIP_MODEL").unwrap_or_else(|_| "clip-vit-large".into());
        Self {
            client: Client::new(),
            base_url,
            model,
        }
    }

    /// Send raw image bytes to Ollama's embedding API and return a 768-dim
    /// CLIP embedding vector.
    ///
    /// The image bytes are base64-encoded and passed in a single-element
    /// `input` array as required by the Ollama embeddings endpoint.
    pub async fn embed_image(&self, image_bytes: &[u8]) -> Result<Vec<f32>> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);

        let resp: serde_json::Value = self
            .client
            .post(format!("{}/api/embeddings", self.base_url))
            .json(&serde_json::json!({
                "model": self.model,
                "input": b64,
            }))
            .send()
            .await
            .context("CLIP embedding request failed")?
            .json()
            .await
            .context("CLIP embedding parse failed")?;

        let emb = resp["embeddings"][0]
            .as_array()
            .context("missing embeddings array in CLIP response")?;

        Ok(emb
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect())
    }

    /// Return the known embedding dimension for CLIP ViT-Large.
    pub fn dimension(&self) -> usize {
        768
    }
}

impl Default for ClipProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_clip_provider_new_uses_defaults() {
        std::env::remove_var("OLLAMA_URL");
        std::env::remove_var("OLLAMA_CLIP_MODEL");
        let provider = ClipProvider::new();
        assert_eq!(provider.dimension(), 768);
        assert_eq!(provider.model, "clip-vit-large");
        assert!(provider.base_url.contains("11434"), "default port is 11434");
    }

    #[test]
    fn test_clip_provider_default_constructs() {
        let provider = ClipProvider::default();
        assert_eq!(provider.dimension(), 768);
    }

    #[test]
    #[serial]
    fn test_clip_provider_respects_env() {
        std::env::set_var("OLLAMA_CLIP_MODEL", "clip-vit-base");
        std::env::set_var("OLLAMA_URL", "http://ollama:9999");
        let provider = ClipProvider::new();
        assert_eq!(provider.model, "clip-vit-base");
        assert_eq!(provider.base_url, "http://ollama:9999");
        std::env::remove_var("OLLAMA_CLIP_MODEL");
        std::env::remove_var("OLLAMA_URL");
    }

    /// The `embed_image` method requires a running Ollama instance with
    /// the CLIP model pulled, so this test is `#[ignore]` by default.
    #[tokio::test]
    #[ignore]
    async fn test_embed_image_real() {
        let provider = ClipProvider::new();
        // A tiny 1x1 white PNG
        let fake_image: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG sig
        ];
        let _ = provider.embed_image(fake_image).await;
    }
}

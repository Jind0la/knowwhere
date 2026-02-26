use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn dimension(&self) -> usize;
    fn name(&self) -> &str;
}

// -- Grok (xAI) --

pub struct GrokProvider {
    api_key: String,
    client: reqwest::Client,
}

impl GrokProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for GrokProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let resp: EmbeddingResponse = self
            .client
            .post("https://api.x.ai/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": "v3-embedding",
                "input": text
            }))
            .send()
            .await
            .context("grok embedding request failed")?
            .error_for_status()
            .context("grok API returned error status")?
            .json()
            .await
            .context("failed to parse grok response")?;

        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .context("empty embedding response from grok")
    }

    fn dimension(&self) -> usize {
        1536
    }

    fn name(&self) -> &str {
        "grok"
    }
}

// -- OpenAI --

pub struct OpenAIProvider {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let resp: EmbeddingResponse = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": "text-embedding-3-small",
                "input": text
            }))
            .send()
            .await
            .context("openai embedding request failed")?
            .error_for_status()
            .context("openai API returned error status")?
            .json()
            .await
            .context("failed to parse openai response")?;

        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .context("empty embedding response from openai")
    }

    fn dimension(&self) -> usize {
        1536
    }

    fn name(&self) -> &str {
        "openai"
    }
}

// -- Local Ollama (Placeholder: deterministischer Pseudo-Embedding-Generator) --

pub struct LocalOllamaProvider {
    dim: usize,
}

impl LocalOllamaProvider {
    pub fn new() -> Self {
        Self { dim: 384 }
    }
}

impl Default for LocalOllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmbeddingProvider for LocalOllamaProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let seed = hasher.finish();

        let mut vec = Vec::with_capacity(self.dim);
        let mut state = seed;
        for _ in 0..self.dim {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            vec.push(((state >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0);
        }

        let mag: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if mag > 0.0 {
            for v in &mut vec {
                *v /= mag;
            }
        }

        Ok(vec)
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "local-ollama"
    }
}

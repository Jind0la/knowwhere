use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn dimension(&self) -> usize;
    fn name(&self) -> &str;
    fn document_prefix(&self) -> &str { "" }
    fn query_prefix(&self) -> &str { "" }
}

pub async fn embed_document(provider: &dyn EmbeddingProvider, text: &str) -> Result<Vec<f32>> {
    let p = provider.document_prefix();
    if p.is_empty() {
        provider.embed(text).await
    } else {
        provider.embed(&format!("{p}{text}")).await
    }
}

pub async fn embed_query(provider: &dyn EmbeddingProvider, text: &str) -> Result<Vec<f32>> {
    let p = provider.query_prefix();
    if p.is_empty() {
        provider.embed(text).await
    } else {
        provider.embed(&format!("{p}{text}")).await
    }
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

// -- Local Ollama (real HTTP embedding via nomic-embed-text) --

pub struct LocalOllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

#[derive(Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

impl LocalOllamaProvider {
    pub fn new() -> Self {
        let base_url =
            std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("OLLAMA_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text-v2-moe".into());
        Self {
            client: reqwest::Client::new(),
            base_url,
            model,
        }
    }
}

impl Default for LocalOllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmbeddingProvider for LocalOllamaProvider {
    fn document_prefix(&self) -> &str { "search_document: " }
    fn query_prefix(&self) -> &str { "search_query: " }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let resp: OllamaEmbeddingResponse = self
            .client
            .post(format!("{}/api/embeddings", self.base_url))
            .json(&serde_json::json!({
                "model": self.model,
                "prompt": text
            }))
            .send()
            .await
            .context("ollama embedding request failed")?
            .error_for_status()
            .context("ollama API returned error status")?
            .json()
            .await
            .context("failed to parse ollama embedding response")?;

        if resp.embedding.is_empty() {
            anyhow::bail!("ollama returned empty embedding");
        }

        Ok(resp.embedding)
    }

    fn dimension(&self) -> usize {
        768
    }

    fn name(&self) -> &str {
        "local-ollama"
    }
}

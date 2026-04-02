use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.embed(t).await?);
        }
        Ok(results)
    }
    
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

pub async fn embed_document_batch(provider: &dyn EmbeddingProvider, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let p = provider.document_prefix();
    if p.is_empty() {
        provider.embed_batch(texts).await
    } else {
        let prefixed: Vec<String> = texts.iter().map(|t| format!("{p}{t}")).collect();
        let refs: Vec<&str> = prefixed.iter().map(String::as_str).collect();
        provider.embed_batch(&refs).await
    }
}

pub async fn embed_query_batch(provider: &dyn EmbeddingProvider, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let p = provider.query_prefix();
    if p.is_empty() {
        provider.embed_batch(texts).await
    } else {
        let prefixed: Vec<String> = texts.iter().map(|t| format!("{p}{t}")).collect();
        let refs: Vec<&str> = prefixed.iter().map(String::as_str).collect();
        provider.embed_batch(&refs).await
    }
}

// =============================================================================
// Cloud Providers (OpenAI / Grok) — behind feature flags
// =============================================================================
// At most one of openai-provider or grok-provider should be enabled at a time.

#[cfg(any(feature = "openai-provider", feature = "grok-provider"))]
use serde::Deserialize as SharedDeserialize;

#[cfg(any(feature = "openai-provider", feature = "grok-provider"))]
#[derive(SharedDeserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[cfg(any(feature = "openai-provider", feature = "grok-provider"))]
#[derive(SharedDeserialize)]
struct EmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

// -- Grok (xAI) --

#[cfg(feature = "grok-provider")]
pub struct GrokProvider {
    api_key: String,
    client: reqwest::Client,
}

#[cfg(feature = "grok-provider")]
impl GrokProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[cfg(feature = "grok-provider")]
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

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let resp: EmbeddingResponse = self
            .client
            .post("https://api.x.ai/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": "v3-embedding",
                "input": texts
            }))
            .send()
            .await
            .context("grok batch embedding request failed")?
            .error_for_status()
            .context("grok API returned error status")?
            .json()
            .await
            .context("failed to parse grok batch response")?;

        let mut data = resp.data;
        data.sort_by_key(|d| d.index);
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimension(&self) -> usize { 1536 }
    fn name(&self) -> &str { "grok" }
}

// -- OpenAI --

#[cfg(feature = "openai-provider")]
pub struct OpenAIProvider {
    api_key: String,
    client: reqwest::Client,
}

#[cfg(feature = "openai-provider")]
impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[cfg(feature = "openai-provider")]
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

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let resp: EmbeddingResponse = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": "text-embedding-3-small",
                "input": texts
            }))
            .send()
            .await
            .context("openai batch embedding request failed")?
            .error_for_status()
            .context("openai API returned error status")?
            .json()
            .await
            .context("failed to parse openai batch response")?;

        let mut data = resp.data;
        data.sort_by_key(|d| d.index);
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimension(&self) -> usize { 1536 }
    fn name(&self) -> &str { "openai" }
}

// =============================================================================
// Local Ollama (always available, the default and tested provider)
// =============================================================================

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

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        use futures::future::try_join_all;
        let futures: Vec<_> = texts.iter().map(|t| self.embed(t)).collect();
        try_join_all(futures).await
    }

    fn dimension(&self) -> usize { 768 }
    fn name(&self) -> &str { "local-ollama" }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use std::sync::Mutex;

    struct MockProvider {
        calls: Mutex<Vec<Vec<String>>>,
        responses: Mutex<Vec<Vec<Vec<f32>>>>,
        call_count: Mutex<usize>,
    }

    impl MockProvider {
        fn new(responses: Vec<Vec<Vec<f32>>>) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                responses: Mutex::new(responses),
                call_count: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for MockProvider {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![1.0])
        }

        async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(texts.iter().map(|s| s.to_string()).collect());
            let mut count = self.call_count.lock().unwrap();
            let resp = self.responses.lock().unwrap();
            let result = resp.get(*count).cloned().unwrap_or(vec![]);
            *count += 1;
            drop(resp);
            Ok(result)
        }

        fn document_prefix(&self) -> &str { "search_document: " }
        fn query_prefix(&self) -> &str { "search_query: " }

        fn dimension(&self) -> usize { 4 }
        fn name(&self) -> &str { "mock" }
    }

    #[tokio::test]
    async fn test_embed_document_batch_with_prefix() {
        let provider = MockProvider::new(vec![
            vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]]
        ]);
        let texts = vec!["hello", "world"];
        let results = embed_document_batch(&provider, &texts).await.unwrap();
        assert_eq!(results.len(), 2);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0][0].starts_with("search_document: "));
    }

    #[tokio::test]
    async fn test_embed_query_batch_with_prefix() {
        let provider = MockProvider::new(vec![
            vec![vec![1.0, 0.0, 0.0, 0.0]]
        ]);
        let texts = vec!["query text"];
        let results = embed_query_batch(&provider, &texts).await.unwrap();
        assert_eq!(results.len(), 1);
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0][0].starts_with("search_query: "));
    }

    #[tokio::test]
    async fn test_batch_order_preserved() {
        let provider = MockProvider::new(vec![
            vec![vec![1.0], vec![2.0], vec![3.0]]
        ]);
        let texts = vec!["first", "second", "third"];
        let results = provider.embed_batch(&texts).await.unwrap();
        assert_eq!(results[0], vec![1.0]);
        assert_eq!(results[1], vec![2.0]);
        assert_eq!(results[2], vec![3.0]);
    }
}

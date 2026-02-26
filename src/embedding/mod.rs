pub mod provider;

pub use provider::{EmbeddingProvider, GrokProvider, LocalOllamaProvider, OpenAIProvider};

use std::sync::Arc;

pub enum ProviderKind {
    Grok,
    OpenAI,
    LocalOllama,
}

pub fn create_provider(kind: ProviderKind, api_key: Option<String>) -> Arc<dyn EmbeddingProvider> {
    match kind {
        ProviderKind::Grok => match api_key {
            Some(key) => Arc::new(GrokProvider::new(key)),
            None => {
                tracing::warn!("no API key for Grok, falling back to local-ollama");
                Arc::new(LocalOllamaProvider::new())
            }
        },
        ProviderKind::OpenAI => match api_key {
            Some(key) => Arc::new(OpenAIProvider::new(key)),
            None => {
                tracing::warn!("no API key for OpenAI, falling back to local-ollama");
                Arc::new(LocalOllamaProvider::new())
            }
        },
        ProviderKind::LocalOllama => Arc::new(LocalOllamaProvider::new()),
    }
}

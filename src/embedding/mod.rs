pub mod provider;

pub use provider::{
    EmbeddingProvider, LocalOllamaProvider,
    embed_document, embed_query, embed_document_batch, embed_query_batch,
};

use std::sync::Arc;

// =============================================================================
// Provider kind — cloud variants behind feature flags
// At most one of openai-provider or grok-provider should be enabled at a time.
// =============================================================================

#[derive(Debug, Clone, Copy)]
pub enum ProviderKind {
    LocalOllama,
    #[cfg(feature = "openai-provider")]
    OpenAI,
    #[cfg(feature = "grok-provider")]
    Grok,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::LocalOllama => write!(f, "local_ollama"),
            #[cfg(feature = "openai-provider")]
            ProviderKind::OpenAI => write!(f, "openai"),
            #[cfg(feature = "grok-provider")]
            ProviderKind::Grok => write!(f, "grok"),
        }
    }
}

impl ProviderKind {
    /// Parse a string into a ProviderKind. Returns None if the string doesn't match
    /// any available provider (given the current feature flags).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "local_ollama" | "ollama" => Some(ProviderKind::LocalOllama),
            #[cfg(feature = "openai-provider")]
            "openai" => Some(ProviderKind::OpenAI),
            #[cfg(feature = "grok-provider")]
            "grok" => Some(ProviderKind::Grok),
            _ => None,
        }
    }
}

pub fn create_provider(kind: ProviderKind, #[allow(unused)] api_key: Option<String>) -> Arc<dyn EmbeddingProvider> {
    match kind {
        ProviderKind::LocalOllama => Arc::new(provider::LocalOllamaProvider::new()),
        
        #[cfg(feature = "openai-provider")]
        ProviderKind::OpenAI => Arc::new(provider::OpenAIProvider::new(
            api_key.expect("OPENAI_API_KEY must be set when openai-provider feature is enabled"),
        )),
        
        #[cfg(feature = "grok-provider")]
        ProviderKind::Grok => Arc::new(provider::GrokProvider::new(
            api_key.expect("GROK_API_KEY must be set when grok-provider feature is enabled"),
        )),
        
        // Compile-time exhaustiveness: if only LocalOllama is available, the match
        // is already exhaustive without any #[cfg] arms
        #[cfg(not(any(feature = "openai-provider", feature = "grok-provider")))]
        _ => {
            // Safety: when no cloud providers are enabled, this arm is only reachable
            // for LocalOllama (which is matched above), so this is unreachable.
            // We use create_provider in a cfg-gated context in main.rs anyway.
            let _ = kind;
            let _ = api_key;
            unreachable!("create_provider called with cloud provider but no cloud features enabled")
        }
    }
}

pub mod audio;
pub mod clip;
pub mod provider;
pub mod sensor;

pub use audio::AudioProvider;
pub use clip::ClipProvider;
pub use provider::{
    embed_document, embed_document_batch, embed_query, embed_query_batch, EmbeddingProvider,
    LocalOllamaProvider,
};

pub use sensor::{embed_sensor, sensor_to_text};

use std::sync::Arc;

// =============================================================================
// Provider kind — cloud variants behind feature flags
// At most one of openai-provider or grok-provider should be enabled at a time.
// =============================================================================

#[derive(Debug, Clone, Copy)]
pub enum ProviderKind {
    Clip,
    LocalOllama,
    #[cfg(feature = "openai-provider")]
    OpenAI,
    #[cfg(feature = "grok-provider")]
    Grok,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Clip => write!(f, "clip"),
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
            "clip" => Some(ProviderKind::Clip),
            "local_ollama" | "ollama" => Some(ProviderKind::LocalOllama),
            #[cfg(feature = "openai-provider")]
            "openai" => Some(ProviderKind::OpenAI),
            #[cfg(feature = "grok-provider")]
            "grok" => Some(ProviderKind::Grok),
            _ => None,
        }
    }
}

pub fn create_provider(
    kind: ProviderKind,
    #[allow(unused)] api_key: Option<String>,
) -> Arc<dyn EmbeddingProvider> {
    match kind {
        ProviderKind::Clip => panic!(
            "ClipProvider is not an EmbeddingProvider — use clip::ClipProvider directly for image embeddings"
        ),
        ProviderKind::LocalOllama => Arc::new(provider::LocalOllamaProvider::new()),

        #[cfg(feature = "openai-provider")]
        ProviderKind::OpenAI => Arc::new(provider::OpenAIProvider::new(
            api_key.expect("OPENAI_API_KEY must be set when openai-provider feature is enabled"),
        )),

        #[cfg(feature = "grok-provider")]
        ProviderKind::Grok => Arc::new(provider::GrokProvider::new(
            api_key.expect("GROK_API_KEY must be set when grok-provider feature is enabled"),
        )),
    }
}

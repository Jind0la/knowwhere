use std::sync::Arc;

use knowwhere_server::embedding::EmbeddingProvider;
use knowwhere_server::embedding::LocalOllamaProvider;
#[cfg(any(
    feature = "openai-provider",
    feature = "grok-provider",
    feature = "voyage-provider"
))]
use knowwhere_server::embedding::{create_provider, ProviderKind};
#[cfg(feature = "postgres-storage")]
use knowwhere_server::storage::PostgresStore;
use knowwhere_server::storage::{MemoryStore, StorageBackend};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitMode {
    Off,
    Proxy,
}

pub fn rate_limit_mode_from_env() -> RateLimitMode {
    match std::env::var("RATE_LIMIT_MODE") {
        Ok(v) if v.eq_ignore_ascii_case("proxy") => RateLimitMode::Proxy,
        Ok(v) if v.eq_ignore_ascii_case("off") => RateLimitMode::Off,
        Ok(_) => RateLimitMode::Off,
        Err(_) if std::env::var("RATE_LIMIT").is_ok() => RateLimitMode::Proxy,
        Err(_) => RateLimitMode::Off,
    }
}

fn memory_store_from_data_dir() -> Arc<dyn StorageBackend> {
    let data_dir = std::env::var("KNOWWHERE_DATA_DIR").unwrap_or_else(|_| "./data".into());
    Arc::new(
        MemoryStore::with_persistence(&data_dir).unwrap_or_else(|e| {
            tracing::warn!("persistence init failed ({e}), using in-memory only");
            MemoryStore::new()
        }),
    )
}

#[cfg(feature = "postgres-storage")]
pub async fn init_store() -> anyhow::Result<(Arc<dyn StorageBackend>, Option<Arc<PostgresStore>>)> {
    if let Ok(database_url) = std::env::var("DATABASE_URL") {
        match PostgresStore::connect(&database_url).await {
            Ok(pg_store) => {
                tracing::info!(
                    "storage: PostgreSQL (primary store — data will persist in PostgreSQL)"
                );
                let pg_arc = Arc::new(pg_store);
                let strict = std::env::var("AUTH_STRICT_MIGRATIONS")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if let Err(e) = pg_arc.run_auth_migrations().await {
                    if strict {
                        anyhow::bail!("auth migrations failed in strict mode: {e}");
                    }
                    tracing::warn!("auth migrations failed ({e}), continuing anyway");
                }
                Ok((Arc::clone(&pg_arc) as Arc<dyn StorageBackend>, Some(pg_arc)))
            }
            Err(e) => {
                tracing::warn!("postgres connection failed ({e}), falling back to MemoryStore");
                Ok((memory_store_from_data_dir(), None))
            }
        }
    } else {
        tracing::info!("DATABASE_URL not set — using MemoryStore (JSON persistence)");
        Ok((memory_store_from_data_dir(), None))
    }
}

#[cfg(not(feature = "postgres-storage"))]
pub async fn init_store() -> anyhow::Result<Arc<dyn StorageBackend>> {
    Ok(memory_store_from_data_dir())
}

/// When set, overrides automatic embedding selection (API keys, defaults).
/// `ollama` / `local` → always [`LocalOllamaProvider`] (useful when the shell/IDE exports cloud keys but you want local dev).
/// `grok` / `xai` → Grok if `grok-provider` feature + `GROK_API_KEY`; else Ollama with a warning.
/// `voyage` → Voyage if `voyage-provider` feature + `VOYAGE_API_KEY`; else Ollama with a warning.
/// `openai` → OpenAI if `openai-provider` feature + `OPENAI_API_KEY`; else Ollama with a warning.
pub const EMBEDDING_PROVIDER_ENV: &str = "KNOWWHERE_EMBEDDING_PROVIDER";

fn local_ollama_provider_with_log(reason: &'static str) -> Arc<dyn EmbeddingProvider> {
    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text".into());
    tracing::info!(model, %reason, "embedding provider: local Ollama");
    Arc::new(LocalOllamaProvider::new())
}

fn embedding_provider_from_env_override() -> Option<Arc<dyn EmbeddingProvider>> {
    let raw = std::env::var(EMBEDDING_PROVIDER_ENV).ok()?;
    let p = raw.trim().to_ascii_lowercase();
    match p.as_str() {
        "" => None,
        "ollama" | "local" => Some(local_ollama_provider_with_log(
            "KNOWWHERE_EMBEDDING_PROVIDER",
        )),
        "grok" | "xai" => {
            #[cfg(feature = "grok-provider")]
            {
                match std::env::var("GROK_API_KEY") {
                    Ok(key) => {
                        tracing::info!(
                            env = EMBEDDING_PROVIDER_ENV,
                            "using Grok embedding provider (forced)"
                        );
                        Some(create_provider(ProviderKind::Grok, Some(key)))
                    }
                    Err(_) => {
                        tracing::warn!(
                            env = EMBEDDING_PROVIDER_ENV,
                            "set to grok but GROK_API_KEY missing — using Ollama"
                        );
                        Some(local_ollama_provider_with_log("forced grok, no key"))
                    }
                }
            }
            #[cfg(not(feature = "grok-provider"))]
            {
                tracing::warn!(
                    env = EMBEDDING_PROVIDER_ENV,
                    "set to grok but grok-provider feature disabled — using Ollama"
                );
                Some(local_ollama_provider_with_log("grok feature off"))
            }
        }
        "voyage" => {
            #[cfg(feature = "voyage-provider")]
            {
                match std::env::var("VOYAGE_API_KEY") {
                    Ok(key) => {
                        tracing::info!(
                            env = EMBEDDING_PROVIDER_ENV,
                            dimension = 1024,
                            "using Voyage embedding provider (forced); new embeddings are 1024d — existing Ollama nodes may be 768d"
                        );
                        Some(create_provider(ProviderKind::Voyage, Some(key)))
                    }
                    Err(_) => {
                        tracing::warn!(
                            env = EMBEDDING_PROVIDER_ENV,
                            "set to voyage but VOYAGE_API_KEY missing — using Ollama"
                        );
                        Some(local_ollama_provider_with_log("forced voyage, no key"))
                    }
                }
            }
            #[cfg(not(feature = "voyage-provider"))]
            {
                tracing::warn!(
                    env = EMBEDDING_PROVIDER_ENV,
                    "set to voyage but voyage-provider feature disabled — using Ollama"
                );
                Some(local_ollama_provider_with_log("voyage feature off"))
            }
        }
        "openai" => {
            #[cfg(feature = "openai-provider")]
            {
                match std::env::var("OPENAI_API_KEY") {
                    Ok(key) => {
                        tracing::info!(
                            env = EMBEDDING_PROVIDER_ENV,
                            "using OpenAI embedding provider (forced)"
                        );
                        Some(create_provider(ProviderKind::OpenAI, Some(key)))
                    }
                    Err(_) => {
                        tracing::warn!(
                            env = EMBEDDING_PROVIDER_ENV,
                            "set to openai but OPENAI_API_KEY missing — using Ollama"
                        );
                        Some(local_ollama_provider_with_log("forced openai, no key"))
                    }
                }
            }
            #[cfg(not(feature = "openai-provider"))]
            {
                tracing::warn!(
                    env = EMBEDDING_PROVIDER_ENV,
                    "set to openai but openai-provider feature disabled — using Ollama"
                );
                Some(local_ollama_provider_with_log("openai feature off"))
            }
        }
        other => {
            tracing::warn!(
                env = EMBEDDING_PROVIDER_ENV,
                value = other,
                "unknown value; using automatic selection"
            );
            None
        }
    }
}

#[cfg(feature = "postgres-storage")]
pub async fn init_trajectory_pool() -> Option<Arc<sqlx::PgPool>> {
    if let Ok(database_url) = std::env::var("DATABASE_URL") {
        match sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
        {
            Ok(pool) => {
                tracing::info!("postgres storage enabled (trajectory logging + tiered context)");
                Some(Arc::new(pool))
            }
            Err(e) => {
                tracing::warn!(
                    "DATABASE_URL set but connection failed ({e}), running without postgres-storage features"
                );
                None
            }
        }
    } else {
        tracing::info!("DATABASE_URL not set — postgres-storage features disabled");
        None
    }
}

pub fn init_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    if let Some(provider) = embedding_provider_from_env_override() {
        return provider;
    }
    if let Ok(key) = std::env::var("VOYAGE_API_KEY") {
        #[cfg(feature = "voyage-provider")]
        {
            tracing::info!(
                dimension = 1024,
                "using Voyage embedding provider (voyage-code-3); new embeddings are 1024d — existing Ollama nodes may be 768d"
            );
            create_provider(ProviderKind::Voyage, Some(key))
        }
        #[cfg(not(feature = "voyage-provider"))]
        {
            drop(key);
            tracing::warn!(
                "VOYAGE_API_KEY is set but voyage-provider feature is not enabled — falling back to Ollama"
            );
            Arc::new(LocalOllamaProvider::new())
        }
    } else if let Ok(key) = std::env::var("GROK_API_KEY") {
        #[cfg(feature = "grok-provider")]
        {
            tracing::info!("using Grok embedding provider");
            create_provider(ProviderKind::Grok, Some(key))
        }
        #[cfg(not(feature = "grok-provider"))]
        {
            drop(key);
            tracing::warn!(
                "GROK_API_KEY is set but grok-provider feature is not enabled — falling back to Ollama"
            );
            Arc::new(LocalOllamaProvider::new())
        }
    } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        #[cfg(feature = "openai-provider")]
        {
            tracing::info!("using OpenAI embedding provider");
            create_provider(ProviderKind::OpenAI, Some(key))
        }
        #[cfg(not(feature = "openai-provider"))]
        {
            drop(key);
            tracing::warn!(
                "OPENAI_API_KEY is set but openai-provider feature is not enabled — falling back to Ollama"
            );
            Arc::new(LocalOllamaProvider::new())
        }
    } else {
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text".into());
        tracing::info!(model, "using local ollama embedding provider");
        Arc::new(LocalOllamaProvider::new())
    }
}

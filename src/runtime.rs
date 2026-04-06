use std::sync::Arc;

use knowwhere_server::embedding::EmbeddingProvider;
use knowwhere_server::embedding::LocalOllamaProvider;
#[cfg(any(feature = "openai-provider", feature = "grok-provider"))]
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
    if let Ok(key) = std::env::var("GROK_API_KEY") {
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
        let model =
            std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text-v2-moe".into());
        tracing::info!(model, "using local ollama embedding provider");
        Arc::new(LocalOllamaProvider::new())
    }
}

use std::sync::Arc;

use axum::middleware;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[cfg(feature = "postgres-storage")]
use sqlx::PgPoolOptions;

use knowwhere_server::api::{auth, auth::ApiKey, docs::ApiDoc, routes};
use knowwhere_server::connectors::frigate::FrigateConnector;
use knowwhere_server::connectors::store_external_event;
use knowwhere_server::embedding::{create_provider, EmbeddingProvider, ProviderKind};
use knowwhere_server::memory::events::InMemoryEventStore;
use knowwhere_server::memory::{DreamMode, GovernancePolicy};
use knowwhere_server::storage::MemoryStore;

fn main() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .name("knowwhere-main".into())
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime")
                .block_on(run())
                .expect("server error");
        })
        .expect("failed to spawn main thread")
        .join()
        .expect("main thread panicked");
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let data_dir = std::env::var("KNOWWHERE_DATA_DIR").unwrap_or_else(|_| "./data".into());
    let store = MemoryStore::with_persistence(&data_dir)
        .unwrap_or_else(|e| {
            tracing::warn!("persistence init failed ({e}), using in-memory only");
            MemoryStore::new()
        });
    let dream = DreamMode::new(store.clone());

    let embedding: Arc<dyn EmbeddingProvider> =
        if let Ok(key) = std::env::var("GROK_API_KEY") {
            tracing::info!("using Grok embedding provider");
            create_provider(ProviderKind::Grok, Some(key))
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            tracing::info!("using OpenAI embedding provider");
            create_provider(ProviderKind::OpenAI, Some(key))
        } else {
            let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text-v2-moe".into());
            tracing::info!(model, "no embedding API key found, using local ollama");
            create_provider(ProviderKind::LocalOllama, None)
        };

    tracing::info!(provider = embedding.name(), "embedding provider ready");

    tokio::spawn(dream.clone().micro_dream_loop());
    tracing::info!("dream mode started (micro-dream every 1h)");

    if let Ok(frigate_url) = std::env::var("FRIGATE_URL") {
        let connector_store = store.clone();
        let connector_embedding = embedding.clone();
        tracing::info!(url = %frigate_url, "connector manager started (frigate poller every 30s)");
        tokio::spawn(async move {
            let frigate = FrigateConnector::new(frigate_url);
            loop {
                match frigate.poll_events().await {
                    Ok(events) => {
                        for event in events {
                            if let Err(e) = store_external_event(
                                &connector_store,
                                &connector_embedding,
                                event,
                            )
                            .await
                            {
                                tracing::warn!("failed to store frigate event: {e}");
                            }
                        }
                    }
                    Err(e) => tracing::warn!("frigate poll error: {e}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    } else {
        tracing::info!("frigate connector disabled (set FRIGATE_URL to enable)");
    }

    let shutdown_store = store.clone();

    #[cfg(feature = "postgres-storage")]
    let trajectory_pool: Option<std::sync::Arc<sqlx::PgPool>> =
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            match sqlx::PgPoolOptions::new()
                .max_connections(5)
                .connect(&database_url)
                .await
            {
                Ok(pool) => {
                    tracing::info!("postgres storage enabled (trajectory logging + tiered context)");
                    Some(std::sync::Arc::new(pool))
                }
                Err(e) => {
                    tracing::warn!("DATABASE_URL set but connection failed ({e}), running without postgres-storage features");
                    None
                }
            }
        } else {
            tracing::info!("DATABASE_URL not set — postgres-storage features disabled");
            None
        };

    let state = routes::AppState {
        store,
        dream,
        embedding,
        governance_policy: GovernancePolicy::default_policy(),
        events: InMemoryEventStore::new(),
        #[cfg(feature = "postgres-storage")]
        trajectory_pool,
    };

    let api_key = ApiKey(std::env::var("KNOWWHERE_API_KEY").ok());

    let protected = Router::new()
        .route("/embed", post(routes::embed_text))
        .route("/store_session", post(routes::store_session))
        .route("/store_external", post(routes::store_external))
        .route("/retrieve/{id}", get(routes::retrieve))
        .route("/retrieve_fractal", post(routes::retrieve_fractal))
        .route("/nodes/recent", get(routes::recent_nodes))
        .route("/nodes/purge_dummy", post(routes::purge_dummy))
        .route("/nodes/reembed_all", post(routes::reembed_all))
        .route("/nodes/{id}", delete(routes::delete_node))
        .route("/dream/status", get(routes::dream_status))
        // -- postgres-storage features (trajectory + tiered context) --
        #[cfg(feature = "postgres-storage")]
        .route("/retrieval/runs", get(routes::list_retrieval_runs))
        #[cfg(feature = "postgres-storage")]
        .route("/retrieval/runs/{id}", get(routes::get_retrieval_run))
        #[cfg(feature = "postgres-storage")]
        .route("/retrieval/runs/{id}/trajectory", get(routes::get_retrieval_trajectory))
        #[cfg(feature = "postgres-storage")]
        .route("/memories/{id}/compact", post(routes::compact_memory))
        #[cfg(feature = "postgres-storage")]
        .route("/memories/{id}", get(routes::get_memory))
        #[cfg(feature = "postgres-storage")]
        .route("/conflicts", get(routes::list_conflicts))
        #[cfg(feature = "postgres-storage")]
        .route("/conflicts/{id}/resolve", post(routes::resolve_conflict))
        // Energy decay routes (Ebbinghaus forgetting curve)
        #[cfg(feature = "postgres-storage")]
        .route("/memories/{id}/energy/boost", post(routes::boost_memory_energy))
        #[cfg(feature = "postgres-storage")]
        .route("/energy/low", get(routes::list_low_energy_memories))
        #[cfg(feature = "postgres-storage")]
        .route("/energy/decay/apply", post(routes::apply_energy_decay))
        #[cfg(feature = "postgres-storage")]
        .route("/energy/compress", post(routes::compress_memory_cluster))
        // Deduplication routes
        #[cfg(feature = "postgres-storage")]
        .route("/deduplication/candidates", get(routes::list_deduplication_candidates))
        #[cfg(feature = "postgres-storage")]
        .route("/deduplication/run", post(routes::run_deduplication))
        #[cfg(feature = "postgres-storage")]
        .route("/deduplication/runs", get(routes::list_deduplication_runs))
        // Self-healing routes (content hashing for external nodes)
        #[cfg(feature = "postgres-storage")]
        .route("/memories/{id}/reindex", post(routes::reindex_external_node))
        #[cfg(feature = "postgres-storage")]
        .route("/memories/{id}/health", get(routes::memory_health_check))
        #[cfg(feature = "postgres-storage")]
        .route("/self-healing/stats", get(routes::self_healing_stats))
        .route_layer(middleware::from_fn(auth::auth_middleware))
        .layer(axum::Extension(api_key.clone()));

    let app = Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .fallback_service(ServeDir::new("frontend"))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    if std::env::var("KNOWWHERE_API_KEY").is_ok() {
        tracing::info!("auth enabled (Bearer token required for protected routes)");
    } else {
        tracing::warn!("KNOWWHERE_API_KEY not set – auth disabled (dev mode)");
    }

    let port = std::env::var("KNOWWHERE_PORT").unwrap_or_else(|_| "3737".into());
    let addr = format!("0.0.0.0:{port}");
    tracing::info!("KnowWhere server listening on {addr}");
    tracing::info!("Swagger UI: http://localhost:{port}/swagger-ui/");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    let shutdown = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        tracing::info!("shutdown signal received, saving state…");
        if let Err(e) = shutdown_store.save_to_disk().await {
            tracing::warn!("final save failed: {e}");
        } else {
            tracing::info!("state saved to disk");
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;

    Ok(())
}

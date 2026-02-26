use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use knowwhere_server::api::{auth, auth::ApiKey, docs::ApiDoc, routes};
use knowwhere_server::connectors::frigate::FrigateConnector;
use knowwhere_server::connectors::store_external_event;
use knowwhere_server::embedding::{create_provider, EmbeddingProvider, ProviderKind};
use knowwhere_server::memory::DreamMode;
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

    let store = MemoryStore::new();
    let dream = DreamMode::new(store.clone());

    let embedding: Arc<dyn EmbeddingProvider> =
        if let Ok(key) = std::env::var("GROK_API_KEY") {
            tracing::info!("using Grok embedding provider");
            create_provider(ProviderKind::Grok, Some(key))
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            tracing::info!("using OpenAI embedding provider");
            create_provider(ProviderKind::OpenAI, Some(key))
        } else {
            tracing::warn!("no embedding API key found, using local-ollama placeholder");
            create_provider(ProviderKind::LocalOllama, None)
        };

    tracing::info!(provider = embedding.name(), "embedding provider ready");

    tokio::spawn(dream.clone().micro_dream_loop());
    tracing::info!("dream mode started (micro-dream every 1h)");

    {
        let connector_store = store.clone();
        let connector_embedding = embedding.clone();
        tokio::spawn(async move {
            let frigate = FrigateConnector::new("http://frigate:5000".into());
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
        tracing::info!("connector manager started (frigate poller every 30s)");
    }

    let state = routes::AppState {
        store,
        dream,
        embedding,
    };

    let api_key = ApiKey(std::env::var("KNOWWHERE_API_KEY").ok());

    let protected = Router::new()
        .route("/embed", post(routes::embed_text))
        .route("/store_session", post(routes::store_session))
        .route("/store_external", post(routes::store_external))
        .route("/retrieve/{id}", get(routes::retrieve))
        .route("/retrieve_fractal", post(routes::retrieve_fractal))
        .route("/nodes/recent", get(routes::recent_nodes))
        .route("/dream/status", get(routes::dream_status))
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

    let addr = "0.0.0.0:3000";
    tracing::info!("KnowWhere server listening on {addr}");
    tracing::info!("Swagger UI: http://localhost:3000/swagger-ui/");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

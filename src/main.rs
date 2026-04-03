use std::sync::Arc;

use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;
use axum_governor::GovernorLayer;
use real::RealIpLayer;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[cfg(feature = "postgres-storage")]
use sqlx::postgres::PgPoolOptions;

use knowwhere_server::api::{auth, auth::ApiKey, docs::ApiDoc, routes, webhooks::DedupCache};
use lazy_limit::{init_rate_limiter, Duration, RuleConfig};
use knowwhere_server::connectors::frigate::FrigateConnector;
use knowwhere_server::connectors::store_external_event;
use knowwhere_server::embedding::EmbeddingProvider;
#[cfg(any(feature = "openai-provider", feature = "grok-provider"))]
use knowwhere_server::embedding::{create_provider, ProviderKind};
#[cfg(not(any(feature = "openai-provider", feature = "grok-provider")))]
use knowwhere_server::embedding::LocalOllamaProvider;
use knowwhere_server::memory::events::InMemoryEventStore;
use knowwhere_server::storage::StorageBackend;
use knowwhere_server::memory::{DreamMode, GovernancePolicy};
use knowwhere_server::scheduler::{AuditScheduler, ConsolidationScheduler, SchedulerConfig};
use knowwhere_server::storage::MemoryStore;
#[cfg(feature = "postgres-storage")]
use knowwhere_server::storage::PostgresStore;
use knowwhere_server::vlm::{VlmConfig, VlmWorker};

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

    // -- Rate Limiter (lazy-limit) — global, initialized once at startup --
    init_rate_limiter!(
        default: RuleConfig::new(Duration::seconds(1), 5),
        routes: [
            ("/login",    RuleConfig::new(Duration::seconds(1), 3)),
            ("/refresh",  RuleConfig::new(Duration::seconds(1), 3)),
            ("/register", RuleConfig::new(Duration::seconds(60), 10)),
        ]
    ).await;

    // ------------------------------------------------------------------
    // Storage: PostgreSQL (if DATABASE_URL) or MemoryStore (JSON fallback)
    // ------------------------------------------------------------------
    #[cfg(feature = "postgres-storage")]
    let store: Arc<dyn StorageBackend> = if let Ok(database_url) = std::env::var("DATABASE_URL") {
        match PostgresStore::connect(&database_url).await {
            Ok(pg_store) => {
                tracing::info!("storage: PostgreSQL (primary store — data will persist in PostgreSQL)");
                Arc::new(pg_store)
            }
            Err(e) => {
                tracing::warn!("postgres connection failed ({e}), falling back to MemoryStore");
                let data_dir = std::env::var("KNOWWHERE_DATA_DIR").unwrap_or_else(|_| "./data".into());
                Arc::new(
                    MemoryStore::with_persistence(&data_dir)
                        .unwrap_or_else(|e| {
                            tracing::warn!("persistence init failed ({e}), using in-memory only");
                            MemoryStore::new()
                        }),
                )
            }
        }
    } else {
        tracing::info!("DATABASE_URL not set — using MemoryStore (JSON persistence)");
        let data_dir = std::env::var("KNOWWHERE_DATA_DIR").unwrap_or_else(|_| "./data".into());
        Arc::new(
            MemoryStore::with_persistence(&data_dir)
                .unwrap_or_else(|e| {
                    tracing::warn!("persistence init failed ({e}), using in-memory only");
                    MemoryStore::new()
                }),
        )
    };

    #[cfg(not(feature = "postgres-storage"))]
    let store: Arc<dyn StorageBackend> = {
        let data_dir = std::env::var("KNOWWHERE_DATA_DIR").unwrap_or_else(|_| "./data".into());
        Arc::new(
            MemoryStore::with_persistence(&data_dir)
                .unwrap_or_else(|e| {
                    tracing::warn!("persistence init failed ({e}), using in-memory only");
                    MemoryStore::new()
                }),
        )
    };

    // Concrete store for VLM worker, schedulers, DreamMode, and FrigateConnector.
    let dream_store = store.clone();
    let connector_store = store.clone();
    let shutdown_store = store.clone();
    let dream = DreamMode::new(dream_store.clone());

    let embedding: Arc<dyn EmbeddingProvider> =
        if let Ok(key) = std::env::var("GROK_API_KEY") {
            #[cfg(feature = "grok-provider")]
            {
                tracing::info!("using Grok embedding provider");
                create_provider(ProviderKind::Grok, Some(key))
            }
            #[cfg(not(feature = "grok-provider"))]
            {
                drop(key);
                tracing::warn!("GROK_API_KEY is set but grok-provider feature is not enabled — falling back to Ollama");
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
                tracing::warn!("OPENAI_API_KEY is set but openai-provider feature is not enabled — falling back to Ollama");
                Arc::new(LocalOllamaProvider::new())
            }
        } else {
            let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text-v2-moe".into());
            tracing::info!(model, "using local ollama embedding provider");
            Arc::new(LocalOllamaProvider::new())
        };

    tracing::info!(provider = embedding.name(), "embedding provider ready");

    tokio::spawn(dream.clone().micro_dream_loop());
    tracing::info!("dream mode started (micro-dream every 1h)");

    if let Ok(frigate_url) = std::env::var("FRIGATE_URL") {
        let connector_embedding = embedding.clone();
        tracing::info!(url = %frigate_url, "connector manager started (frigate poller every 30s)");
        tokio::spawn(async move {
            let frigate = FrigateConnector::new(frigate_url);
            loop {
                match frigate.poll_events().await {
                    Ok(events) => {
                        for event in events {
                            if let Err(e) = store_external_event(
                                connector_store.as_ref(),
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

    #[cfg(feature = "postgres-storage")]
    let trajectory_pool: Option<std::sync::Arc<sqlx::PgPool>> =
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            match sqlx::postgres::PgPoolOptions::new()
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

    // -- VLM Background Worker (4-stage fallback: GPT-5-nano → GPT-4o-mini → Grok-4-fast → Ollama) --
    let vlm_config = VlmConfig::from_env();
    let (vlm_worker, _vlm_join) = if vlm_config.is_configured() {
        let (h, j) = VlmWorker::spawn(store.clone(), embedding.clone(), vlm_config);
        tracing::info!("VLM summarization worker started (OPENAI_API_KEY, GROK_API_KEY, or OLLAMA_VLM_MODEL detected)");
        (Some(h), Some(j))
    } else {
        tracing::warn!("no OPENAI_API_KEY, GROK_API_KEY, or OLLAMA_VLM_MODEL found — VLM summarization disabled");
        (None, None)
    };

    // -- Dream Mode Background Schedulers --
    let scheduler_config = SchedulerConfig::from_env();
    let consolidation_scheduler: Option<Arc<ConsolidationScheduler>>;
    if scheduler_config.is_enabled() {
        // ConsolidationScheduler: periodically enqueues L2 nodes for VLM summarization
        let consolidation = ConsolidationScheduler::new(
            store.clone(),
            vlm_worker.clone(),
            scheduler_config.clone(),
        );
        let (scheduler, _) = consolidation.spawn();
        consolidation_scheduler = Some(scheduler);

        // AuditScheduler: periodically applies energy decay, deduplication, conflict detection
        #[cfg(feature = "postgres-storage")]
        let audit = AuditScheduler::new(store.clone(), trajectory_pool.clone(), scheduler_config.clone());
        #[cfg(not(feature = "postgres-storage"))]
        let audit = AuditScheduler::new(store.clone(), scheduler_config.clone());
        audit.spawn();

        tracing::info!("Dream Mode scheduler started");
    } else {
        consolidation_scheduler = None;
        tracing::info!("Dream Mode scheduler disabled (DREAM_ENABLED=false)");
    }

    // Arc<dyn StorageBackend> for API layer — already an Arc<dyn StorageBackend>
    let store_for_api = store.clone();

    let state = routes::AppState {
        store: store.clone(),
        dream_store: store.clone(),
        dream,
        embedding,
        governance_policy: GovernancePolicy::default_policy(),
        events: InMemoryEventStore::new(),
        #[cfg(feature = "postgres-storage")]
        trajectory_pool,
        vlm_worker,
        consolidation: consolidation_scheduler,
        frigate_dedup: DedupCache::new(),
        frigate_webhook_secret: std::env::var("FRIGATE_WEBHOOK_SECRET").ok(),
    };

    let api_key = ApiKey(std::env::var("KNOWWHERE_API_KEY").ok());

    let mut protected = Router::new()
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
        // -- VLM Summarization Worker (3-stage fallback) --
        .route("/vlm/status", get(routes::vlm_status))
        .route("/vlm/summarize", post(routes::vlm_enqueue))
        // -- System routes --
        .route("/events", get(routes::list_events))
        // -- Governance routes --
        .route("/governance/policy", get(routes::get_governance_policy))
        .route("/governance/policy", post(routes::update_governance_policy))
        // -- Webhook routes --
        .route("/webhooks/frigate", post(routes::webhook_frigate));

    #[cfg(feature = "postgres-storage")]
    {
        protected = protected
            // -- postgres-storage features (trajectory + tiered context) --
            .route("/retrieval/runs", get(routes::list_retrieval_runs))
            .route("/retrieval/runs/{id}", get(routes::get_retrieval_run))
            .route("/retrieval/runs/{id}/trajectory", get(routes::get_retrieval_trajectory))
            .route("/memories/{id}/compact", post(routes::compact_memory))
            .route("/memories/{id}", get(routes::get_memory))
            .route("/conflicts", get(routes::list_conflicts))
            .route("/conflicts/{id}/resolve", post(routes::resolve_conflict))
            // Energy decay routes (Ebbinghaus forgetting curve)
            .route("/memories/{id}/energy/boost", post(routes::boost_memory_energy))
            .route("/energy/low", get(routes::list_low_energy_memories))
            .route("/energy/decay/apply", post(routes::apply_energy_decay))
            .route("/energy/compress", post(routes::compress_memory_cluster))
            // Deduplication routes
            .route("/deduplication/candidates", get(routes::list_deduplication_candidates))
            .route("/deduplication/run", post(routes::run_deduplication))
            .route("/deduplication/runs", get(routes::list_deduplication_runs))
            // Self-healing routes (content hashing for external nodes)
            .route("/memories/{id}/reindex", post(routes::reindex_external_node))
            .route("/memories/{id}/health", get(routes::memory_health_check))
            .route("/self-healing/stats", get(routes::self_healing_stats))
            // Namespace routes
            .route("/namespaces", get(routes::list_namespaces))
            .route("/namespaces", post(routes::create_namespace))
            .route("/namespaces/{path}", get(routes::get_namespace))
            .route("/namespaces/{path}/memories", get(routes::namespace_memories))
            .route("/namespaces/{path}/search", get(routes::namespace_search))
            // Skills routes
            .route("/skills", post(routes::create_skill))
            .route("/skills", get(routes::list_skills))
            .route("/skills/{id}", get(routes::get_skill))
            .route("/skills/{id}", put(routes::update_skill))
            .route("/skills/{id}", delete(routes::delete_skill))
            .route("/skills/{id}/use", post(routes::use_skill))
            .route("/skills/match", get(routes::match_skills));
    }

    // Rate-limit middleware — requires RealIpLayer with proxy headers (X-Forwarded-For, X-Real-IP).
    // Without proxy headers, RealIp can't extract client IP → rate limiter fails.
    // Enable with RATE_LIMIT=1 when behind a reverse proxy (nginx, cloudflare, etc.)
    let rate_limit_layer = if std::env::var("RATE_LIMIT").is_ok() {
        Some(
            ServiceBuilder::new()
                .layer(RealIpLayer::default())
                .layer(GovernorLayer::new(auth::protected_governor_config()))
        )
    } else {
        tracing::warn!("RATE_LIMIT not set — rate limiting disabled (dev mode without proxy)");
        None
    };

    let mut protected = match rate_limit_layer {
        Some(layer) => protected.layer(layer),
        None => protected,
    }
    .route_layer(middleware::from_fn(auth::auth_middleware))
    .layer(axum::Extension(api_key.clone()));

    let app = Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .merge(auth::auth_router_with_state(state.clone()).layer(axum::Extension(api_key.clone())))
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

        tracing::info!("shutdown signal received");
        // Storage backend handles its own persistence (PostgresStore: auto-commit, MemoryStore: auto-save)
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;

    Ok(())
}

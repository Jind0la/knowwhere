use std::sync::Arc;
use tokio::sync::RwLock;
mod runtime;

use axum::middleware;
#[cfg(feature = "postgres-storage")]
use axum::routing::put;
use axum::routing::{delete, get, post};
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

use knowwhere_server::api::{auth, auth::ApiKey, docs::ApiDoc, routes, webhooks::DedupCache};
use knowwhere_server::connectors::frigate::FrigateConnector;
use knowwhere_server::connectors::store_external_event;
use knowwhere_server::embedding::router::EmbeddingRouter;
use knowwhere_server::embedding::{AudioProvider, ClipProvider};
use knowwhere_server::memory::events::InMemoryEventStore;
use knowwhere_server::memory::{DreamMode, GovernancePolicy};
#[cfg(feature = "postgres-storage")]
use knowwhere_server::storage::PostgresStore;
use lazy_limit::{init_rate_limiter, Duration, RuleConfig};
use runtime::{init_embedding_provider, rate_limit_mode_from_env, RateLimitMode};

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

/// Build the auth router with optional PostgresStore extension.
#[cfg(feature = "postgres-storage")]
fn auth_router_with_pg_store<S: Clone + Send + Sync + 'static>(
    state: S,
    api_key: auth::ApiKey,
    auth_state: auth::AuthState,
    pg_store: Option<Arc<PostgresStore>>,
) -> Router<S> {
    let mut router: Router<S> = Router::new()
        .route("/login", post(auth::login))
        .route("/refresh", post(auth::refresh))
        .route("/register", post(auth::register))
        .with_state(state)
        .layer(axum::Extension(api_key))
        .layer(axum::Extension(auth_state));
    if let Some(pg) = pg_store {
        router = router.layer(axum::Extension(pg));
    }
    router
}

#[cfg(not(feature = "postgres-storage"))]
fn auth_router_with_pg_store<S: Clone + Send + Sync + 'static>(
    state: S,
    api_key: auth::ApiKey,
    auth_state: auth::AuthState,
) -> Router<S> {
    Router::new()
        .route("/login", post(auth::login))
        .route("/refresh", post(auth::refresh))
        .route("/register", post(auth::register))
        .with_state(state)
        .layer(axum::Extension(api_key))
        .layer(axum::Extension(auth_state))
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
    )
    .await;

    #[cfg(feature = "postgres-storage")]
    let (store, pg_store_for_auth) = runtime::init_store().await?;
    #[cfg(not(feature = "postgres-storage"))]
    let store = runtime::init_store().await?;

    // Concrete store for VLM worker, schedulers, DreamMode, and FrigateConnector.
    let dream_store = store.clone();
    let connector_store = store.clone();
    let dream = DreamMode::new(dream_store.clone());

    let embedding = init_embedding_provider();

    tracing::info!(provider = embedding.name(), "embedding provider ready");

    // Build cross-modal embedding router for content-type based dispatch.
    let router = Arc::new(EmbeddingRouter::new(
        embedding.clone(),
        Arc::new(ClipProvider::new()),
        Arc::new(AudioProvider::new()),
    ));
    tracing::info!("cross-modal embedding router ready (text → Ollama, image → CLIP, audio → Whisper)");

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
    let trajectory_pool = runtime::init_trajectory_pool().await;

    // -- Dream Mode Audit Scheduler (quality monitoring, NOT summarization) --
    #[cfg(feature = "postgres-storage")]
    {
        use knowwhere_server::scheduler::{SchedulerConfig, AuditScheduler};
        let scheduler_config = SchedulerConfig::from_env();
        if scheduler_config.is_enabled() {
            let audit = AuditScheduler::new(
                store.clone(),
                trajectory_pool.clone(),
                scheduler_config.clone(),
            );
            audit.spawn();
            tracing::info!("Dream Mode audit scheduler started");
        } else {
            tracing::info!("Dream Mode scheduler disabled (DREAM_ENABLED=false)");
        }
    }
    #[cfg(not(feature = "postgres-storage"))]
    {
        tracing::info!("postgres-storage not enabled — audit scheduler skipped");
    }
    // Server-wide temporal_weight default from env, or None.
    // Per-query override via RetrieveFractalRequest.temporal_weight takes precedence.
    let temporal_weight: Option<f32> = std::env::var("KNOWWHERE_TEMPORAL_WEIGHT")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|w| w.clamp(0.0, 0.8));
    tracing::info!(
        ?temporal_weight,
        "temporal_weight config (set KNOWWHERE_TEMPORAL_WEIGHT to override, or POST /config/temporal_weight at runtime)"
    );

    // Server-wide default source-type weights for provenance-aware retrieval.
    // Overridable per-query via RetrieveFractalRequest.source_type_weights.
    // Loaded from env var first, then config file (see SourceTypeWeights::from_config).
    let default_source_type_weights =
        knowwhere_server::retrieval::source_weighting::SourceTypeWeights::from_config();
    tracing::info!(
        ?default_source_type_weights,
        "source_type_weights config (set KNOWWHERE_SOURCE_TYPE_WEIGHTS or KNOWWHERE_SOURCE_TYPE_WEIGHTS_FILE, or place source_weights.json in working directory)"
    );

    let state = routes::AppState {
        store: store.clone(),
        dream_store: store.clone(),
        dream,
        embedding,
        router: Some(router),
        governance_policy: Arc::new(RwLock::new(GovernancePolicy::default_policy())),
        events: InMemoryEventStore::new(),
        #[cfg(feature = "postgres-storage")]
        trajectory_pool,
        #[cfg(feature = "postgres-storage")]
        pg_store: pg_store_for_auth.clone(),
        #[cfg(feature = "reranker")]
        reranker: knowwhere_server::retrieval::cross_encoder::load_reranker(),
        frigate_dedup: DedupCache::new(),
        frigate_webhook_secret: std::env::var("FRIGATE_WEBHOOK_SECRET").ok(),
        homeassistant_dedup: DedupCache::new(),
        homeassistant_webhook_secret: std::env::var("HASS_WEBHOOK_SECRET").ok(),
        temporal_weight: Arc::new(RwLock::new(temporal_weight)),
        default_source_type_weights,
    };

    let api_key = ApiKey(std::env::var("KNOWWHERE_API_KEY").ok());

    // Auth state: holds both the static admin key and registered beta tester keys
    let auth_state = auth::AuthState {
        admin_key: Arc::new(RwLock::new(std::env::var("KNOWWHERE_API_KEY").ok())),
        #[cfg(feature = "postgres-storage")]
        pg_store: pg_store_for_auth.clone(),
        ..Default::default()
    };

    let protected = Router::new()
        .route("/auth/me", get(auth::me))
        .route("/embed", post(routes::embed_text))
        .route("/store_session", post(routes::store_session))
        .route("/store_session_batch", post(routes::store_session_batch))
        .route("/store_external", post(routes::store_external))
        .route("/memory/self_improve", post(routes::self_improve))
        .route("/retrieve/{id}", get(routes::retrieve))
        .route("/retrieve_fractal", post(routes::retrieve_fractal_safe))
        .route("/rerank", post(routes::rerank))
        .route("/chat/subconscious", post(routes::subconscious_chat))
        .route("/nodes/recent", get(routes::recent_nodes))
        .route("/nodes/purge_dummy", post(routes::purge_dummy))
        .route("/nodes/reembed_all", post(routes::reembed_all))
        .route("/maintenance/repair_embeddings", post(routes::repair_embeddings))
        .route("/nodes/{id}", delete(routes::delete_node))
        .route("/nodes/batch_delete", post(routes::batch_delete_nodes))
        .route("/nodes/deduplicate", post(routes::deduplicate_nodes))
        .route("/dream/status", get(routes::dream_status))
        .route("/distance-matrix", post(routes::distance_matrix))
        // -- System routes --
        .route("/events", get(routes::list_events))
        // -- Governance routes --
        .route("/governance/policy", get(routes::get_governance_policy))
        .route("/governance/policy", post(routes::update_governance_policy))
        // -- Runtime config routes --
        .route("/config/temporal_weight", get(routes::get_temporal_weight))
        .route("/config/temporal_weight", post(routes::update_temporal_weight))
        // -- Webhook routes --
        .route("/webhooks/frigate", post(routes::webhook_frigate))
        .route("/webhooks/homeassistant", post(routes::webhook_homeassistant))
        // -- Voice message routes --
        .route("/voice/upload", post(routes::voice_upload::upload_voice));
    #[cfg(feature = "postgres-storage")]
    let protected = protected.route("/entities", get(routes::entity_search));

    #[cfg(feature = "postgres-storage")]
    let protected = protected
        // -- postgres-storage features (trajectory + tiered context) --
        .route("/retrieval/runs", get(routes::list_retrieval_runs))
        .route("/retrieval/runs/{id}", get(routes::get_retrieval_run))
        .route(
            "/retrieval/runs/{id}/trajectory",
            get(routes::get_retrieval_trajectory),
        )
        .route("/memories/{id}/compact", post(routes::compact_memory))
        .route("/memories/{id}", get(routes::get_memory))
        .route("/conflicts", get(routes::list_conflicts))
        .route("/conflicts/{id}/resolve", post(routes::resolve_conflict))
        .route("/conflicts/auto-resolve", post(routes::auto_resolve_conflicts))
        // Energy decay routes (Ebbinghaus forgetting curve)
        .route(
            "/memories/{id}/energy/boost",
            post(routes::boost_memory_energy),
        )
        .route("/energy/low", get(routes::list_low_energy_memories))
        .route("/energy/decay", post(routes::apply_energy_decay))
        .route("/energy/compress", post(routes::compress_memory_cluster))
        // Deduplication routes
        .route(
            "/deduplication/candidates",
            get(routes::list_deduplication_candidates),
        )
        .route("/deduplication/run", post(routes::run_deduplication))
        .route("/deduplication/runs", get(routes::list_deduplication_runs))
        // Self-healing routes (content hashing for external nodes)
        .route(
            "/memories/{id}/reindex",
            post(routes::reindex_external_node),
        )
        .route("/memories/{id}/health", get(routes::memory_health_check))
        .route("/self-healing/stats", get(routes::self_healing_stats))
        // Namespace routes
        .route("/namespaces", get(routes::list_namespaces))
        .route("/namespaces", post(routes::create_namespace))
        .route("/namespaces/{path}", get(routes::get_namespace))
        .route(
            "/namespaces/{path}/memories",
            get(routes::namespace_memories),
        )
        .route("/namespaces/{path}/search", get(routes::namespace_search))
        // Skills routes
        .route("/skills", post(routes::create_skill))
        .route("/skills", get(routes::list_skills))
        .route("/skills/{id}", get(routes::get_skill))
        .route("/skills/{id}", put(routes::update_skill))
        .route("/skills/{id}", delete(routes::delete_skill))
        .route("/skills/{id}/use", post(routes::use_skill))
        .route("/skills/match", get(routes::match_skills))
        // Turn-level routes (per-turn embedding pipeline)
        .route("/store_turn", post(routes::store_turn))
        .route("/store_turns", post(routes::store_turns_batch))
        .route("/retrieve/turns", post(routes::retrieve_turns))
        .route("/sessions/{session_id}/turns", get(routes::get_session_turns));

    // RATE_LIMIT_MODE=proxy enables IP-based limiting behind reverse proxies.
    // Backward compatibility: RATE_LIMIT=1 behaves like RATE_LIMIT_MODE=proxy.
    let rate_limit_mode = rate_limit_mode_from_env();
    let rate_limit_layer = if rate_limit_mode == RateLimitMode::Proxy {
        tracing::info!("rate limiting enabled (proxy mode, requires X-Forwarded-For or X-Real-IP)");
        Some(
            ServiceBuilder::new()
                .layer(RealIpLayer::default())
                .layer(GovernorLayer::new(auth::protected_governor_config())),
        )
    } else {
        tracing::warn!(
            "rate limiting disabled (set RATE_LIMIT_MODE=proxy when running behind a reverse proxy)"
        );
        None
    };

    let protected = match rate_limit_layer {
        Some(layer) => protected.layer(layer),
        None => protected,
    }
    .route_layer(middleware::from_fn(auth::auth_middleware))
    .layer(axum::Extension(auth_state.clone()));

    #[cfg(feature = "postgres-storage")]
    let auth_router = auth_router_with_pg_store(
        state.clone(),
        api_key.clone(),
        auth_state.clone(),
        pg_store_for_auth.clone(),
    );
    #[cfg(not(feature = "postgres-storage"))]
    let auth_router = auth_router_with_pg_store(state.clone(), api_key.clone(), auth_state.clone());

    let app = Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .merge(auth_router)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .fallback_service(ServeDir::new("frontend"))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
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

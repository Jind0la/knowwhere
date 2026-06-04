use utoipa::OpenApi;

use crate::api::{auth, routes};
use crate::memory::dream::DreamStatus;
use crate::memory::types::{MemorySource, MemoryType};
use crate::memory::{FractalNode, Relation};
use crate::multimodal::MultimodalData;

#[cfg(feature = "postgres-storage")]
use crate::memory::self_healing::{HealingStats, HealthCheckResult};

#[cfg(feature = "postgres-storage")]
use crate::api::routes::healing::ReindexResponse;

#[cfg(not(feature = "postgres-storage"))]
mod conditional_schemas {
    use super::*;

    #[derive(OpenApi)]
    #[openapi(
        info(title = "KnowWhere API", version = "0.1.0"),
        paths(
            routes::health,
            auth::me,
            routes::embed_text,
            routes::store_session,
            routes::store_external,
            routes::retrieve,
            routes::retrieve_fractal,
            routes::subconscious_chat,
            routes::recent_nodes,
            routes::delete_node,
            routes::purge_dummy,
            routes::reembed_all,
            routes::dream_status,
            routes::list_events,
            routes::get_governance_policy,
            routes::update_governance_policy,
            routes::webhook_frigate,
            routes::webhook_homeassistant,
        ),
        components(schemas(
            routes::HealthResponse,
            auth::AuthContext,
            auth::AuthTokenKind,
            routes::EmbedRequest,
            routes::EmbedResponse,
            routes::StoreSessionRequest,
            routes::StoreExternalRequest,
            routes::StoreNodeResponse,
            routes::PurgeResponse,
            routes::ReembedResponse,
            routes::RetrieveFractalRequest,
            routes::RetrievalScoreDebug,
            routes::SubconsciousChatRequest,
            routes::SubconsciousSource,
            routes::SubconsciousChatResponse,
            routes::ScoredNode,
            FractalNode,
            MemoryType,
            MemorySource,
            Relation,
            MultimodalData,
            DreamStatus,
            routes::UpdatePolicyRequest,
            routes::UpdatePolicyResponse,
            routes::FrigateWebhookEvent,
            routes::HomeAssistantWebhookPayload,
        ))
    )]
    pub struct ApiDoc {}
}

#[cfg(feature = "postgres-storage")]
mod conditional_schemas {
    use super::*;
    // Submodules referenced directly for utoipa __path_* type resolution
    use crate::api::routes::trajectory;
    use crate::api::routes::conflicts;
    use crate::api::routes::energy;
    use crate::api::routes::dedup;
    use crate::api::routes::healing;
    use crate::api::routes::namespaces;
    use crate::api::routes::skills_routes;
    use crate::api::routes::turn_handlers;

    #[derive(OpenApi)]
    #[openapi(
        info(title = "KnowWhere API", version = "0.1.0"),
        paths(
            routes::health,
            auth::me,
            routes::embed_text,
            routes::store_session,
            routes::store_external,
            routes::retrieve,
            routes::retrieve_fractal,
            routes::subconscious_chat,
            routes::recent_nodes,
            routes::delete_node,
            routes::purge_dummy,
            routes::reembed_all,
            routes::dream_status,
            routes::list_events,
            routes::get_governance_policy,
            routes::update_governance_policy,
            routes::webhook_frigate,
            routes::webhook_homeassistant,
            trajectory::list_retrieval_runs,
            trajectory::get_retrieval_run,
            trajectory::get_retrieval_trajectory,
            trajectory::compact_memory,
            trajectory::get_memory,
            conflicts::list_conflicts,
            conflicts::resolve_conflict,
            conflicts::auto_resolve_conflicts,
            energy::boost_memory_energy,
            energy::list_low_energy_memories,
            energy::apply_energy_decay,
            energy::compress_memory_cluster,
            dedup::list_deduplication_candidates,
            dedup::run_deduplication,
            dedup::list_deduplication_runs,
            healing::reindex_external_node,
            healing::memory_health_check,
            healing::self_healing_stats,
            namespaces::list_namespaces,
            namespaces::create_namespace,
            namespaces::get_namespace,
            namespaces::namespace_memories,
            namespaces::namespace_search,
            skills_routes::create_skill,
            skills_routes::list_skills,
            skills_routes::get_skill,
            skills_routes::update_skill,
            skills_routes::delete_skill,
            skills_routes::use_skill,
            skills_routes::match_skills,
            turn_handlers::store_turn,
            turn_handlers::store_turns_batch,
            turn_handlers::retrieve_turns,
            turn_handlers::get_session_turns,
        ),
        components(schemas(
            routes::HealthResponse,
            auth::AuthContext,
            auth::AuthTokenKind,
            routes::EmbedRequest,
            routes::EmbedResponse,
            routes::StoreSessionRequest,
            routes::StoreExternalRequest,
            routes::StoreNodeResponse,
            routes::PurgeResponse,
            routes::ReembedResponse,
            routes::RetrieveFractalRequest,
            routes::RetrievalScoreDebug,
            routes::SubconsciousChatRequest,
            routes::SubconsciousSource,
            routes::SubconsciousChatResponse,
            routes::ScoredNode,
            FractalNode,
            MemoryType,
            MemorySource,
            Relation,
            MultimodalData,
            DreamStatus,
            ReindexResponse,
            HealthCheckResult,
            HealingStats,
            routes::UpdatePolicyRequest,
            routes::UpdatePolicyResponse,
            routes::FrigateWebhookEvent,
            routes::HomeAssistantWebhookPayload,
        ))
    )]
    pub struct ApiDoc {}
}

pub use conditional_schemas::ApiDoc;

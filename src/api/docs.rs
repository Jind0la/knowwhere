use utoipa::OpenApi;

use crate::api::routes;
use crate::memory::dream::DreamStatus;
use crate::memory::types::{MemorySource, MemoryType};
use crate::memory::{FractalNode, Relation};
use crate::multimodal::MultimodalData;
use crate::vlm::VlmWorkerStatus;

#[cfg(feature = "postgres-storage")]
use crate::memory::self_healing::{HealingStats, HealthCheckResult};

#[cfg(feature = "postgres-storage")]
use crate::api::routes::ReindexResponse;

#[cfg(not(feature = "postgres-storage"))]
mod conditional_schemas {
    use super::*;

    #[derive(OpenApi)]
    #[openapi(
        info(title = "KnowWhere API", version = "0.1.0"),
        paths(
            routes::health,
            routes::embed_text,
            routes::store_session,
            routes::store_external,
            routes::retrieve,
            routes::retrieve_fractal,
            routes::recent_nodes,
            routes::delete_node,
            routes::purge_dummy,
            routes::reembed_all,
            routes::dream_status,
            routes::vlm_status,
            routes::vlm_enqueue,
            routes::list_events,
            routes::get_governance_policy,
            routes::update_governance_policy,
            routes::webhook_frigate,
        ),
        components(schemas(
            routes::HealthResponse,
            routes::EmbedRequest,
            routes::EmbedResponse,
            routes::StoreSessionRequest,
            routes::StoreExternalRequest,
            routes::StoreNodeResponse,
            routes::PurgeResponse,
            routes::ReembedResponse,
            routes::RetrieveFractalRequest,
            routes::ScoredNode,
            FractalNode,
            MemoryType,
            MemorySource,
            Relation,
            MultimodalData,
            DreamStatus,
            routes::VlmEnqueueRequest,
            routes::VlmEnqueueResponse,
            VlmWorkerStatus,
            routes::UpdatePolicyRequest,
            routes::UpdatePolicyResponse,
            routes::FrigateWebhookEvent,
        ))
    )]
    pub struct ApiDoc {}
}

#[cfg(feature = "postgres-storage")]
mod conditional_schemas {
    use super::*;

    #[derive(OpenApi)]
    #[openapi(
        info(title = "KnowWhere API", version = "0.1.0"),
        paths(
            routes::health,
            routes::embed_text,
            routes::store_session,
            routes::store_external,
            routes::retrieve,
            routes::retrieve_fractal,
            routes::recent_nodes,
            routes::delete_node,
            routes::purge_dummy,
            routes::reembed_all,
            routes::dream_status,
            routes::vlm_status,
            routes::vlm_enqueue,
            routes::list_events,
            routes::get_governance_policy,
            routes::update_governance_policy,
            routes::webhook_frigate,
            routes::list_retrieval_runs,
            routes::get_retrieval_run,
            routes::get_retrieval_trajectory,
            routes::compact_memory,
            routes::get_memory,
            routes::list_conflicts,
            routes::resolve_conflict,
            routes::boost_memory_energy,
            routes::list_low_energy_memories,
            routes::apply_energy_decay,
            routes::compress_memory_cluster,
            routes::list_deduplication_candidates,
            routes::run_deduplication,
            routes::list_deduplication_runs,
            routes::reindex_external_node,
            routes::memory_health_check,
            routes::self_healing_stats,
            routes::list_namespaces,
            routes::create_namespace,
            routes::get_namespace,
            routes::namespace_memories,
            routes::namespace_search,
            routes::create_skill,
            routes::list_skills,
            routes::get_skill,
            routes::update_skill,
            routes::delete_skill,
            routes::use_skill,
            routes::match_skills,
        ),
        components(schemas(
            routes::HealthResponse,
            routes::EmbedRequest,
            routes::EmbedResponse,
            routes::StoreSessionRequest,
            routes::StoreExternalRequest,
            routes::StoreNodeResponse,
            routes::PurgeResponse,
            routes::ReembedResponse,
            routes::RetrieveFractalRequest,
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
            routes::VlmEnqueueRequest,
            routes::VlmEnqueueResponse,
            VlmWorkerStatus,
            routes::UpdatePolicyRequest,
            routes::UpdatePolicyResponse,
            routes::FrigateWebhookEvent,
        ))
    )]
    pub struct ApiDoc {}
}

pub use conditional_schemas::ApiDoc;

use utoipa::OpenApi;

use crate::api::routes;
use crate::memory::dream::DreamStatus;
use crate::memory::{FractalNode, NodeType, Relation};
use crate::multimodal::MultimodalData;

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
            NodeType,
            Relation,
            MultimodalData,
            DreamStatus,
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
            NodeType,
            Relation,
            MultimodalData,
            DreamStatus,
            ReindexResponse,
            HealthCheckResult,
            HealingStats,
        ))
    )]
    pub struct ApiDoc {}
}

pub use conditional_schemas::ApiDoc;

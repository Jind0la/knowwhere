use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
#[cfg(feature = "postgres-storage")]
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api::auth::AuthContext;
use crate::api::turns::{BatchTurnItem, PaginatedSessionTurns, ScoredTurn, SessionTurn, SessionTurnsResponse, TurnContext};
use crate::api::webhooks::{check_webhook_secret, DedupCache};
use crate::embedding::router::EmbeddingRouter;
use crate::embedding::{embed_document, embed_document_batch, embed_query, EmbeddingProvider};
use crate::memory::dream::DreamStatus;
#[cfg(feature = "postgres-storage")]
use crate::memory::skills::CreateSkillResponse;
use crate::memory::types::{ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity};
use crate::memory::{
    DreamMode, Event, EventStore, FractalNode, GovernancePolicy, GovernanceValidator,
    InMemoryEventStore,
};
use crate::memory::fact_extraction::{FactExtractionContext, FactExtractor};
use crate::multimodal::MultimodalData;
use crate::storage::FusionStrategy;

#[path = "routes/governance_events.rs"]
mod governance_events;
pub use governance_events::*;
#[path = "routes/webhooks.rs"]
mod webhook_routes;
pub use webhook_routes::*;

#[path = "health.rs"]
mod health;
pub use health::*;

#[path = "store.rs"]
mod store;
pub use store::*;

#[path = "retrieve.rs"]
mod retrieve;
pub use retrieve::*;

#[path = "rerank.rs"]
mod rerank;
pub use rerank::*;

#[path = "maintenance.rs"]
mod maintenance;
pub use maintenance::*;

use crate::api::subconscious_qa::{
    is_multi_session_type, is_temporal_question, openai_qa_answer, qa_answer, qa_context_limit,
    source_context_block, source_timestamp,
};
use crate::api::types::*;

pub use crate::api::types::{
    clean_for_embedding, AppState, RetrievalScoreDebug, ScoredNode,
};




#[path = "trajectory.rs"]
mod trajectory;
pub use trajectory::*;

#[path = "conflicts.rs"]
mod conflicts;
pub use conflicts::*;

#[path = "energy.rs"]
mod energy;
pub use energy::*;

#[path = "dedup.rs"]
mod dedup;
pub use dedup::*;

#[path = "healing.rs"]
mod healing;
pub use healing::*;

#[path = "namespaces.rs"]
mod namespaces;
pub use namespaces::*;

#[path = "skills_routes.rs"]
mod skills_routes;
pub use skills_routes::*;

#[path = "turn_handlers.rs"]
mod turn_handlers;
pub use turn_handlers::*;

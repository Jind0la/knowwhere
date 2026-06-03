#[cfg(feature = "postgres-storage")]
use std::path::PathBuf;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api::webhooks::check_webhook_secret;
#[cfg(feature = "postgres-storage")]
use crate::memory::skills::CreateSkillResponse;
use crate::memory::types::Sensitivity;
use crate::memory::{
    Event, EventStore, GovernancePolicy,
};
use crate::multimodal::MultimodalData;

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


pub use crate::api::types::{
    clean_for_embedding, AppState, RetrievalScoreDebug, ScoredNode,
};




#[path = "trajectory.rs"]
pub mod trajectory;

#[path = "conflicts.rs"]
pub mod conflicts;

#[path = "energy.rs"]
pub mod energy;

#[path = "dedup.rs"]
pub mod dedup;

#[path = "healing.rs"]
pub mod healing;

#[path = "namespaces.rs"]
pub mod namespaces;

#[path = "skills_routes.rs"]
pub mod skills_routes;
pub use skills_routes::*;

#[path = "turn_handlers.rs"]
pub mod turn_handlers;

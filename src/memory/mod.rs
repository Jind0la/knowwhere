pub mod agent;
pub mod control_room;
pub mod dream;
pub mod events;
pub mod fractal_node;
pub mod governance;
#[cfg(feature = "postgres-storage")]
pub mod tiered;
pub mod types;

#[cfg(feature = "postgres-storage")]
pub mod namespaces;

#[cfg(feature = "postgres-storage")]
pub mod self_healing;

#[cfg(feature = "postgres-storage")]
pub mod skills;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod governance_test;

pub use agent::{AgentId, AgentProvenance, AgentRegistry, AgentRole, AgentState, MemoryVisibility};
pub use control_room::{ControlRoom, ControlRoomSnapshot, LayerStats};
pub use dream::{audit, consolidation, DreamMode};
pub use events::{Event, EventStore, EventType, InMemoryEventStore};
#[allow(deprecated)]
pub use fractal_node::{cosine_similarity, FractalNode, NodeType, Relation};
pub use governance::{
    GovernanceCandidate, GovernancePolicy, GovernanceValidator, GovernedScoredNode,
};
#[cfg(feature = "postgres-storage")]
pub use tiered::TieredCompactionWorker;
pub use types::{ConflictState, ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity};

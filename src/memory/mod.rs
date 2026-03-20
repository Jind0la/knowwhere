pub mod dream;
pub mod events;
pub mod fractal_node;
pub mod governance;
pub mod tiered;
pub mod types;

#[cfg(feature = "postgres-storage")]
pub mod self_healing;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod governance_test;

pub use dream::{audit, consolidation, DreamMode};
pub use events::{Event, EventType, EventStore, InMemoryEventStore};
pub use fractal_node::{cosine_similarity, FractalNode, NodeType, Relation};
pub use governance::{GovernanceCandidate, GovernancePolicy, GovernanceValidator, GovernedScoredNode};
pub use tiered::TieredCompactionWorker;
pub use types::{ConflictState, ContextTier, MemorySource, MemoryStatus, MemoryType, Sensitivity};

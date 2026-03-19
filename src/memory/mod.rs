pub mod dream;
pub mod events;
pub mod fractal_node;
pub mod governance;
pub mod types;

#[cfg(test)]
mod tests;

pub use dream::{audit, consolidation, DreamMode};
pub use events::{Event, EventType, EventStore};
pub use fractal_node::{cosine_similarity, FractalNode, NodeType, Relation};
pub use governance::{GovernanceCandidate, GovernancePolicy, GovernanceValidator, GovernedScoredNode};
pub use types::{MemorySource, MemoryStatus, MemoryType, Sensitivity, ConflictState};

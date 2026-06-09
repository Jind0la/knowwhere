//! Multi-Agent Identity & Provenance — Layer 5
//!
//! Implements agent identity tracking for multi-agent orchestration.
//! Each agent has a unique ID, role, and capability set.
//! Memories can be tagged with agent provenance for audit trails.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Multi-Agent Orchestration" + "Provenance Tracking"

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use utoipa::ToSchema;
use uuid::Uuid;

// -----------------------------------------------------------------------------
// AgentId — strongly-typed agent identifier
// -----------------------------------------------------------------------------

/// A strongly-typed agent identifier.
///
/// Distinct from a memory node UUID — this identifies the *agent*
/// that created or owns memories, not the memories themselves.
///
/// Stored in FractalNode metadata as:
/// ```json
/// {"agent_id": "550e8400-e29b-41d4-a716-446655440000", "visibility": "shared"}
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Generate a new random agent ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse from a string UUID.
    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }

    /// Return the metadata key used to tag FractalNodes.
    pub const METADATA_KEY: &'static str = "agent_id";

    /// Return the metadata key for provenance tracking.
    pub const PROVENANCE_KEY: &'static str = "provenance";
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// -----------------------------------------------------------------------------
// AgentRole — what this agent does
// -----------------------------------------------------------------------------

/// The role an agent plays in the multi-agent system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    /// Human user — the source of goals and preferences.
    User,
    /// Planner/Orchestrator — decomposes work and delegates.
    Orchestrator,
    /// Executor — performs tasks and reports results.
    Worker,
    /// Reviewer — validates and approves work.
    Reviewer,
    /// Observer — reads shared state but doesn't write.
    Observer,
    /// System — automated processes (Dream Mode, consolidation, etc.).
    System,
}

impl AgentRole {
    pub fn label(&self) -> &'static str {
        match self {
            AgentRole::User => "User",
            AgentRole::Orchestrator => "Orchestrator",
            AgentRole::Worker => "Worker",
            AgentRole::Reviewer => "Reviewer",
            AgentRole::Observer => "Observer",
            AgentRole::System => "System",
        }
    }

    /// Whether this role can write to the shared layer.
    pub fn can_write_shared(&self) -> bool {
        matches!(
            self,
            AgentRole::User | AgentRole::Orchestrator | AgentRole::Worker | AgentRole::System
        )
    }

    /// Whether this role can read the shared layer.
    pub fn can_read_shared(&self) -> bool {
        true // All roles can read shared
    }

    /// Whether this role can write to its own private layer.
    pub fn can_write_private(&self) -> bool {
        matches!(
            self,
            AgentRole::User
                | AgentRole::Orchestrator
                | AgentRole::Worker
                | AgentRole::Reviewer
                | AgentRole::System
        )
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "user" => Some(AgentRole::User),
            "orchestrator" => Some(AgentRole::Orchestrator),
            "worker" => Some(AgentRole::Worker),
            "reviewer" => Some(AgentRole::Reviewer),
            "observer" => Some(AgentRole::Observer),
            "system" => Some(AgentRole::System),
            _ => None,
        }
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// -----------------------------------------------------------------------------
// AgentState — full agent identity
// -----------------------------------------------------------------------------

/// The complete state of a registered agent in the multi-agent system.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentState {
    /// Unique agent identifier.
    pub id: AgentId,
    /// Human-readable name (e.g. "backend-eng", "qa-engineer").
    pub name: String,
    /// The role this agent plays.
    pub role: AgentRole,
    /// Optional set of capability tags (e.g. ["rust", "embedding", "sql"]).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// When this agent was registered.
    pub registered_at: DateTime<Utc>,
    /// When this agent was last active.
    pub last_active: DateTime<Utc>,
    /// Whether this agent is currently active.
    pub active: bool,
}

impl AgentState {
    /// Create a new agent with the given name and role.
    pub fn new(name: impl Into<String>, role: AgentRole, capabilities: Vec<String>) -> Self {
        let now = Utc::now();
        Self {
            id: AgentId::new(),
            name: name.into(),
            role,
            capabilities,
            registered_at: now,
            last_active: now,
            active: true,
        }
    }

    /// Touch the last_active timestamp.
    pub fn touch(&mut self) {
        self.last_active = Utc::now();
    }

    /// Check if this agent has a specific capability.
    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }
}

// -----------------------------------------------------------------------------
// AgentRegistry — manages agent lifecycle
// -----------------------------------------------------------------------------

/// Thread-safe registry of all agents in the system.
///
/// Agents are registered at startup or dynamically. The registry tracks
/// which agents exist, their roles, and their current status.
#[derive(Clone)]
pub struct AgentRegistry {
    agents: Arc<RwLock<HashMap<AgentId, AgentState>>>,
}

impl AgentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new agent.
    pub async fn register(&self, agent: AgentState) -> AgentId {
        let id = agent.id;
        self.agents.write().await.insert(id, agent);
        tracing::info!(agent_id = %id, "agent registered");
        id
    }

    /// Deregister an agent.
    pub async fn deregister(&self, id: &AgentId) -> bool {
        let removed = self.agents.write().await.remove(id).is_some();
        if removed {
            tracing::info!(agent_id = %id, "agent deregistered");
        }
        removed
    }

    /// Look up an agent by ID.
    pub async fn get(&self, id: &AgentId) -> Option<AgentState> {
        self.agents.read().await.get(id).cloned()
    }

    /// List all registered agents.
    pub async fn list(&self) -> Vec<AgentState> {
        self.agents.read().await.values().cloned().collect()
    }

    /// List all active agents.
    pub async fn list_active(&self) -> Vec<AgentState> {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.active)
            .cloned()
            .collect()
    }

    /// Find an agent by name.
    pub async fn find_by_name(&self, name: &str) -> Option<AgentState> {
        self.agents
            .read()
            .await
            .values()
            .find(|a| a.name == name)
            .cloned()
    }

    /// Touch an agent's last_active timestamp.
    pub async fn touch(&self, id: &AgentId) -> bool {
        if let Some(agent) = self.agents.write().await.get_mut(id) {
            agent.touch();
            true
        } else {
            false
        }
    }

    /// Count registered agents.
    pub async fn count(&self) -> usize {
        self.agents.read().await.len()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------------
// AgentProvenance — provenance metadata for memories
// -----------------------------------------------------------------------------

/// Tracks who created a memory and why.
///
/// Stored in FractalNode.provenance as a JSON value:
/// ```json
/// {
///   "agent_id": "550e8400-...",
///   "agent_name": "backend-eng",
///   "agent_role": "worker",
///   "reason": "Handoff from orchestration task t_abc123",
///   "parent_task": "t_abc123",
///   "visibility": "shared"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentProvenance {
    /// Which agent created this memory.
    pub agent_id: AgentId,
    /// Human-readable agent name.
    pub agent_name: String,
    /// The agent's role at creation time.
    pub agent_role: AgentRole,
    /// Why this memory was created (human-readable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Parent task ID (for orchestration handoffs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task: Option<String>,
    /// When the memory was created.
    pub created_at: DateTime<Utc>,
    /// The visibility of this memory.
    pub visibility: MemoryVisibility,
}

impl AgentProvenance {
    /// Create provenance from an AgentState.
    pub fn from_agent(agent: &AgentState, visibility: MemoryVisibility) -> Self {
        Self {
            agent_id: agent.id,
            agent_name: agent.name.clone(),
            agent_role: agent.role,
            reason: None,
            parent_task: None,
            created_at: Utc::now(),
            visibility,
        }
    }

    /// Set the reason for this provenance.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Set the parent task ID.
    pub fn with_parent_task(mut self, task_id: impl Into<String>) -> Self {
        self.parent_task = Some(task_id.into());
        self
    }

    /// Convert to serde_json::Value for storage in FractalNode.provenance.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Tag a metadata map with agent_id and visibility.
    pub fn tag_metadata(&self, metadata: &mut serde_json::Map<String, serde_json::Value>) {
        metadata.insert(
            AgentId::METADATA_KEY.to_string(),
            serde_json::Value::String(self.agent_id.to_string()),
        );
        metadata.insert(
            "visibility".to_string(),
            serde_json::Value::String(self.visibility.to_string()),
        );
    }
}

// -----------------------------------------------------------------------------
// MemoryVisibility — shared vs private
// -----------------------------------------------------------------------------

/// Whether a memory is visible to all agents or private to one.
///
/// This is the core mechanism for "No leakage" — private memories
/// are filtered out when another agent queries unless explicitly
/// shared via a handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MemoryVisibility {
    /// Visible to ALL agents in the system.
    /// Use for: architectural decisions, shared knowledge, handoff results.
    Shared,
    /// Visible only to the owning agent.
    /// Use for: agent-internal state, intermediate reasoning, private notes.
    #[default]
    Private,
    /// Visible to the owning agent + explicitly listed agents.
    /// Use for: targeted handoffs, pair programming, review workflows.
    Restricted {
        /// List of agent IDs that can access this memory.
        allowed_agents: Vec<AgentId>,
    },
}

impl MemoryVisibility {
    pub fn label(&self) -> &'static str {
        match self {
            MemoryVisibility::Shared => "shared",
            MemoryVisibility::Private => "private",
            MemoryVisibility::Restricted { .. } => "restricted",
        }
    }

    /// Whether the given agent can read this memory.
    pub fn is_accessible_by(&self, agent_id: &AgentId) -> bool {
        match self {
            MemoryVisibility::Shared => true,
            MemoryVisibility::Private => false,
            MemoryVisibility::Restricted { allowed_agents } => allowed_agents.contains(agent_id),
        }
    }

    /// Whether the owning agent can read this (always true for the owner).
    /// The caller must verify ownership separately.
    pub fn is_owner_accessible(&self) -> bool {
        true // Owner always has access
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "shared" => Some(MemoryVisibility::Shared),
            "private" => Some(MemoryVisibility::Private),
            _ => None,
        }
    }
}

impl std::fmt::Display for MemoryVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_roundtrip() {
        let id = AgentId::new();
        let s = id.to_string();
        let parsed = AgentId::parse(&s).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_agent_state_creation() {
        let agent = AgentState::new(
            "backend-eng",
            AgentRole::Worker,
            vec!["rust".into(), "sql".into()],
        );
        assert_eq!(agent.name, "backend-eng");
        assert_eq!(agent.role, AgentRole::Worker);
        assert!(agent.has_capability("rust"));
        assert!(!agent.has_capability("python"));
        assert!(agent.active);
    }

    #[tokio::test]
    async fn test_registry_register_and_lookup() {
        let registry = AgentRegistry::new();
        let agent = AgentState::new("test-agent", AgentRole::Worker, vec![]);
        let id = registry.register(agent).await;

        let found = registry.get(&id).await.unwrap();
        assert_eq!(found.name, "test-agent");
        assert_eq!(registry.count().await, 1);
    }

    #[tokio::test]
    async fn test_registry_deregister() {
        let registry = AgentRegistry::new();
        let agent = AgentState::new("temp", AgentRole::Observer, vec![]);
        let id = registry.register(agent).await;

        assert!(registry.deregister(&id).await);
        assert!(registry.get(&id).await.is_none());
        assert_eq!(registry.count().await, 0);
    }

    #[test]
    fn test_visibility_shared_accessible_by_all() {
        let vis = MemoryVisibility::Shared;
        let any_agent = AgentId::new();
        assert!(vis.is_accessible_by(&any_agent));
    }

    #[test]
    fn test_visibility_private_not_accessible() {
        let vis = MemoryVisibility::Private;
        let any_agent = AgentId::new();
        assert!(!vis.is_accessible_by(&any_agent));
    }

    #[test]
    fn test_visibility_restricted_access() {
        let alice = AgentId::new();
        let bob = AgentId::new();
        let carol = AgentId::new();

        let vis = MemoryVisibility::Restricted {
            allowed_agents: vec![alice, bob],
        };

        assert!(vis.is_accessible_by(&alice));
        assert!(vis.is_accessible_by(&bob));
        assert!(!vis.is_accessible_by(&carol));
    }

    #[test]
    fn test_provenance_serialization() {
        let agent = AgentState::new("test", AgentRole::Worker, vec![]);
        let prov = AgentProvenance::from_agent(&agent, MemoryVisibility::Shared)
            .with_reason("Handoff from test")
            .with_parent_task("t_test_123");

        let json = serde_json::to_value(&prov).unwrap();
        assert_eq!(json["agent_name"], "test");
        assert_eq!(json["visibility"], "shared");
        assert_eq!(json["reason"], "Handoff from test");
        assert_eq!(json["parent_task"], "t_test_123");
    }

    #[test]
    fn test_agent_role_permissions() {
        // Orchestrator and Worker can write shared
        assert!(AgentRole::Orchestrator.can_write_shared());
        assert!(AgentRole::Worker.can_write_shared());
        assert!(AgentRole::User.can_write_shared());
        assert!(AgentRole::System.can_write_shared());

        // Reviewer and Observer cannot write shared
        assert!(!AgentRole::Reviewer.can_write_shared());
        assert!(!AgentRole::Observer.can_write_shared());

        // Observer cannot write private either
        assert!(!AgentRole::Observer.can_write_private());
    }
}

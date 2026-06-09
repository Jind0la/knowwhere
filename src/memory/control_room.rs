//! Control Room — Shared Memory Layer for Multi-Agent Coordination
//!
//! The Control Room is the central coordination point for multi-agent
//! memory sharing. It manages:
//!
//! - **Shared Layer**: Memories visible to all agents (architectural decisions,
//!   handoff results, governance policies)
//! - **Private Layer**: Per-agent isolation — each agent has private memory
//!   that other agents cannot read
//! - **Handoff Protocol**: Agent A → Agent B memory transfer with provenance
//!
//! # Security Guarantee: No Leakage
//!
//! Private memories are filtered at the storage layer. Agent A's private
//! memories are NEVER visible to Agent B unless explicitly shared via
//! `handoff()`. This is enforced in `query_scoped()`.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Multi-Agent Memory Architecture" + "Control Room Protocol"

use crate::memory::agent::{AgentId, AgentProvenance, AgentRegistry, MemoryVisibility};
use crate::memory::types::MemoryType;
use crate::memory::FractalNode;
use crate::storage::backend::{HybridQuery, ScoredNode, StorageBackend};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use utoipa::ToSchema;
use uuid::Uuid;

// -----------------------------------------------------------------------------
// ControlRoom — the central coordinator
// -----------------------------------------------------------------------------

/// The Control Room coordinates multi-agent memory access.
///
/// It wraps the storage backend and enforces visibility rules:
/// - Shared memories: all agents can read
/// - Private memories: only the owning agent can read
/// - Restricted memories: only explicitly allowed agents can read
///
/// # Example
///
/// ```rust,ignore
/// let room = ControlRoom::new(store, registry);
///
/// // Agent A stores a private memory
/// room.store_private(&agent_a, content, vector).await?;
///
/// // Agent B cannot see it
/// let results = room.query_scoped(&agent_b, query).await?;
/// // results does NOT contain agent A's private memory
///
/// // Agent A hands off to Agent B
/// room.handoff(&agent_a, &agent_b, memory_id, "Review this").await?;
/// // Now Agent B can see it
/// ```
#[derive(Clone)]
pub struct ControlRoom {
    store: Arc<dyn StorageBackend>,
    registry: AgentRegistry,
    /// Cache of which agent owns which memory (agent_id → memory_ids).
    /// In production this would be queried from the store; here we use
    /// metadata-based tagging.
    ownership_cache: Arc<RwLock<HashMap<AgentId, Vec<Uuid>>>>,
}

impl ControlRoom {
    /// Create a new ControlRoom wrapping the given storage backend.
    pub fn new(store: Arc<dyn StorageBackend>, registry: AgentRegistry) -> Self {
        Self {
            store,
            registry,
            ownership_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // -------------------------------------------------------------------------
    // Store operations (visibility-aware)
    // -------------------------------------------------------------------------

    /// Store a memory in the shared layer — visible to all agents.
    pub async fn store_shared(
        &self,
        agent: &crate::memory::agent::AgentState,
        content: String,
        vector: Vec<f32>,
        memory_type: Option<MemoryType>,
    ) -> anyhow::Result<Uuid> {
        let mut metadata = serde_json::Map::new();
        let provenance = AgentProvenance::from_agent(agent, MemoryVisibility::Shared)
            .with_reason("Shared store");
        provenance.tag_metadata(&mut metadata);

        let memory_type = memory_type.unwrap_or(MemoryType::Semantic);
        let source = crate::memory::types::MemorySource::Conversation;

        let node = FractalNode::new_typed(
            Some(content),
            None,
            vector,
            metadata
                .into_iter()
                .collect::<HashMap<String, serde_json::Value>>(),
            memory_type,
            source,
        );

        let mut node = node;
        node.provenance = provenance.to_value();

        let id = self.store.insert(node).await?;
        tracing::info!(
            agent = %agent.name,
            memory_id = %id,
            visibility = "shared",
            "memory stored in shared layer"
        );
        Ok(id)
    }

    /// Store a memory in the agent's private layer — visible only to them.
    pub async fn store_private(
        &self,
        agent: &crate::memory::agent::AgentState,
        content: String,
        vector: Vec<f32>,
        memory_type: Option<MemoryType>,
    ) -> anyhow::Result<Uuid> {
        let mut metadata = serde_json::Map::new();
        let provenance = AgentProvenance::from_agent(agent, MemoryVisibility::Private)
            .with_reason("Private store");
        provenance.tag_metadata(&mut metadata);

        let memory_type = memory_type.unwrap_or(MemoryType::Episodic);
        let source = crate::memory::types::MemorySource::Conversation;

        let node = FractalNode::new_typed(
            Some(content),
            None,
            vector,
            metadata
                .into_iter()
                .collect::<HashMap<String, serde_json::Value>>(),
            memory_type,
            source,
        );

        let mut node = node;
        node.provenance = provenance.to_value();

        let id = self.store.insert(node).await?;

        // Update ownership cache
        self.ownership_cache
            .write()
            .await
            .entry(agent.id)
            .or_default()
            .push(id);

        tracing::info!(
            agent = %agent.name,
            memory_id = %id,
            visibility = "private",
            "memory stored in private layer"
        );
        Ok(id)
    }

    /// Store a memory with restricted visibility — visible to owner + listed agents.
    pub async fn store_restricted(
        &self,
        agent: &crate::memory::agent::AgentState,
        content: String,
        vector: Vec<f32>,
        allowed_agents: Vec<AgentId>,
        memory_type: Option<MemoryType>,
    ) -> anyhow::Result<Uuid> {
        let vis = MemoryVisibility::Restricted {
            allowed_agents: allowed_agents.clone(),
        };
        let mut metadata = serde_json::Map::new();
        let provenance = AgentProvenance::from_agent(agent, vis).with_reason("Restricted store");
        provenance.tag_metadata(&mut metadata);

        // Also store the allowed_agents list in metadata for retrieval filtering
        let allowed_json: Vec<String> = allowed_agents.iter().map(|a| a.to_string()).collect();
        metadata.insert(
            "allowed_agents".to_string(),
            serde_json::Value::Array(
                allowed_json
                    .iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
        );

        let memory_type = memory_type.unwrap_or(MemoryType::Semantic);
        let source = crate::memory::types::MemorySource::Conversation;

        let node = FractalNode::new_typed(
            Some(content),
            None,
            vector,
            metadata
                .into_iter()
                .collect::<HashMap<String, serde_json::Value>>(),
            memory_type,
            source,
        );

        let mut node = node;
        node.provenance = provenance.to_value();

        let id = self.store.insert(node).await?;
        tracing::info!(
            agent = %agent.name,
            memory_id = %id,
            allowed_count = allowed_agents.len(),
            "memory stored with restricted visibility"
        );
        Ok(id)
    }

    // -------------------------------------------------------------------------
    // Query operations (visibility-filtered)
    // -------------------------------------------------------------------------

    /// Query memories visible to a specific agent.
    ///
    /// This is the core security guarantee: **private memories from other
    /// agents are filtered out**. The agent sees:
    /// 1. All shared memories
    /// 2. Their own private memories
    /// 3. Restricted memories where they're listed
    pub async fn query_scoped(
        &self,
        agent: &crate::memory::agent::AgentState,
        query: HybridQuery,
    ) -> anyhow::Result<Vec<ScoredNode>> {
        // Step 1: Retrieve all candidates (no visibility filter yet)
        let candidates = self.store.hybrid_retrieve(&query).await?;

        // Step 2: Filter by visibility
        let filtered: Vec<ScoredNode> = candidates
            .into_iter()
            .filter(|scored| self.is_visible_to(agent, &scored.node))
            .collect();

        tracing::debug!(
            agent = %agent.name,
            total = filtered.len(),
            "scoped query filtered"
        );

        Ok(filtered)
    }

    /// Query ONLY the shared layer.
    pub async fn query_shared(&self, query: HybridQuery) -> anyhow::Result<Vec<ScoredNode>> {
        let candidates = self.store.hybrid_retrieve(&query).await?;
        let filtered: Vec<ScoredNode> = candidates
            .into_iter()
            .filter(|scored| self.get_visibility(&scored.node) == Some(MemoryVisibility::Shared))
            .collect();
        Ok(filtered)
    }

    /// Query ONLY the agent's private layer.
    pub async fn query_private(
        &self,
        agent: &crate::memory::agent::AgentState,
        query: HybridQuery,
    ) -> anyhow::Result<Vec<ScoredNode>> {
        let candidates = self.store.hybrid_retrieve(&query).await?;
        let filtered: Vec<ScoredNode> = candidates
            .into_iter()
            .filter(|scored| self.is_private_of(agent, &scored.node))
            .collect();
        Ok(filtered)
    }

    // -------------------------------------------------------------------------
    // Handoff Protocol — transfer memory between agents
    // -------------------------------------------------------------------------

    /// Hand off a memory from one agent to another.
    ///
    /// After handoff, the memory becomes visible to the target agent.
    /// The original provenance is preserved, and a handoff event is recorded.
    ///
    /// This is the primary mechanism for cross-agent collaboration:
    /// Orchestrator → Worker: task assignment
    /// Worker → Reviewer: work for review
    /// Worker → Orchestrator: results
    pub async fn handoff(
        &self,
        from: &crate::memory::agent::AgentState,
        to: &crate::memory::agent::AgentState,
        memory_id: Uuid,
        reason: impl Into<String>,
    ) -> anyhow::Result<bool> {
        let reason = reason.into();

        // Get the original node
        let node = self.store.get(&memory_id).await?;
        let Some(mut node) = node else {
            tracing::warn!(memory_id = %memory_id, "handoff failed: memory not found");
            return Ok(false);
        };

        // Verify the from-agent owns or has access to this memory
        if !self.is_visible_to(from, &node) {
            tracing::warn!(
                from = %from.name,
                memory_id = %memory_id,
                "handoff failed: agent does not own this memory"
            );
            return Ok(false);
        }

        // Update visibility to restricted (owner + target)
        let new_vis = MemoryVisibility::Restricted {
            allowed_agents: vec![to.id],
        };

        // Update provenance to record the handoff
        let mut prov: AgentProvenance = serde_json::from_value(node.provenance.clone())
            .unwrap_or_else(|_| AgentProvenance::from_agent(from, new_vis.clone()));
        prov.visibility = new_vis.clone();
        prov.reason = Some(format!("Handoff {} → {}: {}", from.name, to.name, reason));
        prov.parent_task = Some(format!("handoff:{}→{}", from.id, to.id));

        // Update node metadata
        node.metadata.insert(
            AgentId::METADATA_KEY.to_string(),
            serde_json::Value::String(to.id.to_string()),
        );
        node.metadata.insert(
            "visibility".to_string(),
            serde_json::Value::String(new_vis.to_string()),
        );
        node.metadata.insert(
            "handoff_from".to_string(),
            serde_json::Value::String(from.id.to_string()),
        );
        node.metadata.insert(
            "handoff_to".to_string(),
            serde_json::Value::String(to.id.to_string()),
        );
        node.provenance = serde_json::to_value(&prov).unwrap_or_default();

        // Persist the updated node

        // We need to re-insert with new metadata — update doesn't support metadata mutation
        // Workaround: we'll store the handoff info by updating the content
        // Actually, let's use a different approach — store the handoff as a new derived node
        // that references the original

        // For now: create a handoff copy with restricted visibility.
        // Include handoff metadata directly in the stored node.
        let handoff_content = format!(
            "[HANDOFF {} → {}] {}",
            from.name,
            to.name,
            node.content.as_deref().unwrap_or("(no content)")
        );

        // Build the node manually so we can attach handoff metadata
        let mut handoff_metadata = serde_json::Map::new();
        let provenance = AgentProvenance::from_agent(
            from,
            MemoryVisibility::Restricted {
                allowed_agents: vec![to.id],
            },
        )
        .with_reason(format!("Handoff {} → {}: {}", from.name, to.name, reason))
        .with_parent_task(format!("handoff:{}→{}", from.id, to.id));
        provenance.tag_metadata(&mut handoff_metadata);
        handoff_metadata.insert(
            "handoff_from".to_string(),
            serde_json::Value::String(from.id.to_string()),
        );
        handoff_metadata.insert(
            "handoff_to".to_string(),
            serde_json::Value::String(to.id.to_string()),
        );
        handoff_metadata.insert(
            "handoff_original".to_string(),
            serde_json::Value::String(memory_id.to_string()),
        );
        handoff_metadata.insert(
            "allowed_agents".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::String(to.id.to_string())]),
        );

        let handoff_node = FractalNode::new_typed(
            Some(handoff_content),
            None,
            node.vector.clone(),
            handoff_metadata
                .into_iter()
                .collect::<HashMap<String, serde_json::Value>>(),
            node.memory_type,
            crate::memory::types::MemorySource::Conversation,
        );

        let mut handoff_node = handoff_node;
        handoff_node.provenance = serde_json::to_value(&provenance).unwrap_or_default();

        let handoff_id = self.store.insert(handoff_node).await?;

        tracing::info!(
            from = %from.name,
            to = %to.name,
            original_id = %memory_id,
            handoff_id = %handoff_id,
            reason = %reason,
            "handoff complete"
        );

        Ok(true)
    }

    // -------------------------------------------------------------------------
    // Orchestration Helpers
    // -------------------------------------------------------------------------

    /// Get the list of all memories visible to an agent.
    pub async fn list_visible(
        &self,
        agent: &crate::memory::agent::AgentState,
        limit: usize,
    ) -> anyhow::Result<Vec<FractalNode>> {
        let all = self.store.list_all().await?;
        let visible: Vec<FractalNode> = all
            .into_iter()
            .filter(|node| self.is_visible_to(agent, node))
            .take(limit)
            .collect();
        Ok(visible)
    }

    /// Get stats about the memory layers for an agent.
    pub async fn stats(
        &self,
        agent: &crate::memory::agent::AgentState,
    ) -> anyhow::Result<LayerStats> {
        let all = self.store.list_all().await?;
        let total = all.len();
        let shared = all
            .iter()
            .filter(|n| self.get_visibility(n) == Some(MemoryVisibility::Shared))
            .count();
        let private = all.iter().filter(|n| self.is_private_of(agent, n)).count();
        let other_private = all
            .iter()
            .filter(|n| {
                self.get_visibility(n) == Some(MemoryVisibility::Private)
                    && !self.is_private_of(agent, n)
            })
            .count();
        let restricted = total - shared - private - other_private;

        Ok(LayerStats {
            total,
            shared,
            private_own: private,
            private_other: other_private,
            restricted,
        })
    }

    /// Get the underlying store (for direct access when needed).
    pub fn store(&self) -> &Arc<dyn StorageBackend> {
        &self.store
    }

    /// Get the agent registry.
    pub fn registry(&self) -> &AgentRegistry {
        &self.registry
    }

    // -------------------------------------------------------------------------
    // Visibility helpers
    // -------------------------------------------------------------------------

    /// Check if a node is visible to the given agent.
    fn is_visible_to(&self, agent: &crate::memory::agent::AgentState, node: &FractalNode) -> bool {
        match self.get_visibility(node) {
            None => {
                // No visibility tag → treat as shared (backward compatible)
                true
            }
            Some(MemoryVisibility::Shared) => true,
            Some(MemoryVisibility::Private) => {
                // Private: only visible to the owning agent
                self.is_owned_by(agent, node)
            }
            Some(MemoryVisibility::Restricted { allowed_agents }) => {
                // Restricted: visible to owner + allowed agents
                self.is_owned_by(agent, node) || allowed_agents.contains(&agent.id)
            }
        }
    }

    /// Check if a node is in the agent's private layer.
    fn is_private_of(&self, agent: &crate::memory::agent::AgentState, node: &FractalNode) -> bool {
        self.get_visibility(node) == Some(MemoryVisibility::Private)
            && self.is_owned_by(agent, node)
    }

    /// Check if the agent owns this node.
    fn is_owned_by(&self, agent: &crate::memory::agent::AgentState, node: &FractalNode) -> bool {
        node.metadata
            .get(AgentId::METADATA_KEY)
            .and_then(|v| v.as_str())
            .map(|id_str| id_str == agent.id.to_string())
            .unwrap_or(false)
    }

    /// Extract the visibility from a node's metadata.
    fn get_visibility(&self, node: &FractalNode) -> Option<MemoryVisibility> {
        let vis_str = node.metadata.get("visibility")?.as_str()?;

        // Try basic parse first
        if let Some(vis) = MemoryVisibility::parse(vis_str) {
            return Some(vis);
        }

        // Check for restricted with allowed_agents
        if vis_str == "restricted" {
            let allowed = node
                .metadata
                .get("allowed_agents")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| AgentId::parse(s).ok())
                        .collect()
                })
                .unwrap_or_default();
            return Some(MemoryVisibility::Restricted {
                allowed_agents: allowed,
            });
        }

        None
    }
}

// -----------------------------------------------------------------------------
// LayerStats — memory layer statistics
// -----------------------------------------------------------------------------

/// Statistics about the memory layers visible to an agent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LayerStats {
    /// Total memories in the system.
    pub total: usize,
    /// Memories in the shared layer.
    pub shared: usize,
    /// Agent's own private memories.
    pub private_own: usize,
    /// Other agents' private memories (invisible to this agent).
    pub private_other: usize,
    /// Restricted-visibility memories.
    pub restricted: usize,
}

// -----------------------------------------------------------------------------
// ControlRoomSnapshot — serializable state for demo/testing
// -----------------------------------------------------------------------------

/// A snapshot of the Control Room state for testing and verification.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ControlRoomSnapshot {
    /// Number of registered agents.
    pub agent_count: usize,
    /// Layer statistics per agent.
    pub layer_stats: HashMap<String, LayerStats>,
    /// Total memories in the store.
    pub total_memories: usize,
}

impl ControlRoom {
    /// Create a snapshot of the current Control Room state.
    pub async fn snapshot(&self) -> anyhow::Result<ControlRoomSnapshot> {
        let agents = self.registry.list().await;
        let mut layer_stats = HashMap::new();

        for agent in &agents {
            let stats = self.stats(agent).await?;
            layer_stats.insert(agent.name.clone(), stats);
        }

        Ok(ControlRoomSnapshot {
            agent_count: agents.len(),
            layer_stats,
            total_memories: self.store.count().await,
        })
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::agent::{AgentRole, AgentState};
    use crate::storage::in_memory::MemoryStore;

    fn zero_vector() -> Vec<f32> {
        vec![0.0f32; 768]
    }

    async fn setup() -> (ControlRoom, AgentState, AgentState) {
        let store = Arc::new(MemoryStore::new());
        let registry = AgentRegistry::new();

        let agent_a = AgentState::new("agent-a", AgentRole::Worker, vec!["rust".into()]);
        let agent_b = AgentState::new("agent-b", AgentRole::Reviewer, vec!["qa".into()]);

        registry.register(agent_a.clone()).await;
        registry.register(agent_b.clone()).await;

        let room = ControlRoom::new(store, registry);
        (room, agent_a, agent_b)
    }

    #[tokio::test]
    async fn test_shared_visible_to_all() {
        let (room, agent_a, agent_b) = setup().await;

        // Agent A stores shared memory
        let id = room
            .store_shared(&agent_a, "Shared knowledge".into(), zero_vector(), None)
            .await
            .unwrap();

        // Both agents can see it
        let query = HybridQuery::text("Shared knowledge", 10).with_recency_boost(0.0);
        let results_a = room.query_scoped(&agent_a, query.clone()).await.unwrap();
        let results_b = room.query_scoped(&agent_b, query.clone()).await.unwrap();

        assert!(results_a.iter().any(|r| r.id == id));
        assert!(results_b.iter().any(|r| r.id == id));
    }

    #[tokio::test]
    async fn test_private_not_leaked() {
        let (room, agent_a, agent_b) = setup().await;

        // Agent A stores private memory
        let id = room
            .store_private(&agent_a, "Secret plan".into(), zero_vector(), None)
            .await
            .unwrap();

        // Agent A can see it
        let query = HybridQuery::text("Secret plan", 10).with_recency_boost(0.0);
        let results_a = room.query_scoped(&agent_a, query.clone()).await.unwrap();
        assert!(
            results_a.iter().any(|r| r.id == id),
            "Agent A should see own private memory"
        );

        // Agent B CANNOT see it (NO LEAKAGE)
        let results_b = room.query_scoped(&agent_b, query.clone()).await.unwrap();
        assert!(
            !results_b.iter().any(|r| r.id == id),
            "LEAK DETECTED: Agent B can see Agent A's private memory!"
        );
    }

    #[tokio::test]
    async fn test_handoff_makes_visible() {
        let (room, agent_a, agent_b) = setup().await;

        // Agent A stores private memory
        let id = room
            .store_private(&agent_a, "Work for review".into(), zero_vector(), None)
            .await
            .unwrap();

        // Agent B cannot see it yet — use list_visible for reliable visibility check
        let before = room.list_visible(&agent_b, 50).await.unwrap();
        assert!(!before.iter().any(|r| r.id == id));

        // Handoff from A to B
        let success = room
            .handoff(&agent_a, &agent_b, id, "Please review this")
            .await
            .unwrap();
        assert!(success);

        // Now Agent B CAN see the handoff copy
        let after = room.list_visible(&agent_b, 50).await.unwrap();
        assert!(
            after.iter().any(|r| {
                r.metadata.get("handoff_to").and_then(|v| v.as_str())
                    == Some(&agent_b.id.to_string())
            }),
            "Handoff should make memory visible to target agent"
        );
    }

    #[tokio::test]
    async fn test_restricted_visibility() {
        let (room, agent_a, agent_b) = setup().await;
        let agent_c = AgentState::new("agent-c", AgentRole::Observer, vec![]);
        room.registry().register(agent_c.clone()).await;

        // Agent A stores restricted memory for B only
        let _id = room
            .store_restricted(
                &agent_a,
                "Restricted to B".into(),
                zero_vector(),
                vec![agent_b.id],
                None,
            )
            .await
            .unwrap();

        let query = HybridQuery::text("Restricted to B", 10).with_recency_boost(0.0);

        // Agent B can see it
        let results_b = room.query_scoped(&agent_b, query.clone()).await.unwrap();
        assert!(results_b
            .iter()
            .any(|r| { r.node.content.as_deref() == Some("Restricted to B") }));

        // Agent C cannot see it
        let results_c = room.query_scoped(&agent_c, query.clone()).await.unwrap();
        assert!(!results_c
            .iter()
            .any(|r| { r.node.content.as_deref() == Some("Restricted to B") }));
    }

    #[tokio::test]
    async fn test_layer_stats() {
        let (room, agent_a, agent_b) = setup().await;

        // Store mix of shared and private
        room.store_shared(&agent_a, "Shared 1".into(), zero_vector(), None)
            .await
            .unwrap();
        room.store_private(&agent_a, "Private A".into(), zero_vector(), None)
            .await
            .unwrap();
        room.store_private(&agent_b, "Private B".into(), zero_vector(), None)
            .await
            .unwrap();

        let stats = room.stats(&agent_a).await.unwrap();
        assert_eq!(stats.shared, 1);
        assert_eq!(stats.private_own, 1);
        assert_eq!(
            stats.private_other, 1,
            "Agent B's private memory should be counted as other"
        );
    }

    #[tokio::test]
    async fn test_snapshot() {
        let (room, agent_a, _agent_b) = setup().await;

        room.store_shared(&agent_a, "Test".into(), zero_vector(), None)
            .await
            .unwrap();

        let snap = room.snapshot().await.unwrap();
        assert_eq!(snap.agent_count, 2);
        assert!(snap.layer_stats.contains_key("agent-a"));
        assert!(snap.layer_stats.contains_key("agent-b"));
    }
}

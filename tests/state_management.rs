//! State Management Certification Tests
//!
//! Cert-style test cases for multi-agent orchestration, state management,
//! governance, and the core security guarantee: **no leakage between agents**.
//!
//! These tests form the certification suite for GH-600:
//! GitHub Certification for Multi-Agent State Management.
//!
//! # Test Categories
//!
//! 1. **Agent Identity & Registration** — CERT-AGENT
//! 2. **Shared Layer Integrity** — CERT-SHARED
//! 3. **Private Layer Isolation** — CERT-PRIVATE (the no-leakage tests)
//! 4. **Handoff Protocol** — CERT-HANDOFF
//! 5. **Restricted Visibility** — CERT-RESTRICTED
//! 6. **Provenance Audit Trail** — CERT-PROV
//! 7. **Orchestration Workflow** — CERT-ORCH
//! 8. **Governance Integration** — CERT-GOV
//! 9. **Stress: Multi-Agent Concurrent Access** — CERT-STRESS
//! 10. **Full Pipeline: Orchestrator → Worker → Reviewer** — CERT-E2E

use knowwhere_server::memory::agent::{AgentId, AgentRegistry, AgentRole, AgentState};
use knowwhere_server::memory::control_room::ControlRoom;
use knowwhere_server::memory::types::MemoryType;
use knowwhere_server::memory::FractalNode;
use knowwhere_server::storage::backend::{HybridQuery, StorageBackend};
use knowwhere_server::storage::in_memory::MemoryStore;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// Helper: create a zero-vector for tests (768-dim, matching nomic-embed-text)
fn zero_vector() -> Vec<f32> {
    vec![0.0f32; 768]
}

fn random_vector() -> Vec<f32> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..768).map(|_| rng.gen_range(-0.5..0.5)).collect()
}

async fn setup_room(num_agents: usize) -> (ControlRoom, Vec<AgentState>, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    let registry = AgentRegistry::new();
    let mut agents = Vec::new();

    let role_order = [
        AgentRole::User,
        AgentRole::Orchestrator,
        AgentRole::Worker,
        AgentRole::Reviewer,
        AgentRole::Observer,
        AgentRole::System,
    ];

    for i in 0..num_agents {
        let agent = AgentState::new(
            format!("agent-{}", i),
            role_order[i % role_order.len()],
            vec![format!("cap-{}", i % 3)],
        );
        registry.register(agent.clone()).await;
        agents.push(agent);
    }

    let room = ControlRoom::new(store.clone() as Arc<dyn StorageBackend>, registry);
    (room, agents, store)
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-AGENT: Agent Identity & Registration
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_agent_001_register_and_lookup() {
    let (room, agents, _) = setup_room(1).await;
    let agent = &agents[0];

    let found = room.registry().get(&agent.id).await.unwrap();
    assert_eq!(found.name, agent.name);
    assert_eq!(found.role, agent.role);
}

#[tokio::test]
async fn cert_agent_002_unique_ids() {
    let (_, agents, _) = setup_room(5).await;
    let mut ids: Vec<AgentId> = agents.iter().map(|a| a.id).collect();
    ids.sort_by_key(|id| id.0.to_string());
    ids.dedup();
    assert_eq!(ids.len(), 5, "All 5 agents must have unique IDs");
}

#[tokio::test]
async fn cert_agent_003_role_permissions() {
    // Orchestrator can write shared
    assert!(AgentRole::Orchestrator.can_write_shared());
    // Observer cannot write shared
    assert!(!AgentRole::Observer.can_write_shared());
    // Observer cannot write private
    assert!(!AgentRole::Observer.can_write_private());
    // Reviewer can write private
    assert!(AgentRole::Reviewer.can_write_private());
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-SHARED: Shared Layer Integrity
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_shared_001_all_agents_see_shared() {
    let (room, agents, _) = setup_room(3).await;

    // Agent 0 stores shared memory
    let id = room
        .store_shared(
            &agents[0],
            "Global architectural decision".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    let query = HybridQuery::text("architectural decision", 10).with_recency_boost(0.0);

    // ALL agents can see it
    for agent in &agents {
        let results = room.query_scoped(agent, query.clone()).await.unwrap();
        assert!(
            results.iter().any(|r| r.id == id),
            "Agent {} should see shared memory",
            agent.name
        );
    }
}

#[tokio::test]
async fn cert_shared_002_shared_layer_query_scoped() {
    let (room, agents, _) = setup_room(2).await;

    room.store_shared(&agents[0], "Shared A".into(), zero_vector(), None)
        .await
        .unwrap();
    room.store_shared(&agents[1], "Shared B".into(), zero_vector(), None)
        .await
        .unwrap();

    let shared_results = room
        .query_shared(HybridQuery::text("Shared", 10))
        .await
        .unwrap();
    assert_eq!(
        shared_results.len(),
        2,
        "Both shared memories should be in shared layer"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-PRIVATE: Private Layer Isolation — NO LEAKAGE
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_private_001_no_cross_agent_leakage() {
    // THE CORE SECURITY TEST: Agent B MUST NOT see Agent A's private memory
    let (room, agents, _) = setup_room(2).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];

    let secret_id = room
        .store_private(
            agent_a,
            "TOP SECRET: Agent A's private data".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    let query = HybridQuery::text("TOP SECRET", 10).with_recency_boost(0.0);

    // Agent A sees it
    let results_a = room.query_scoped(agent_a, query.clone()).await.unwrap();
    assert!(results_a.iter().any(|r| r.id == secret_id));

    // Agent B MUST NOT see it
    let results_b = room.query_scoped(agent_b, query.clone()).await.unwrap();
    assert!(
        !results_b.iter().any(|r| r.id == secret_id),
        "CERT FAILURE: LEAK DETECTED — Agent B can see Agent A's private memory!"
    );
}

#[tokio::test]
async fn cert_private_002_query_private_layer_isolation() {
    let (room, agents, _) = setup_room(2).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];

    room.store_private(agent_a, "A-private-1".into(), zero_vector(), None)
        .await
        .unwrap();
    room.store_private(agent_a, "A-private-2".into(), zero_vector(), None)
        .await
        .unwrap();
    room.store_private(agent_b, "B-private-1".into(), zero_vector(), None)
        .await
        .unwrap();

    let a_private = room
        .query_private(agent_a, HybridQuery::text("private", 10))
        .await
        .unwrap();
    let b_private = room
        .query_private(agent_b, HybridQuery::text("private", 10))
        .await
        .unwrap();

    assert_eq!(
        a_private.len(),
        2,
        "Agent A should see exactly 2 private memories"
    );
    assert_eq!(
        b_private.len(),
        1,
        "Agent B should see exactly 1 private memory"
    );

    // Verify no cross-contamination
    let a_contents: Vec<&str> = a_private
        .iter()
        .filter_map(|s| s.node.content.as_deref())
        .collect();
    assert!(a_contents.contains(&"A-private-1"));
    assert!(a_contents.contains(&"A-private-2"));
    assert!(
        !a_contents.contains(&"B-private-1"),
        "LEAK: Agent A sees B's private memory"
    );
}

#[tokio::test]
async fn cert_private_003_many_agents_no_leakage() {
    // Stress test: 10 agents, each with private data — verify zero leakage
    let (room, agents, _) = setup_room(10).await;

    // Each agent stores 3 private memories
    for agent in &agents {
        for j in 0..3 {
            room.store_private(
                agent,
                format!("{}-secret-{}", agent.name, j),
                zero_vector(),
                None,
            )
            .await
            .unwrap();
        }
    }

    // Verify each agent sees ONLY their own private memories
    for agent in &agents {
        let results = room
            .query_private(agent, HybridQuery::text("secret", 30))
            .await
            .unwrap();

        assert_eq!(
            results.len(),
            3,
            "Agent {} should see exactly 3 private memories",
            agent.name
        );

        for scored in &results {
            let content = scored.node.content.as_deref().unwrap_or("");
            assert!(
                content.contains(&agent.name),
                "LEAK: Agent {} sees memory '{}' that belongs to another agent!",
                agent.name,
                content
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-HANDOFF: Handoff Protocol
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_handoff_001_basic_transfer() {
    let (room, agents, _) = setup_room(2).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];

    let mem_id = room
        .store_private(
            agent_a,
            "Task result ready for review".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    // Before handoff: B cannot see it
    let query = HybridQuery::text("Task result", 10).with_recency_boost(0.0);
    let before = room.query_scoped(agent_b, query.clone()).await.unwrap();
    assert!(!before.iter().any(|r| r.id == mem_id));

    // Handoff
    let success = room
        .handoff(agent_a, agent_b, mem_id, "Review request")
        .await
        .unwrap();
    assert!(success);

    // After handoff: B can see the handoff copy
    let after = room.query_scoped(agent_b, query).await.unwrap();
    let has_handoff = after.iter().any(|r| {
        r.node.metadata.get("handoff_to").and_then(|v| v.as_str()) == Some(&agent_b.id.to_string())
    });
    assert!(
        has_handoff,
        "Handoff should make memory visible to target agent"
    );
}

#[tokio::test]
async fn cert_handoff_002_unauthorized_handoff_rejected() {
    let (room, agents, _) = setup_room(3).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];
    let agent_c = &agents[2];

    let mem_id = room
        .store_private(agent_a, "A's secret".into(), zero_vector(), None)
        .await
        .unwrap();

    // Agent C tries to hand off A's memory — should fail
    let result = room
        .handoff(agent_c, agent_b, mem_id, "Illegal handoff")
        .await
        .unwrap();
    assert!(!result, "Unauthorized handoff must be rejected");
}

#[tokio::test]
async fn cert_handoff_003_handoff_preserves_provenance() {
    let (room, agents, _) = setup_room(2).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];

    let mem_id = room
        .store_private(agent_a, "Work product".into(), zero_vector(), None)
        .await
        .unwrap();

    room.handoff(agent_a, agent_b, mem_id, "Review")
        .await
        .unwrap();

    // Query from B's perspective — verify handoff metadata
    let results = room
        .query_scoped(agent_b, HybridQuery::text("Work product", 5))
        .await
        .unwrap();

    let handoff_node = results
        .iter()
        .find(|r| {
            r.node.metadata.get("handoff_to").and_then(|v| v.as_str())
                == Some(&agent_b.id.to_string())
        })
        .expect("Handoff node must exist");

    let agent_a_id_str = agent_a.id.to_string();
    assert_eq!(
        handoff_node
            .node
            .metadata
            .get("handoff_from")
            .and_then(|v| v.as_str()),
        Some(agent_a_id_str.as_str()),
        "Handoff must preserve source agent"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-RESTRICTED: Restricted Visibility
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_restricted_001_only_allowed_agents_can_see() {
    let (room, agents, _) = setup_room(4).await;
    let owner = &agents[0];
    let allowed = &agents[1];
    let unauthorized1 = &agents[2];
    let unauthorized2 = &agents[3];

    room.store_restricted(
        owner,
        "Restricted content".into(),
        zero_vector(),
        vec![allowed.id],
        None,
    )
    .await
    .unwrap();

    let query = HybridQuery::text("Restricted content", 10).with_recency_boost(0.0);

    // Owner and allowed agent can see it
    assert!(room
        .query_scoped(owner, query.clone())
        .await
        .unwrap()
        .iter()
        .any(|r| r.node.content.as_deref() == Some("Restricted content")));
    assert!(room
        .query_scoped(allowed, query.clone())
        .await
        .unwrap()
        .iter()
        .any(|r| r.node.content.as_deref() == Some("Restricted content")));

    // Unauthorized agents cannot
    assert!(!room
        .query_scoped(unauthorized1, query.clone())
        .await
        .unwrap()
        .iter()
        .any(|r| r.node.content.as_deref() == Some("Restricted content")));
    assert!(!room
        .query_scoped(unauthorized2, query)
        .await
        .unwrap()
        .iter()
        .any(|r| r.node.content.as_deref() == Some("Restricted content")));
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-PROV: Provenance Audit Trail
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_prov_001_shared_memory_has_provenance() {
    let (room, agents, _) = setup_room(1).await;
    let agent = &agents[0];

    let id = room
        .store_shared(agent, "Decision: use Rust".into(), zero_vector(), None)
        .await
        .unwrap();

    let node = room.store().get(&id).await.unwrap().unwrap();

    let prov: serde_json::Value = node.provenance;
    assert_eq!(prov["agent_name"], agent.name);
    assert_eq!(prov["visibility"], "shared");
}

#[tokio::test]
async fn cert_prov_002_private_memory_has_provenance() {
    let (room, agents, _) = setup_room(1).await;
    let agent = &agents[0];

    let id = room
        .store_private(agent, "Private thought".into(), zero_vector(), None)
        .await
        .unwrap();

    let node = room.store().get(&id).await.unwrap().unwrap();

    let prov: serde_json::Value = node.provenance;
    assert_eq!(prov["agent_name"], agent.name);
    assert_eq!(prov["visibility"], "private");
}

#[tokio::test]
async fn cert_prov_003_handoff_creates_audit_trail() {
    let (room, agents, _) = setup_room(2).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];

    let mem_id = room
        .store_private(agent_a, "Audit test".into(), zero_vector(), None)
        .await
        .unwrap();

    room.handoff(agent_a, agent_b, mem_id, "For audit")
        .await
        .unwrap();

    let all_nodes = room.store().list_all().await.unwrap();
    let handoff_nodes: Vec<&FractalNode> = all_nodes
        .iter()
        .filter(|n| n.metadata.get("handoff_from").is_some())
        .collect();

    assert!(!handoff_nodes.is_empty(), "Handoff must leave audit trail");
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-ORCH: Orchestration Workflow
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_orch_001_full_simple_workflow() {
    // Simulate: Orchestrator → Worker → Reviewer pipeline
    let (room, agents, _) = setup_room(3).await;
    let orchestrator = &agents[0]; // AgentRole::User
    let worker = &agents[1]; // AgentRole::Orchestrator
    let reviewer = &agents[2]; // AgentRole::Worker

    // 1. Orchestrator stores task in shared layer
    let task_id = room
        .store_shared(
            orchestrator,
            "Task: Build feature X".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    // 2. Worker stores progress privately
    let progress_id = room
        .store_private(worker, "Progress: 50% done".into(), zero_vector(), None)
        .await
        .unwrap();

    // 3. Worker hands off result to Reviewer
    let result_id = room
        .store_private(
            worker,
            "Result: Feature X complete".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    room.handoff(worker, reviewer, result_id, "Please review")
        .await
        .unwrap();

    // 4. Reviewer stores feedback in shared layer
    let feedback_id = room
        .store_shared(
            reviewer,
            "Review: LGTM, approved".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    // Verify:
    // - Orchestrator sees: task (shared), feedback (shared), but NOT worker's private progress
    let orch_view = room.list_visible(orchestrator, 50).await.unwrap();
    let orch_ids: Vec<Uuid> = orch_view.iter().map(|n| n.id).collect();
    assert!(orch_ids.contains(&task_id), "Orchestrator must see task");
    assert!(
        orch_ids.contains(&feedback_id),
        "Orchestrator must see feedback"
    );
    assert!(
        !orch_ids.contains(&progress_id),
        "Orchestrator must NOT see worker's private progress"
    );

    // - Worker sees: task, their own progress, feedback — but NOT other private data
    let worker_view = room.list_visible(worker, 50).await.unwrap();
    let worker_ids: Vec<Uuid> = worker_view.iter().map(|n| n.id).collect();
    assert!(worker_ids.contains(&task_id));
    assert!(worker_ids.contains(&progress_id));
    assert!(worker_ids.contains(&feedback_id));

    // - Reviewer sees: task, feedback (own), handoff result — but NOT worker's private progress
    let reviewer_view = room.list_visible(reviewer, 50).await.unwrap();
    let reviewer_ids: Vec<Uuid> = reviewer_view.iter().map(|n| n.id).collect();
    assert!(reviewer_ids.contains(&task_id));
    assert!(reviewer_ids.contains(&feedback_id));
    assert!(
        !reviewer_ids.contains(&progress_id),
        "Reviewer must NOT see worker's private progress"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-GOV: Governance Integration
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_gov_001_agent_visibility_respects_sensitivity() {
    // High-sensitivity private data must not leak even within the same agent type
    let (room, agents, _) = setup_room(2).await;
    let agent_a = &agents[0];
    let agent_b = &agents[1];

    // Store with explicit agent metadata
    let mut metadata = HashMap::new();
    metadata.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_a.id.to_string()),
    );
    metadata.insert(
        "visibility".to_string(),
        serde_json::Value::String("private".to_string()),
    );

    let node = FractalNode::new_typed(
        Some("Highly sensitive data".into()),
        None,
        zero_vector(),
        metadata,
        MemoryType::Semantic,
        knowwhere_server::memory::types::MemorySource::Manual,
    );

    let id = room.store().insert(node).await.unwrap();

    let query = HybridQuery::text("sensitive data", 10).with_recency_boost(0.0);

    // Agent A (owner) sees it
    assert!(room
        .query_scoped(agent_a, query.clone())
        .await
        .unwrap()
        .iter()
        .any(|r| r.id == id));

    // Agent B does NOT
    assert!(!room
        .query_scoped(agent_b, query)
        .await
        .unwrap()
        .iter()
        .any(|r| r.id == id));
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-STRESS: Multi-Agent Concurrent Access
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_stress_001_concurrent_stores_maintain_isolation() {
    let (room, agents, _) = setup_room(5).await;

    // All agents store private data concurrently
    let mut handles = Vec::new();
    for agent in agents.iter().cloned() {
        let room = room.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..5 {
                room.store_private(
                    &agent,
                    format!("{}-item-{}", agent.name, i),
                    zero_vector(),
                    None,
                )
                .await
                .unwrap();
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Verify each agent sees exactly 5 private memories
    for agent in &agents {
        let results = room
            .query_private(agent, HybridQuery::text("item", 50))
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            5,
            "Agent {} should have exactly 5 private items",
            agent.name
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// CERT-E2E: Full Pipeline
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cert_e2e_001_full_orchestration_pipeline() {
    // Full multi-agent pipeline simulating GH-600 certification workflow
    let (room, agents, _) = setup_room(5).await;
    let user = &agents[0]; // AgentRole::User
    let orchestrator = &agents[1]; // AgentRole::Orchestrator
    let worker1 = &agents[2]; // AgentRole::Worker
    let worker2 = &agents[3]; // AgentRole::Reviewer
    let reviewer = &agents[4]; // AgentRole::Observer

    // Phase 1: User defines goal (shared)
    let goal_id = room
        .store_shared(
            user,
            "GOAL: Implement multi-agent state management".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    // Phase 2: Orchestrator decomposes (shared)
    let plan_id = room
        .store_shared(
            orchestrator,
            "PLAN: 3 subtasks for state management".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    // Phase 3: Workers execute privately, then handoff results
    let w1_private = room
        .store_private(
            worker1,
            "Worker1: implementing agent registry".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();
    let w1_result = room
        .store_private(
            worker1,
            "Worker1: DONE — agent registry code".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();
    room.handoff(worker1, orchestrator, w1_result, "Task complete")
        .await
        .unwrap();

    let w2_private = room
        .store_private(
            worker2,
            "Worker2: implementing control room".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();
    let w2_result = room
        .store_private(
            worker2,
            "Worker2: DONE — control room code".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();
    room.handoff(worker2, orchestrator, w2_result, "Task complete")
        .await
        .unwrap();

    // Phase 4: Orchestrator integrates and hands off to reviewer
    let integration_id = room
        .store_shared(
            orchestrator,
            "INTEGRATION: All components assembled".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();
    room.handoff(
        orchestrator,
        reviewer,
        integration_id,
        "Ready for certification review",
    )
    .await
    .unwrap();

    // Phase 5: Reviewer certifies (shared)
    let cert_id = room
        .store_shared(
            reviewer,
            "CERTIFICATION: PASSED — GH-600 requirements met".into(),
            zero_vector(),
            None,
        )
        .await
        .unwrap();

    // Final verification: stats
    let stats = room.stats(user).await.unwrap();
    assert!(
        stats.shared >= 3,
        "Should have goal + plan + integration + certification"
    );
    assert_eq!(stats.private_own, 0, "User had no private data");
    assert!(
        stats.private_other >= 2,
        "There are other agents' private memories"
    );

    // Verify no leakage: workers' intermediate private data is invisible to reviewer
    let reviewer_view = room.list_visible(reviewer, 50).await.unwrap();
    let reviewer_contents: Vec<&str> = reviewer_view
        .iter()
        .filter_map(|n| n.content.as_deref())
        .collect();
    assert!(
        !reviewer_contents.contains(&"Worker1: implementing agent registry"),
        "LEAK: Reviewer can see Worker1's intermediate private data"
    );
    assert!(
        !reviewer_contents.contains(&"Worker2: implementing control room"),
        "LEAK: Reviewer can see Worker2's intermediate private data"
    );

    // Verify certification exists and is shared
    assert!(
        reviewer_view.iter().any(|n| n.id == cert_id),
        "Reviewer should see the certification result"
    );
}

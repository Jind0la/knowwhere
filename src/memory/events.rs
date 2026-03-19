//! Event Sourcing — Layer 0: Immutable Event Log
//!
//! The event log is the source of truth. All state changes are recorded as
//! immutable events. This enables replay, audit, and reconstruction.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Layer 0: Immutable Event Log"
//!
//! # Design Principles
//!
//! - **Append-only**: Events are never updated or deleted.
//! - **Immutable payload**: Once written, the payload cannot be changed.
//! - **Typed events**: Each event has a type and structured payload.
//! - **Replay capability**: State can be rebuilt by replaying events.
//!
//! # Event Types
//!
//! | Event | When | Payload |
//! |-------|------|---------|
//! | `session_stored` | User/AI message stored | memory_id, memory_type, source |
//! | `external_stored` | External pointer stored | memory_id, pointer, source |
//! | `memory_accessed` | Memory retrieved | memory_id, query_id |
//! | `memory_updated` | Memory content changed | memory_id, field, old, new |
//! | `memory_superseded` | Memory replaced | memory_id, superseded_by_id |
//! | `edge_created` | Knowledge edge added | from_id, to_id, edge_type |
//! | `edge_deleted` | Knowledge edge removed | edge_id |
//! | `consolidation_run` | Dream Mode consolidation | run_id, stats |
//! | `audit_run` | Dream Mode audit | run_id, findings |
//! | `memory_deleted` | Memory soft-deleted | memory_id |

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A typed event in the immutable event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub event_type: EventType,
    pub payload: serde_json::Value, // immutable once written
    pub created_at: DateTime<Utc>,
}

impl Event {
    /// Create a new event.
    pub fn new(event_type: EventType, payload: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type,
            payload,
            created_at: Utc::now(),
        }
    }

    /// Serialize to JSON bytes (for storage).
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("event must be serializable")
    }

    /// Deserialize from JSON bytes (for replay).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

// -----------------------------------------------------------------------------
// Event Types
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventType {
    // Memory lifecycle
    SessionStored,
    ExternalStored,
    MemoryAccessed,
    MemoryUpdated {
        field: String,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
    },
    MemorySuperseded {
        superseded_by_id: Uuid,
    },
    MemoryDeleted,

    // Knowledge graph
    EdgeCreated {
        to_node_id: Uuid,
        edge_type: String,
    },
    EdgeDeleted,

    // Dream Mode
    ConsolidationRun {
        run_id: Uuid,
        memories_processed: usize,
        new_memories_created: usize,
        edges_created: usize,
    },
    AuditRun {
        run_id: Uuid,
        issues_found: usize,
    },

    // Governance
    SensitivityChanged {
        old: String,
        new: String,
    },
    ConflictDetected {
        memory_id: Uuid,
        contradicts_id: Uuid,
    },
    ConflictResolved {
        resolution: String,
    },
}

impl EventType {
    /// Human-readable name for debugging/logging.
    pub fn name(&self) -> &'static str {
        match self {
            EventType::SessionStored => "session_stored",
            EventType::ExternalStored => "external_stored",
            EventType::MemoryAccessed => "memory_accessed",
            EventType::MemoryUpdated { .. } => "memory_updated",
            EventType::MemorySuperseded { .. } => "memory_superseded",
            EventType::MemoryDeleted => "memory_deleted",
            EventType::EdgeCreated { .. } => "edge_created",
            EventType::EdgeDeleted => "edge_deleted",
            EventType::ConsolidationRun { .. } => "consolidation_run",
            EventType::AuditRun { .. } => "audit_run",
            EventType::SensitivityChanged { .. } => "sensitivity_changed",
            EventType::ConflictDetected { .. } => "conflict_detected",
            EventType::ConflictResolved { .. } => "conflict_resolved",
        }
    }

    /// Whether this event can trigger a re-embedding.
    pub fn requires_reembedding(&self) -> bool {
        matches!(
            self,
            EventType::MemoryUpdated { .. } | EventType::MemorySuperseded { .. }
        )
    }
}

// -----------------------------------------------------------------------------
// Event Builder Helpers
// -----------------------------------------------------------------------------

/// Convenience builders for common events.
pub mod builders {
    use super::*;

    pub fn session_stored(memory_id: Uuid, memory_type: &str, source: &str) -> Event {
        Event::new(
            EventType::SessionStored,
            serde_json::json!({
                "memory_id": memory_id.to_string(),
                "memory_type": memory_type,
                "source": source,
            }),
        )
    }

    pub fn external_stored(memory_id: Uuid, pointer: &str, source: &str) -> Event {
        Event::new(
            EventType::ExternalStored,
            serde_json::json!({
                "memory_id": memory_id.to_string(),
                "pointer": pointer,
                "source": source,
            }),
        )
    }

    pub fn memory_accessed(memory_id: Uuid, query_id: Uuid) -> Event {
        Event::new(
            EventType::MemoryAccessed,
            serde_json::json!({
                "memory_id": memory_id.to_string(),
                "query_id": query_id.to_string(),
            }),
        )
    }

    pub fn memory_updated(
        memory_id: Uuid,
        field: &str,
        old_value: Option<serde_json::Value>,
        new_value: Option<serde_json::Value>,
    ) -> Event {
        Event::new(
            EventType::MemoryUpdated {
                field: field.to_string(),
                old_value,
                new_value,
            },
            serde_json::json!({
                "memory_id": memory_id.to_string(),
                "field": field,
            }),
        )
    }

    pub fn memory_superseded(memory_id: Uuid, superseded_by_id: Uuid) -> Event {
        Event::new(
            EventType::MemorySuperseded {
                superseded_by_id,
            },
            serde_json::json!({
                "memory_id": memory_id.to_string(),
                "superseded_by": superseded_by_id.to_string(),
            }),
        )
    }

    pub fn edge_created(from_id: Uuid, to_id: Uuid, edge_type: &str) -> Event {
        Event::new(
            EventType::EdgeCreated {
                to_node_id: to_id,
                edge_type: edge_type.to_string(),
            },
            serde_json::json!({
                "from_node_id": from_id.to_string(),
                "to_node_id": to_id.to_string(),
                "edge_type": edge_type,
            }),
        )
    }

    pub fn consolidation_run(
        run_id: Uuid,
        memories_processed: usize,
        new_memories_created: usize,
        edges_created: usize,
    ) -> Event {
        Event::new(
            EventType::ConsolidationRun {
                run_id,
                memories_processed,
                new_memories_created,
                edges_created,
            },
            serde_json::json!({
                "run_id": run_id.to_string(),
                "memories_processed": memories_processed,
                "new_memories_created": new_memories_created,
                "edges_created": edges_created,
            }),
        )
    }

    pub fn audit_run(run_id: Uuid, issues_found: usize) -> Event {
        Event::new(
            EventType::AuditRun {
                run_id,
                issues_found,
            },
            serde_json::json!({
                "run_id": run_id.to_string(),
                "issues_found": issues_found,
            }),
        )
    }
}

// -----------------------------------------------------------------------------
// Event Store Trait (for dependency injection)
// -----------------------------------------------------------------------------

use async_trait::async_trait;

/// Trait for event storage backends.
/// Default implementation: PostgreSQL (see postgres_store.rs).
#[async_trait]
pub trait EventStore: Send + Sync {
    /// Append a single event.
    async fn append(&self, event: &Event) -> anyhow::Result<()>;

    /// Append multiple events atomically.
    async fn append_batch(&self, events: &[Event]) -> anyhow::Result<()> {
        for event in events {
            self.append(event).await?;
        }
        Ok(())
    }

    /// Read events after a given cursor (for replay).
    async fn read_after(
        &self,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> anyhow::Result<Vec<Event>>;

    /// Read events by type.
    async fn read_by_type(
        &self,
        event_type: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<Event>>;

    /// Count total events.
    async fn count(&self) -> anyhow::Result<i64>;
}

// -----------------------------------------------------------------------------
// In-Memory Event Store
// -----------------------------------------------------------------------------

use std::sync::Arc;
use tokio::sync::RwLock;

/// Simple in-memory event store.
/// Suitable for development and single-instance deployments.
/// For production with multiple instances, use PostgresStore instead.
#[derive(Clone)]
pub struct InMemoryEventStore {
    events: Arc<RwLock<Vec<Event>>>,
}

impl InMemoryEventStore {
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a new InMemoryEventStore with some seed events.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::with_capacity(capacity))),
        }
    }
}

impl Default for InMemoryEventStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn append(&self, event: &Event) -> anyhow::Result<()> {
        self.events.write().await.push(event.clone());
        Ok(())
    }

    async fn read_after(
        &self,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> anyhow::Result<Vec<Event>> {
        let events = self.events.read().await;
        let start = if let Some(after) = after_id {
            events.iter().position(|e| e.id == after).map(|p| p + 1).unwrap_or(0)
        } else {
            0
        };
        Ok(events[start..].iter().take(limit as usize).cloned().collect())
    }

    async fn read_by_type(&self, event_type: &str, limit: i64) -> anyhow::Result<Vec<Event>> {
        let events = self.events.read().await;
        Ok(events
            .iter()
            .filter(|e| e.event_type.name() == event_type)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn count(&self) -> anyhow::Result<i64> {
        Ok(self.events.read().await.len() as i64)
    }
}

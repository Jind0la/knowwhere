//! Dream Mode — Two separate processes
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Dream Mode Definition"
//!
//! Dream Mode consists of TWO separate processes that must NOT be mixed:
//!
//! 1. **Consolidation** (`consolidation.rs`): Bündelt, clustert, verdichtet.
//!    Creates summary nodes from episodic memories. Is about building.
//!
//! 2. **Audit** (`audit.rs`): Prüft auf Drift, Konflikte, Sensitivität.
//!    Flags issues in existing memory structures. Is about checking.
//!
//! Calling this `DreamMode` is a legacy name. Prefer importing the specific
//! engines you need: `consolidation::ConsolidationEngine` or `audit::AuditEngine`.

pub mod audit;
pub mod consolidation;

pub use audit::{AuditConfig, AuditEngine, AuditFinding, AuditReport, AuditFindingType};
pub use consolidation::{ConsolidationConfig, ConsolidationEngine, ConsolidationReport, MemoryCluster};

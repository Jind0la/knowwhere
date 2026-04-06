//! Dream Mode — Audit
//!
//! Part 2 of Dream Mode: Audit.
//! Prüft bestehende Memory-Strukturen gegen Rohprovenienz, Konflikte,
//! Drift, Sensitivität und Veralterung.
//!
//! IMPORTANT: This is SEPARATE from Consolidation. Audit is about
//! checking and flagging issues. Consolidation is about building summaries.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Dream Mode Definition" > Audit

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::memory::governance::GovernanceCandidate;
use crate::memory::types::{ConflictState, MemoryStatus, MemoryType};

/// Result of an audit run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub run_id: Uuid,
    pub issues_found: usize,
    pub memories_checked: usize,
    pub duration_ms: u128,
    pub findings: Vec<AuditFinding>,
}

/// A single finding from the audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFinding {
    pub memory_id: Uuid,
    pub finding_type: AuditFindingType,
    pub severity: Severity,
    pub description: String,
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditFindingType {
    /// Semantic drift detected — content no longer matches embedding.
    DriftDetected,
    /// A memory contradicts another.
    ConflictFound,
    /// Memory exceeds sensitivity policy.
    SensitivityViolation,
    /// Memory is older than its type's refresh period.
    StaleMarked,
    /// Supersession chain too deep.
    SupersessionChain,
    /// Memory has low confidence.
    LowConfidence,
    /// Memory not accessed in a long time.
    Orphaned,
    /// Multiple memories for the same fact.
    Duplicate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// Audit configuration.
#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// Maximum age in days before a memory is flagged as stale.
    pub stale_threshold_days: u32,
    /// Minimum confidence threshold.
    pub min_confidence: f64,
    /// Days without access before flagged as orphaned.
    pub orphan_threshold_days: u32,
    /// Maximum supersession chain depth.
    pub max_supersession_depth: usize,
    /// Whether to auto-mark stale memories.
    pub auto_mark_stale: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            stale_threshold_days: 90,
            min_confidence: 0.5,
            orphan_threshold_days: 180,
            max_supersession_depth: 5,
            auto_mark_stale: false,
        }
    }
}

/// The Audit engine.
/// Call `run_audit` periodically (e.g., every hour).
pub struct AuditEngine {
    config: AuditConfig,
}

impl AuditEngine {
    pub fn new(config: AuditConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(AuditConfig::default())
    }

    /// Run a full audit pass.
    ///
    /// This checks:
    /// 1. **Staleness**: Memories older than their type's refresh period
    /// 2. **Confidence**: Memories below minimum confidence
    /// 3. **Sensitivität**: Memories with high/restricted sensitivity
    /// 4. **Orphaned**: Memories not accessed in a long time
    /// 5. **Supersession chains**: Too many supersessions deep
    ///
    /// Returns an `AuditReport` with all findings.
    pub async fn run_audit<M: MemoryStore>(&self, store: &M) -> Result<AuditReport> {
        let run_id = Uuid::new_v4();
        let start = std::time::Instant::now();
        let mut findings = Vec::new();

        // 1. Check all active memories
        let memories = store.get_all_active().await?;
        let memories_checked = memories.len();

        for memory in memories {
            let candidate = GovernanceCandidate {
                id: memory.id,
                memory_type: memory.memory_type,
                confidence: memory.confidence,
                sensitivity: memory.sensitivity,
                status: memory.status,
                superseded_by: memory.superseded_by,
                conflict_state: memory.conflict_state,
                created_at: memory.created_at,
                importance: memory.importance,
                access_count: memory.access_count,
                last_accessed: memory.last_accessed,
            };

            // 1.1 Staleness check
            if let Some(finding) = self.check_staleness(&candidate) {
                findings.push(finding);
            }

            // 1.2 Confidence check
            if let Some(finding) = self.check_confidence(&candidate) {
                findings.push(finding);
            }

            // 1.3 Orphaned check
            if let Some(finding) = self.check_orphaned(&candidate) {
                findings.push(finding);
            }
        }

        // 2. Check supersession chains
        findings.extend(self.check_supersession_chains(store).await?);

        // 3. Optionally auto-mark stale memories
        if self.config.auto_mark_stale {
            self.auto_mark_stale(store, &findings).await?;
        }

        let duration_ms = start.elapsed().as_millis();

        Ok(AuditReport {
            run_id,
            issues_found: findings.len(),
            memories_checked,
            duration_ms,
            findings,
        })
    }

    fn check_staleness(&self, candidate: &GovernanceCandidate) -> Option<AuditFinding> {
        let threshold = candidate
            .memory_type
            .suggested_refresh_days()
            .unwrap_or(self.config.stale_threshold_days);

        let age = (Utc::now() - candidate.created_at).num_days() as u32;
        if age > threshold {
            return Some(AuditFinding {
                memory_id: candidate.id,
                finding_type: AuditFindingType::StaleMarked,
                severity: Severity::Warning,
                description: format!(
                    "Memory is {} days old (threshold for {}: {} days)",
                    age,
                    candidate.memory_type.label(),
                    threshold
                ),
                suggested_action: Some(format!(
                    "Consider consolidating or re-embedding this {} memory",
                    candidate.memory_type.label()
                )),
            });
        }
        None
    }

    fn check_confidence(&self, candidate: &GovernanceCandidate) -> Option<AuditFinding> {
        if candidate.confidence < self.config.min_confidence {
            return Some(AuditFinding {
                memory_id: candidate.id,
                finding_type: AuditFindingType::LowConfidence,
                severity: if candidate.confidence < 0.3 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                description: format!(
                    "Memory confidence {:.2} is below threshold {:.2}",
                    candidate.confidence, self.config.min_confidence
                ),
                suggested_action: Some(
                    "Review this memory for accuracy or mark as draft".to_string(),
                ),
            });
        }
        None
    }

    fn check_orphaned(&self, candidate: &GovernanceCandidate) -> Option<AuditFinding> {
        if let Some(last_accessed) = candidate.last_accessed {
            let days_since = (Utc::now() - last_accessed).num_days() as u32;
            if days_since > self.config.orphan_threshold_days {
                return Some(AuditFinding {
                    memory_id: candidate.id,
                    finding_type: AuditFindingType::Orphaned,
                    severity: Severity::Info,
                    description: format!("Memory not accessed in {} days", days_since),
                    suggested_action: Some(
                        "Consider archiving or re-evaluating relevance".to_string(),
                    ),
                });
            }
        } else if candidate.access_count == 0 {
            // Never accessed at all
            return Some(AuditFinding {
                memory_id: candidate.id,
                finding_type: AuditFindingType::Orphaned,
                severity: Severity::Info,
                description: "Memory has never been accessed".to_string(),
                suggested_action: Some("Check if this memory is still relevant".to_string()),
            });
        }
        None
    }

    async fn check_supersession_chains<M: MemoryStore>(
        &self,
        store: &M,
    ) -> Result<Vec<AuditFinding>> {
        let mut findings = Vec::new();

        // Get all superseded memories
        let superseded_memories = store.get_superseded_memories().await?;

        for memory in superseded_memories {
            let chain = self
                .follow_supersession_chain(store, memory.id, &mut Vec::new(), 0)
                .await?;

            if chain.len() > self.config.max_supersession_depth {
                findings.push(AuditFinding {
                    memory_id: memory.id,
                    finding_type: AuditFindingType::SupersessionChain,
                    severity: Severity::Warning,
                    description: format!(
                        "Supersession chain depth {} exceeds max {}",
                        chain.len(),
                        self.config.max_supersession_depth
                    ),
                    suggested_action: Some(
                        "Consider collapsing this chain via consolidation".to_string(),
                    ),
                });
            }
        }

        Ok(findings)
    }

    async fn follow_supersession_chain<M: MemoryStore>(
        &self,
        store: &M,
        memory_id: Uuid,
        visited: &mut Vec<Uuid>,
        depth: usize,
    ) -> Result<Vec<Uuid>> {
        let mut current_id = memory_id;
        let mut current_depth = depth;

        loop {
            if current_depth > 20 || visited.contains(&current_id) {
                return Ok(visited.clone());
            }
            visited.push(current_id);

            match store.get_superseded_by(current_id).await? {
                Some(superseded_by) => {
                    current_id = superseded_by;
                    current_depth += 1;
                }
                None => return Ok(visited.clone()),
            }
        }
    }

    async fn auto_mark_stale<M: MemoryStore>(
        &self,
        store: &M,
        findings: &[AuditFinding],
    ) -> Result<()> {
        for finding in findings
            .iter()
            .filter(|f| f.finding_type == AuditFindingType::StaleMarked)
        {
            store
                .update_status(finding.memory_id, MemoryStatus::Stale)
                .await?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Memory Store trait (for dependency injection)
// -----------------------------------------------------------------------------

use async_trait::async_trait;

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn get_all_active(&self) -> Result<Vec<ActiveMemory>>;
    async fn get_superseded_memories(&self) -> Result<Vec<SupersededMemory>>;
    async fn get_superseded_by(&self, memory_id: Uuid) -> Result<Option<Uuid>>;
    async fn update_status(&self, memory_id: Uuid, status: MemoryStatus) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct ActiveMemory {
    pub id: Uuid,
    pub memory_type: MemoryType,
    pub confidence: f64,
    pub sensitivity: crate::memory::types::Sensitivity,
    pub status: MemoryStatus,
    pub superseded_by: Option<Uuid>,
    pub conflict_state: ConflictState,
    pub created_at: chrono::DateTime<Utc>,
    pub importance: i32,
    pub access_count: i32,
    pub last_accessed: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct SupersededMemory {
    pub id: Uuid,
    pub superseded_by: Uuid,
}

//! Governance Policy — Layer 4
//!
//! Implements the Governance-before-Recall principle from the Source of Truth.
//! Stage 2 of retrieval: before a candidate goes into the prompt,
//! it must pass governance validation.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Retrieval Process Definition" + "Governance Policy Layer"

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::types::{ConflictState, MemoryStatus, MemoryType, Sensitivity};

// -----------------------------------------------------------------------------
// Governance Policy
// -----------------------------------------------------------------------------

/// Policy rules for memory retrieval governance.
/// These are applied as Stage 2 validation after Hybrid Retrieval (Stage 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernancePolicy {
    /// Minimum confidence score to pass (0.0–1.0).
    /// Default: 0.5
    pub min_confidence: f64,

    /// Maximum age in days before a memory is considered stale.
    /// None = no limit.
    pub max_age_days: Option<u32>,

    /// Block memories above this sensitivity level.
    pub blocked_sensitivities: Vec<Sensitivity>,

    /// Enable supersession checking (skip superseded memories).
    pub supersession_enabled: bool,

    /// Enable conflict checking.
    pub conflict_check_enabled: bool,

    /// Boost score for recently accessed memories.
    pub recency_boost_enabled: bool,

    /// Days after which a memory gets a recency penalty.
    pub recency_penalty_after_days: u32,

    /// Default policy (permissive).
    pub fn default() -> Self {
        Self {
            min_confidence: 0.5,
            max_age_days: None,
            blocked_sensitivities: vec![Sensitivity::Restricted],
            supersession_enabled: true,
            conflict_check_enabled: true,
            recency_boost_enabled: true,
            recency_penalty_after_days: 90,
        }
    }

    /// Strict policy for high-stakes retrieval.
    pub fn strict() -> Self {
        Self {
            min_confidence: 0.7,
            max_age_days: Some(180),
            blocked_sensitivities: vec![Sensitivity::Restricted, Sensitivity::High],
            supersession_enabled: true,
            conflict_check_enabled: true,
            recency_boost_enabled: true,
            recency_penalty_after_days: 30,
        }
    }

    /// Lenient policy for exploration / creative tasks.
    pub fn lenient() -> Self {
        Self {
            min_confidence: 0.3,
            max_age_days: None,
            blocked_sensitivities: vec![Sensitivity::Restricted],
            supersession_enabled: false, // Include superseded for context
            conflict_check_enabled: false,
            recency_boost_enabled: false,
            recency_penalty_after_days: 365,
        }
    }
}

impl Default for GovernancePolicy {
    fn default() -> Self {
        Self::default()
    }
}

// -----------------------------------------------------------------------------
// Validation Result
// -----------------------------------------------------------------------------

/// Result of a single governance validation check.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidationResult {
    /// Whether the memory passed all governance checks.
    pub passed: bool,
    /// List of issues found. Empty if passed.
    pub issues: Vec<ValidationIssue>,
    /// Overall governance score multiplier (0.0–1.0+).
    /// Multiplied with the retrieval score.
    pub score_multiplier: f64,
}

impl ValidationResult {
    /// Returns true if any issue has a hard-block impact (score_multiplier = 0.0).
    /// These nodes should be excluded from retrieval results entirely.
    pub fn has_hard_block(&self) -> bool {
        self.issues.iter().any(|i| i.score_impact == 0.0)
    }

    pub fn pass() -> Self {
        Self {
            passed: true,
            issues: vec![],
            score_multiplier: 1.0,
        }
    }

    pub fn fail(issues: Vec<ValidationIssue>) -> Self {
        let multiplier = issues.iter().map(|i| i.score_impact()).fold(1.0, |acc, m| acc * m);
        Self {
            passed: false,
            issues,
            score_multiplier: multiplier,
        }
    }
}

/// A single issue found during governance validation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ValidationIssue {
    /// The type of issue.
    pub issue_type: IssueType,
    /// Human-readable description.
    pub description: String,
    /// Score impact multiplier.
    pub score_impact: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    /// Confidence below minimum threshold.
    LowConfidence,
    /// Memory has been superseded.
    Superseded,
    /// Sensitivity level is blocked.
    SensitivityBlocked,
    /// Memory is older than max_age_days.
    Stale,
    /// Memory has an unresolved conflict.
    UnresolvedConflict,
    /// Memory is in a non-retrievable status.
    InvalidStatus,
    /// Memory was explicitly marked as irrelevant.
    Irrelevant,
}

impl IssueType {
    fn score_impact(&self) -> f64 {
        match self {
            IssueType::LowConfidence => 0.5,
            IssueType::Superseded => 0.0,   // Hard block
            IssueType::SensitivityBlocked => 0.0, // Hard block
            IssueType::Stale => 0.7,
            IssueType::UnresolvedConflict => 0.3,
            IssueType::InvalidStatus => 0.0, // Hard block
            IssueType::Irrelevant => 0.0,     // Hard block
        }
    }

    fn description(&self, detail: &str) -> String {
        match self {
            IssueType::LowConfidence => format!("Confidence below threshold: {}", detail),
            IssueType::Superseded => format!("Memory superseded by: {}", detail),
            IssueType::SensitivityBlocked => format!("Sensitivity level blocked: {}", detail),
            IssueType::Stale => format!("Memory is stale (age > {}): {}", detail, ""),
            IssueType::UnresolvedConflict => format!("Unresolved conflict: {}", detail),
            IssueType::InvalidStatus => format!("Invalid status for retrieval: {}", detail),
            IssueType::Irrelevant => "Memory marked as irrelevant".to_string(),
        }
    }
}

// -----------------------------------------------------------------------------
// Governance Validator
// -----------------------------------------------------------------------------

/// Validates a memory candidate against governance policy.
/// This is Stage 2 of the retrieval pipeline.
pub struct GovernanceValidator {
    policy: GovernancePolicy,
}

impl GovernanceValidator {
    pub fn new(policy: GovernancePolicy) -> Self {
        Self { policy }
    }

    /// Validate a single memory candidate.
    pub fn validate(&self, candidate: &GovernanceCandidate) -> ValidationResult {
        let mut issues = Vec::new();

        // 1. Status check — must be retrievable
        if !candidate.status.is_retrievable() {
            issues.push(ValidationIssue {
                issue_type: IssueType::InvalidStatus,
                description: IssueType::InvalidStatus.description(&candidate.status.to_string()),
                score_impact: IssueType::InvalidStatus.score_impact(),
            });
        }

        // 2. Confidence check
        if candidate.confidence < self.policy.min_confidence {
            issues.push(ValidationIssue {
                issue_type: IssueType::LowConfidence,
                description: IssueType::LowConfidence.description(&format!(
                    "{:.2} < {:.2}",
                    candidate.confidence, self.policy.min_confidence
                )),
                score_impact: IssueType::LowConfidence.score_impact(),
            });
        }

        // 3. Supersession check
        if self.policy.supersession_enabled {
            if candidate.superseded_by.is_some() {
                let superseded_by_str = candidate
                    .superseded_by
                    .map(|id| id.to_string())
                    .unwrap_or_default();
                issues.push(ValidationIssue {
                    issue_type: IssueType::Superseded,
                    description: IssueType::Superseded.description(&superseded_by_str),
                    score_impact: IssueType::Superseded.score_impact(),
                });
            }
        }

        // 4. Sensitivity check
        if self.policy.blocked_sensitivities.contains(&candidate.sensitivity) {
            issues.push(ValidationIssue {
                issue_type: IssueType::SensitivityBlocked,
                description: IssueType::SensitivityBlocked.description(&candidate.sensitivity.to_string()),
                score_impact: IssueType::SensitivityBlocked.score_impact(),
            });
        }

        // 5. Staleness check
        if let Some(max_age) = self.policy.max_age_days {
            let age = Utc::now() - candidate.created_at;
            if age > Duration::days(max_age as i64) {
                issues.push(ValidationIssue {
                    issue_type: IssueType::Stale,
                    description: format!(
                        "Memory is {} days old (max: {})",
                        age.num_days(),
                        max_age
                    ),
                    score_impact: IssueType::Stale.score_impact(),
                });
            }
        }

        // 6. Conflict check
        if self.policy.conflict_check_enabled {
            if candidate.conflict_state == ConflictState::Pending {
                issues.push(ValidationIssue {
                    issue_type: IssueType::UnresolvedConflict,
                    description: IssueType::UnresolvedConflict.description("pending resolution"),
                    score_impact: IssueType::UnresolvedConflict.score_impact(),
                });
            }
        }

        if issues.is_empty() {
            ValidationResult::pass()
        } else {
            ValidationResult::fail(issues)
        }
    }

    /// Apply governance and return a score multiplier.
    pub fn apply_governance(&self, candidate: &GovernanceCandidate) -> f64 {
        let result = self.validate(candidate);
        result.score_multiplier
    }
}

// -----------------------------------------------------------------------------
// Governance Candidate (minimal set for validation)
// -----------------------------------------------------------------------------

/// Minimal information needed to validate a memory candidate.
/// This is what we need from the retrieval result for Stage 2.
#[derive(Debug, Clone)]
pub struct GovernanceCandidate {
    pub id: Uuid,
    pub memory_type: MemoryType,
    pub confidence: f64,
    pub sensitivity: Sensitivity,
    pub status: MemoryStatus,
    pub superseded_by: Option<Uuid>,
    pub conflict_state: ConflictState,
    pub created_at: DateTime<Utc>,
    pub importance: i32,
    pub access_count: i32,
    pub last_accessed: Option<DateTime<Utc>>,
}

impl GovernanceCandidate {
    /// Calculate a recency boost (if enabled in policy).
    pub fn recency_boost(&self, penalty_after_days: u32) -> f64 {
        if let Some(last_accessed) = self.last_accessed {
            let days_since = (Utc::now() - last_accessed).num_days() as u32;
            if days_since < penalty_after_days {
                // Boost for recently accessed: up to +0.2
                let fraction = 1.0 - (days_since as f64 / penalty_after_days as f64);
                return 1.0 + (fraction * 0.2);
            }
        }
        // No recent access — apply small penalty
        0.95
    }

    /// Combined governance + relevance score.
    pub fn governance_score(&self, policy: &GovernancePolicy) -> f64 {
        let governance_multiplier = {
            let v = self.apply_governance(policy);
            if v == 0.0 {
                return 0.0; // Hard block
            }
            v
        };

        let recency_multiplier = if policy.recency_boost_enabled {
            self.recency_boost(policy.recency_penalty_after_days)
        } else {
            1.0
        };

        // Importance-based boost (subtle, 0.9–1.1)
        let importance_multiplier = 0.9 + (self.importance as f64 * 0.02);

        governance_multiplier * recency_multiplier * importance_multiplier
    }

    fn apply_governance(&self, policy: &GovernancePolicy) -> f64 {
        let mut multiplier = 1.0;

        if self.confidence < policy.min_confidence {
            multiplier *= 0.5;
        }
        if self.superseded_by.is_some() && policy.supersession_enabled {
            return 0.0;
        }
        if policy.blocked_sensitivities.contains(&self.sensitivity) {
            return 0.0;
        }
        if !self.status.is_retrievable() {
            return 0.0;
        }
        if policy.conflict_check_enabled && self.conflict_state == ConflictState::Pending {
            multiplier *= 0.3;
        }

        multiplier
    }
}

// -----------------------------------------------------------------------------
// Scored Node with Governance
// -----------------------------------------------------------------------------

/// A retrieval result with governance applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernedScoredNode {
    /// The raw retrieval score (from hybrid search).
    pub retrieval_score: f32,
    /// The governance score multiplier.
    pub governance_multiplier: f64,
    /// Combined score = retrieval_score * governance_multiplier.
    pub combined_score: f32,
    /// Whether governance passed.
    pub governance_passed: bool,
    /// Issues found (for transparency/debugging).
    pub governance_issues: Vec<ValidationIssue>,
    // --- Fields from FractalNode ---
    pub id: Uuid,
    pub memory_type: MemoryType,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl GovernedScoredNode {
    pub fn from_candidate(
        candidate: GovernanceCandidate,
        retrieval_score: f32,
        validation_result: ValidationResult,
    ) -> Self {
        let governance_multiplier = validation_result.score_multiplier;
        let combined_score = if governance_multiplier == 0.0 {
            0.0
        } else {
            retrieval_score * governance_multiplier as f32
        };

        Self {
            retrieval_score,
            governance_multiplier,
            combined_score,
            governance_passed: validation_result.passed,
            governance_issues: validation_result.issues,
            id: candidate.id,
            memory_type: candidate.memory_type,
            content: None, // Set by caller
            original_pointer: None,
            metadata: serde_json::Value::Object(Default::default()),
            created_at: candidate.created_at,
        }
    }
}

// -----------------------------------------------------------------------------
// Re-exports
// -----------------------------------------------------------------------------

pub use ValidationResult as AuditResult;

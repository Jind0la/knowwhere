//! Integration tests for the Governance-before-Recall engine.
//!
//! Tests the full validation pipeline: three candidates with different
//! governance profiles are validated against the default policy.
//!
//! Run with: cargo test --test governance_test

use chrono::{Days, Utc};
use uuid::Uuid;

use crate::memory::governance::{GovernanceCandidate, GovernancePolicy, GovernanceValidator};
use crate::memory::types::{ConflictState, MemoryStatus, MemoryType, Sensitivity};

/// Helper: build a GovernanceCandidate with configurable properties.
fn candidate(
    confidence: f64,
    superseded_by: Option<Uuid>,
    sensitivity: Sensitivity,
    status: MemoryStatus,
    memory_type: MemoryType,
    age_days: u64,
) -> GovernanceCandidate {
    GovernanceCandidate {
        id: Uuid::new_v4(),
        memory_type,
        confidence,
        sensitivity,
        status,
        superseded_by,
        conflict_state: ConflictState::None,
        created_at: Utc::now()
            .checked_sub_days(Days::new(age_days))
            .unwrap_or_else(Utc::now),
        importance: 5,
        access_count: 1,
        last_accessed: Some(Utc::now()),
    }
}

#[test]
fn test_governance_candidate_passing() {
    // A high-confidence, active, non-superseded episodic memory should pass.
    let cand = candidate(
        0.9,
        None,
        Sensitivity::Normal,
        MemoryStatus::Active,
        MemoryType::Episodic,
        1,
    );

    let policy = GovernancePolicy::default();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    assert!(
        result.passed,
        "High-confidence active memory should pass governance, but got: {:?}",
        result.issues
    );
    assert!(
        result.score_multiplier >= 1.0,
        "Passing candidate should get multiplier >= 1.0, got {}",
        result.score_multiplier
    );
    assert!(
        result.issues.is_empty(),
        "Passing candidate should have no issues, got: {:?}",
        result.issues
    );
}

#[test]
fn test_governance_low_confidence() {
    // Confidence below min_confidence (0.5) should fail.
    let cand = candidate(
        0.3,
        None,
        Sensitivity::Normal,
        MemoryStatus::Active,
        MemoryType::Semantic,
        1,
    );

    let policy = GovernancePolicy::default();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    assert!(
        !result.passed,
        "Low-confidence memory should fail governance"
    );
    assert!(
        result.score_multiplier < 1.0,
        "Low-confidence should reduce score multiplier"
    );
    assert!(
        result.issues.iter().any(|i| format!("{:?}", i.issue_type).contains("LowConfidence")),
        "Should have LowConfidence issue, got: {:?}",
        result.issues
    );
}

#[test]
fn test_governance_superseded_blocks() {
    // A superseded memory should be hard-blocked (score_multiplier = 0).
    let superseded_id = Uuid::new_v4();
    let cand = candidate(
        0.95,
        Some(superseded_id),
        Sensitivity::Normal,
        MemoryStatus::Active,
        MemoryType::Semantic,
        1,
    );

    let policy = GovernancePolicy::default();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    assert!(
        !result.passed,
        "Superseded memory should fail governance"
    );
    assert_eq!(
        result.score_multiplier, 0.0,
        "Superseded should get hard block (multiplier = 0.0)"
    );
    assert!(
        result.issues.iter().any(|i| format!("{:?}", i.issue_type).contains("Superseded")),
        "Should have Superseded issue, got: {:?}",
        result.issues
    );
}

#[test]
fn test_governance_restricted_blocks() {
    // A Restricted-sensitivity memory should be hard-blocked.
    let cand = candidate(
        0.95,
        None,
        Sensitivity::Restricted,
        MemoryStatus::Active,
        MemoryType::Procedural,
        1,
    );

    let policy = GovernancePolicy::default();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    assert!(
        !result.passed,
        "Restricted sensitivity should fail governance"
    );
    assert_eq!(
        result.score_multiplier, 0.0,
        "Restricted should get hard block"
    );
    assert!(
        result.issues.iter().any(|i| format!("{:?}", i.issue_type).contains("SensitivityBlocked")),
        "Should have SensitivityBlocked issue"
    );
}

#[test]
fn test_governance_stale_penalty() {
    // A memory older than max_age_days should be penalised.
    // Strict policy has max_age_days = 180, so 200-day-old memory should fail.
    let cand = candidate(
        0.95,
        None,
        Sensitivity::Normal,
        MemoryStatus::Active,
        MemoryType::Semantic,
        200,
    );

    let policy = GovernancePolicy::strict();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    // 200 days > 180 max_age → should fail or be penalised
    assert!(
        !result.passed || result.score_multiplier < 1.0,
        "Stale memory should be penalised or blocked: {:?}",
        result.issues
    );
}

#[test]
fn test_governance_conflict_pending() {
    // A memory with pending conflict should be penalised.
    let cand = GovernanceCandidate {
        id: Uuid::new_v4(),
        memory_type: MemoryType::Semantic,
        confidence: 0.9,
        sensitivity: Sensitivity::Normal,
        status: MemoryStatus::Active,
        superseded_by: None,
        conflict_state: ConflictState::Pending,
        created_at: Utc::now(),
        importance: 5,
        access_count: 1,
        last_accessed: Some(Utc::now()),
    };

    let policy = GovernancePolicy::default();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    // Pending conflict with conflict_check_enabled = true should penalise
    assert!(
        result.score_multiplier < 1.0,
        "Pending conflict should reduce score multiplier, got {}",
        result.score_multiplier
    );
}

#[test]
fn test_governance_apply_governance_score() {
    // governance_score should return penalised-but-nonzero for low confidence.
    let cand = candidate(
        0.3,
        None,
        Sensitivity::Normal,
        MemoryStatus::Active,
        MemoryType::Episodic,
        1,
    );

    let policy = GovernancePolicy::default();

    let score = cand.governance_score(&policy);

    // Low confidence → multiplier 0.5 (not hard-blocked)
    assert!(
        score > 0.0 && score < 1.0,
        "governance_score should be penalised but non-zero for low confidence: {score}"
    );
}

#[test]
fn test_governance_recency_boost() {
    // Recently accessed memory should get a recency boost.
    let cand = GovernanceCandidate {
        id: Uuid::new_v4(),
        memory_type: MemoryType::Episodic,
        confidence: 0.8,
        sensitivity: Sensitivity::Normal,
        status: MemoryStatus::Active,
        superseded_by: None,
        conflict_state: ConflictState::None,
        created_at: Utc::now(),
        importance: 5,
        access_count: 10,
        last_accessed: Some(Utc::now()),
    };

    let boost = cand.recency_boost(90);

    assert!(
        boost > 1.0,
        "Recently accessed memory should get recency boost > 1.0, got {boost}"
    );
}

#[test]
fn test_governance_strict_policy_blocks_high_sensitivity() {
    // Strict policy blocks both High and Restricted sensitivities.
    let cand = candidate(
        0.95,
        None,
        Sensitivity::High,
        MemoryStatus::Active,
        MemoryType::Procedural,
        1,
    );

    let policy = GovernancePolicy::strict();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    assert!(
        !result.passed,
        "Strict policy should block High sensitivity"
    );
    assert_eq!(result.score_multiplier, 0.0);
}

#[test]
fn test_governance_lenient_policy_keeps_superseded() {
    // Lenient policy has supersession_enabled = false, superseded memories included.
    let superseded_id = Uuid::new_v4();
    let cand = candidate(
        0.95,
        Some(superseded_id),
        Sensitivity::Normal,
        MemoryStatus::Active,
        MemoryType::Semantic,
        1,
    );

    let policy = GovernancePolicy::lenient();
    let validator = GovernanceValidator::new(policy);

    let result = validator.validate(&cand);

    // With supersession disabled, superseded should NOT be blocked
    assert!(
        result.passed || result.score_multiplier > 0.0,
        "Lenient policy should not block superseded: {:?}",
        result.issues
    );
}

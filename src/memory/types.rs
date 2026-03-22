//! Memory Types — Canonical 5-Type System
//!
//! Implements the typed memory objects from the Source of Truth document.
//! Each type has distinct consolidation logic and epistemological properties.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section: "Typed Memory Objects"

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The 5 canonical memory types following cognitive science principles.
///
/// Each type has different:
///
/// - **Halbwertszeit (half-life)**: How quickly the memory might become stale
/// - **Consolidation Logic**: How this type is processed in Dream Mode
/// - **Epistemological Status**: Whether it's a fact, preference, or claim
/// - **Governance Rules**: Which policies apply
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// A specific event or session fact.
    ///
    /// Examples:
    /// - "Am 14.03. entschied Nimar, den Rust-Kern zu priorisieren"
    /// - "In session #42 the user said they prefer German responses"
    ///
    /// Consolidation: **High temporal sensitivity**.
    /// These memories should be clustered by time and eventually summarized
    /// into semantic memories. They have the shortest relevance span.
    Episodic,

    /// A stabilized knowledge statement or fact.
    ///
    /// Examples:
    /// - "KnowWhere follows Pointer-First architecture"
    /// - "TypeScript is a superset of JavaScript"
    ///
    /// Consolidation: **Conflict and supersession capable**.
    /// These memories can be superseded by newer, more accurate versions.
    /// When a conflict is detected, the old one should be marked superseded.
    Semantic,

    /// A personal preference or choice.
    ///
    /// Examples:
    /// - "User prefers async/await over callbacks"
    /// - "The user likes detailed explanations, not just summaries"
    ///
    /// Consolidation: **Version-sensitive**.
    /// Preferences can change over time. Old preferences should be archived,
    /// not deleted, so the system can track preference evolution.
    Preference,

    /// A rule, workflow, or procedural knowledge.
    ///
    /// Examples:
    /// - "To build KnowWhere: cargo run"
    /// - "Before deploying, always run tests"
    ///
    /// Consolidation: **Governance-critical**.
    /// Procedural memories are high-stakes. Changes require explicit overrides.
    /// Deleting or superseding them can break working systems.
    Procedural,

    /// Meta-cognitive knowledge about the memory system itself.
    ///
    /// Examples:
    /// - "This statement is only a hypothesis"
    /// - "The confidence score for this memory is low"
    ///
    /// Consolidation: **Audit-critical**.
    /// Meta memories are used to track the system's own uncertainty
    /// and self-knowledge. They require special handling in audits.
    Meta,
}

impl MemoryType {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            MemoryType::Episodic => "Episodic",
            MemoryType::Semantic => "Semantic",
            MemoryType::Preference => "Preference",
            MemoryType::Procedural => "Procedural",
            MemoryType::Meta => "Meta",
        }
    }

    /// Description of what this type represents.
    pub fn description(&self) -> &'static str {
        match self {
            MemoryType::Episodic => "Specific events or session facts",
            MemoryType::Semantic => "Stabilized knowledge or facts",
            MemoryType::Preference => "Personal preferences or choices",
            MemoryType::Procedural => "Rules, workflows, or how-to knowledge",
            MemoryType::Meta => "Meta-cognitive knowledge about the system itself",
        }
    }

    /// The consolidation logic for this type (from Source of Truth).
    pub fn consolidation_logic(&self) -> &'static str {
        match self {
            MemoryType::Episodic => "high_temporal_sensitivity",
            MemoryType::Semantic => "conflict_and_supersession_capable",
            MemoryType::Preference => "version_sensitive",
            MemoryType::Procedural => "governance_critical",
            MemoryType::Meta => "audit_critical",
        }
    }

    /// Default importance for new memories of this type.
    pub fn default_importance(&self) -> i32 {
        match self {
            MemoryType::Episodic => 5,
            MemoryType::Semantic => 6,
            MemoryType::Preference => 7,
            MemoryType::Procedural => 8, // Procedural memories are high-stakes
            MemoryType::Meta => 4,
        }
    }

    /// Default confidence for new memories of this type.
    pub fn default_confidence(&self) -> f64 {
        match self {
            MemoryType::Episodic => 0.8,
            MemoryType::Semantic => 0.85,
            MemoryType::Preference => 0.75, // Preferences can be less certain
            MemoryType::Procedural => 0.9, // Procedural should be well-verified
            MemoryType::Meta => 0.5,       // Meta-knowledge starts with low confidence
        }
    }

    /// How many days until this memory type might become stale (suggestion only).
    pub fn suggested_refresh_days(&self) -> Option<u32> {
        match self {
            MemoryType::Episodic => Some(7),   // Events go stale after a week
            MemoryType::Semantic => Some(90),  // Facts are stable longer
            MemoryType::Preference => Some(30), // Preferences can change
            MemoryType::Procedural => Some(180), // Procedures are very stable
            MemoryType::Meta => Some(14),       // Meta-knowledge needs frequent audit
        }
    }

    /// Whether this type can have outgoing `evolves_into` edges.
    pub fn can_evolve(&self) -> bool {
        matches!(
            self,
            MemoryType::Semantic | MemoryType::Preference | MemoryType::Procedural
        )
    }

    /// Whether this type can have `contradicts` edges.
    pub fn can_contradict(&self) -> bool {
        matches!(self, MemoryType::Semantic | MemoryType::Meta)
    }

    /// Parse from string (for API input).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "episodic" => Some(MemoryType::Episodic),
            "semantic" => Some(MemoryType::Semantic),
            "preference" => Some(MemoryType::Preference),
            "procedural" => Some(MemoryType::Procedural),
            "meta" => Some(MemoryType::Meta),
            _ => None,
        }
    }

    /// All types as a list.
    pub fn all() -> [Self; 5] {
        [
            MemoryType::Episodic,
            MemoryType::Semantic,
            MemoryType::Preference,
            MemoryType::Procedural,
            MemoryType::Meta,
        ]
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Preference => "preference",
            MemoryType::Procedural => "procedural",
            MemoryType::Meta => "meta",
        })
    }
}

// -----------------------------------------------------------------------------
// Sensitivity Classification
// -----------------------------------------------------------------------------

/// Sensitivity level for governance policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    #[default]
    Normal,
    Low,
    High,
    Restricted,
}

impl Sensitivity {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "normal" => Some(Sensitivity::Normal),
            "low" => Some(Sensitivity::Low),
            "high" => Some(Sensitivity::High),
            "restricted" => Some(Sensitivity::Restricted),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Sensitivity::Normal => "Normal",
            Sensitivity::Low => "Low",
            Sensitivity::High => "High",
            Sensitivity::Restricted => "Restricted",
        }
    }
}

// -----------------------------------------------------------------------------
// Conflict State
// -----------------------------------------------------------------------------

/// Memory conflict state for governance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConflictState {
    #[default]
    None,
    Pending,
    Resolved,
}

impl ConflictState {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" => Some(ConflictState::None),
            "pending" => Some(ConflictState::Pending),
            "resolved" => Some(ConflictState::Resolved),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// Memory Status
// -----------------------------------------------------------------------------

/// Lifecycle status of a memory.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    #[default]
    Active,
    Draft,
    Archived,
    Deleted,
    Superseded,
    Stale,
}

impl MemoryStatus {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(MemoryStatus::Active),
            "draft" => Some(MemoryStatus::Draft),
            "archived" => Some(MemoryStatus::Archived),
            "deleted" => Some(MemoryStatus::Deleted),
            "superseded" => Some(MemoryStatus::Superseded),
            "stale" => Some(MemoryStatus::Stale),
            _ => None,
        }
    }

    /// Whether this memory should be included in retrieval results.
    pub fn is_retrievable(&self) -> bool {
        matches!(self, MemoryStatus::Active | MemoryStatus::Draft)
    }
}

// -----------------------------------------------------------------------------
// Source Type
// -----------------------------------------------------------------------------

/// Where a memory originated from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemorySource {
    #[default]
    Conversation,
    Document,
    Import,
    Manual,
    Consolidation,
}

impl MemorySource {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "conversation" => Some(MemorySource::Conversation),
            "document" => Some(MemorySource::Document),
            "import" => Some(MemorySource::Import),
            "manual" => Some(MemorySource::Manual),
            "consolidation" => Some(MemorySource::Consolidation),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// Context Tier (L0/L1/L2) — Tiered Context Loading
// -----------------------------------------------------------------------------

/// Context tier for tiered context loading.
///
/// Enables hierarchical context loading with 3 levels:
/// - **L0 (Summary)**: One-sentence summary, minimal tokens
/// - **L1 (Overview)**: Paragraph-level summary
/// - **L2 (Raw)**: Full original content (default for existing memories)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContextTier {
    /// L0: one-sentence summary (minimal tokens, ~20-50)
    Summary,
    /// L1: paragraph overview (~100-300 tokens)
    Overview,
    /// L2: full raw content (default for backward compatibility)
    #[default]
    Raw,
}

impl ContextTier {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            ContextTier::Summary => "summary",
            ContextTier::Overview => "overview",
            ContextTier::Raw => "raw",
        }
    }

    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "summary" => Some(ContextTier::Summary),
            "overview" => Some(ContextTier::Overview),
            "raw" => Some(ContextTier::Raw),
            _ => None,
        }
    }

    /// The tier below this one (for compaction chain: Raw → Overview → Summary).
    pub fn parent_tier(&self) -> Option<Self> {
        match self {
            ContextTier::Raw => Some(ContextTier::Overview),
            ContextTier::Overview => Some(ContextTier::Summary),
            ContextTier::Summary => None,
        }
    }

    /// Which SQL column holds the content for this tier.
    pub fn content_column(&self) -> &'static str {
        match self {
            ContextTier::Summary => "summary_content",
            ContextTier::Overview => "overview_content",
            ContextTier::Raw => "content",
        }
    }
}

impl std::fmt::Display for ContextTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

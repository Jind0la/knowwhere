//! Lightweight regex-based fact extraction for inline ingestion.
//!
//! Extracts structured facts from raw text at storage time — no LLM required.
//! Complements the ConsolidationScheduler's LLM-based claim extraction by
//! catching obvious patterns immediately (before async consolidation runs).
//!
//! # Integration Points
//!
//! 1. **store_session** — after storing a session node, extract facts inline
//! 2. **store_external** — after storing external content, extract facts inline
//! 3. **ConsolidationScheduler** — already extracts via LLM; this module's
//!    helper converts facts to high-weighted FractalNodes
//!
//! # Regex Rules with Examples
//!
//! | # | Rule       | Example Input                                 | Extracted Fact              | Conf |
//! |---|-----------|----------------------------------------------|-----------------------------|------|
//! | 1 | Preference | "I really like Rust for systems."            | "Rust for systems"          | 0.85 |
//! | 2 | Decision   | "I decided to use PostgreSQL instead."       | "use PostgreSQL instead"    | 0.90 |
//! | 3 | Change     | "I no longer use Docker."                    | "use Docker" (superseded)   | 0.80 |
//! | 4 | Fact       | "The API server runs on port 3737."          | "The API server runs on..." | 0.60 |
//! | 5 | Intent     | "I plan to add fact extraction this week."   | "add fact extraction this..."| 0.70 |
//! | 6 | Correction | "Actually, the port should be 3737."         | "the port should be 3737"   | 0.75 |
//!
//! Rules 1-3 have strong confidence (>0.80) — minimal false positives.
//! Rule 4 runs at 0.60 confidence — permissive but capped at 12 facts/doc.
//! Rules 5-6 capture temporal nuance (intents, belief changes).
//! German variants (e.g., "ich mag", "ich werde") are covered in rules 1, 2, 3, 5.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use uuid::Uuid;

use crate::memory::types::{ContextTier, MemorySource, MemoryType};
use crate::memory::FractalNode;

/// A lightweight fact extracted from text using regex rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedFact {
    /// The fact claim text.
    pub claim: String,
    /// Supporting context or reason (if found).
    pub reason: String,
    /// Which rule matched (for debugging).
    pub rule: String,
    /// Confidence score 0.0–1.0 based on match quality.
    pub confidence: f64,
    /// The exact text span that triggered the match.
    pub matched_span: String,
}

impl ExtractedFact {
    /// Convert to a FractalNode suitable for storage.
    ///
    /// Creates a Decision memory type node with high importance (9) and
    /// provenance metadata linking back to the source node.
    pub fn to_fractal_node(
        &self,
        source_node_id: Uuid,
        source_session_id: Option<&str>,
        embedding: Vec<f32>,
    ) -> FractalNode {
        let content = if self.reason.is_empty() {
            format!("claim: {}", self.claim)
        } else {
            format!("claim: {}  reason: {}", self.claim, self.reason)
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            "decision_what".to_string(),
            serde_json::Value::String(self.claim.clone()),
        );
        if !self.reason.is_empty() {
            metadata.insert(
                "decision_why".to_string(),
                serde_json::Value::String(self.reason.clone()),
            );
        }
        metadata.insert(
            "derived_from".to_string(),
            serde_json::Value::String("inline_fact_extraction".to_string()),
        );
        metadata.insert(
            "extraction_rule".to_string(),
            serde_json::Value::String(self.rule.clone()),
        );
        metadata.insert(
            "source_node_ids".to_string(),
            serde_json::Value::Array(vec![serde_json::Value::String(
                source_node_id.to_string(),
            )]),
        );
        if let Some(sid) = source_session_id {
            metadata.insert(
                "source_session_ids".to_string(),
                serde_json::Value::Array(vec![serde_json::Value::String(sid.to_string())]),
            );
            metadata.insert(
                "session_id".to_string(),
                serde_json::Value::String(sid.to_string()),
            );
        }
        metadata.insert(
            "claim_scope".to_string(),
            serde_json::Value::String("fact".to_string()),
        );
        // Mark as inline-extracted for retrieval boosting
        metadata.insert(
            "fact_extraction".to_string(),
            serde_json::Value::String("inline".to_string()),
        );
        // Set explicit trust weight for retrieval scoring boost
        metadata.insert(
            "trust_weight".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(2.0).expect("2.0 is always valid JSON number")),
        );

        let mut node = FractalNode::new_typed(
            Some(content),
            None,
            embedding,
            metadata,
            MemoryType::Decision,
            MemorySource::Consolidation,
        );
        node.importance = 9; // High importance — facts are key knowledge
        node.confidence = self.confidence.clamp(0.0, 1.0);
        node.context_tier = ContextTier::Overview;
        node.parent_tier_id = Some(source_node_id);
        // Evidence grounding: link back to the source passage
        node.source_memory_id = Some(source_node_id);
        // Boost weight for retrieval — fact nodes should rank high
        node.weight = 2.0;
        node
    }
}

// ── 6 Regex Rules for Lightweight Fact Extraction ──

/// Rule 1: Explicit Preference
///
/// Matches: "I like/prefer/enjoy/love/favor X"
/// German: "ich mag/liebe/bevorzuge X"
/// Confidence: 0.85 (strong signal)
static RULE_PREFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:I\s+(?:really\s+)?(?:like|prefer|enjoy|love|favor(?:ite)?|hate|dislike)|ich\s+(?:mag|liebe|bevorzuge|hasse))\s+(.+?)(?:\.|$|\s+because|\s+since|\s+as\s+it)",
    )
    .expect("invalid regex pattern")
});

/// Rule 2: Explicit Decision Statement
///
/// Matches: "I decided/chose/went with X", "we decided to X"
/// Also: "DECISION:" prefix, "Entscheidung", "entschieden"
/// Confidence: 0.90 (very strong signal)
static RULE_DECISION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:(?:I|we)\s+(?:decided|chose|went\s+with|opted\s+for)\s+(?:to\s+)?(.+?)(?:\.|$|\s+because|\s+since))|(?:DECISION:\s*(.+?)(?:\.|$))|(?:(?:Entscheidung|entschieden)[:\s]+(.+?)(?:\.|$))",
    )
    .expect("invalid regex pattern")
});

/// Rule 3: Change Over Time / Supersession
///
/// Matches: "no longer", "used to X", "I now X", "not anymore", "changed mind"
/// German: "nicht mehr", "früher", "jetzt", "geändert"
/// Confidence: 0.80 (temporal change signal)
static RULE_CHANGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:(?:no\s+longer|used\s+to|I\s+now|I\s+currently|changed\s+(?:my\s+)?mind|not\s+anymore|I've\s+changed)\s+(.+?)(?:\.|$))|(?:(?:nicht\s+mehr|früher|jetzt|geändert)\s+(.+?)(?:\.|$))",
    )
    .expect("invalid regex pattern")
});

/// Rule 4: Key Fact Assertion (Subject–Verb–Object pattern)
///
/// Matches: "The X is Y", "X was Y", "X has Y", "X → Y"
/// More restrained — only captures with strong connectors.
/// Confidence: 0.60 (lower — many false positives possible)
static RULE_FACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:the\s+\w+(?:\s+\w+){0,3}\s+(?:is|was|are|were|has|have)\s+(?:a\s+|an\s+|the\s+)?\w[\w\s]{5,60}?)(?:\.|$)",
    )
    .expect("invalid regex pattern")
});

/// Rule 5: Future Intent / Plan
///
/// Matches: "I will X", "I plan to X", "I want to X", "I'm going to X"
/// German: "ich werde", "ich plane", "ich möchte"
/// Confidence: 0.70 (intent, not yet a fact)
static RULE_INTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:I\s+(?:will|plan(?:\s+to)?|want(?:\s+to)?|am\s+going\s+to|intend\s+to)\s+(.+?)(?:\.|$))|(?:(?:ich\s+werde|ich\s+plane|ich\s+möchte)\s+(.+?)(?:\.|$))",
    )
    .expect("invalid regex pattern")
});

/// Rule 6: Correction / "Actually" / "Update"
///
/// Matches: "Actually, X", "Correction: X", "Update: X", "Wait, X"
/// Indicates supersession of a prior belief.
/// Confidence: 0.75
static RULE_CORRECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:actually[,:]?\s+(.+?)(?:\.|$)|correction[,:]?\s*(.+?)(?:\.|$)|update[,:]?\s*(.+?)(?:\.|$)|wait[,:]?\s*(.+?)(?:\.|$))",
    )
    .expect("invalid regex pattern")
});

/// Context for fact extraction at storage time.
pub struct FactExtractionContext<'a> {
    /// The session ID (if known) — propagated to extracted fact nodes.
    pub session_id: Option<&'a str>,
    /// The source node ID that generated these facts.
    pub source_node_id: Uuid,
    /// Optional embedding provider dimension (used for zero-vector facts).
    pub embedding_dim: usize,
}

/// Lightweight fact extractor using regex rules.
///
/// No LLM required. Runs at ingest time to extract obvious facts
/// before async consolidation has a chance to run.
pub struct FactExtractor;

impl FactExtractor {
    /// Extract facts from raw text using all 6 regex rules.
    ///
    /// Returns deduplicated facts sorted by confidence (highest first).
    /// Caps at 12 facts per document to avoid over-generation.
    pub fn extract_facts(text: &str) -> Vec<ExtractedFact> {
        let mut facts: Vec<ExtractedFact> = Vec::new();

        // Rule 1: Preferences (high signal)
        for cap in RULE_PREFERENCE.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                let claim = m.as_str().trim().to_string();
                if claim.len() >= 4 && !claim.eq_ignore_ascii_case("it") {
                    facts.push(ExtractedFact {
                        claim: claim.clone(),
                        reason: String::new(),
                        rule: "preference".to_string(),
                        confidence: 0.85,
                        matched_span: claim,
                    });
                }
            }
        }

        // Rule 2: Decisions (strongest signal)
        for cap in RULE_DECISION.captures_iter(text) {
            // Try each capture group
            for group_idx in 1..=3 {
                if let Some(m) = cap.get(group_idx) {
                    let claim = m.as_str().trim().to_string();
                    if claim.len() >= 4 {
                        facts.push(ExtractedFact {
                            claim: claim.clone(),
                            reason: String::new(),
                            rule: "decision".to_string(),
                            confidence: 0.90,
                            matched_span: claim,
                        });
                        break; // Only one group per match
                    }
                }
            }
        }

        // Rule 3: Changes (temporal signal)
        for cap in RULE_CHANGE.captures_iter(text) {
            for group_idx in 1..=2 {
                if let Some(m) = cap.get(group_idx) {
                    let claim = m.as_str().trim().to_string();
                    if claim.len() >= 4 {
                        facts.push(ExtractedFact {
                            claim: claim.clone(),
                            reason: "change detected".to_string(),
                            rule: "change".to_string(),
                            confidence: 0.80,
                            matched_span: claim,
                        });
                        break;
                    }
                }
            }
        }

        // Rule 4: Key facts (lower confidence)
        for cap in RULE_FACT.captures_iter(text) {
            if let Some(m) = cap.get(0) {
                let claim = m.as_str().trim().to_string();
                // Filter noise: skip very short or overly generic matches
                if claim.len() >= 10 && claim.len() <= 120 {
                    facts.push(ExtractedFact {
                        claim: claim.clone(),
                        reason: String::new(),
                        rule: "fact".to_string(),
                        confidence: 0.60,
                        matched_span: claim,
                    });
                }
            }
        }

        // Rule 5: Future intent
        for cap in RULE_INTENT.captures_iter(text) {
            for group_idx in 1..=2 {
                if let Some(m) = cap.get(group_idx) {
                    let claim = m.as_str().trim().to_string();
                    if claim.len() >= 4 {
                        facts.push(ExtractedFact {
                            claim: claim.clone(),
                            reason: "future intent".to_string(),
                            rule: "intent".to_string(),
                            confidence: 0.70,
                            matched_span: claim,
                        });
                        break;
                    }
                }
            }
        }

        // Rule 6: Corrections
        for cap in RULE_CORRECTION.captures_iter(text) {
            for group_idx in 1..=4 {
                if let Some(m) = cap.get(group_idx) {
                    let claim = m.as_str().trim().to_string();
                    if claim.len() >= 4 {
                        facts.push(ExtractedFact {
                            claim: claim.clone(),
                            reason: "correction of prior belief".to_string(),
                            rule: "correction".to_string(),
                            confidence: 0.75,
                            matched_span: claim,
                        });
                        break;
                    }
                }
            }
        }

        // Deduplicate by claim text (case-insensitive)
        facts.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut seen = std::collections::HashSet::new();
        facts.retain(|f| {
            let key = f.claim.to_lowercase();
            seen.insert(key)
        });

        // Cap at 12 to avoid over-generation during inline extraction
        facts.truncate(12);
        facts
    }

    /// Extract facts and convert directly to FractalNodes.
    ///
    /// Uses zero vectors (will be re-embedded by the caller or during
    /// next consolidation cycle). This is the inline path — we don't
    /// want to block on embedding for every fact extraction.
    pub fn extract_and_create_nodes(
        text: &str,
        ctx: &FactExtractionContext,
    ) -> Vec<FractalNode> {
        let facts = Self::extract_facts(text);
        let zero_vector = vec![0.0f32; ctx.embedding_dim];
        facts
            .into_iter()
            .map(|f| f.to_fractal_node(ctx.source_node_id, ctx.session_id, zero_vector.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preference_extraction() {
        let text = "I really like Rust for systems programming. I also enjoy hiking in the mountains.";
        let facts = FactExtractor::extract_facts(text);
        assert!(
            facts.iter().any(|f| f.claim.contains("Rust")),
            "should extract Rust preference"
        );
        assert!(
            facts.iter().any(|f| f.rule == "preference"),
            "should use preference rule"
        );
    }

    #[test]
    fn test_decision_extraction() {
        let text = "I decided to use PostgreSQL instead of SQLite because it scales better.";
        let facts = FactExtractor::extract_facts(text);
        assert!(
            facts.iter().any(|f| f.claim.contains("PostgreSQL")),
            "should extract decision about PostgreSQL"
        );
    }

    #[test]
    fn test_change_extraction() {
        let text = "I no longer use Docker — switched to native macOS. I used to prefer Python but now I write everything in Rust.";
        let facts = FactExtractor::extract_facts(text);
        assert!(
            facts.iter().any(|f| f.rule == "change"),
            "should detect change"
        );
    }

    #[test]
    fn test_intent_extraction() {
        let text = "I will add fact extraction to the codebase this week. I plan to finish by Friday.";
        let facts = FactExtractor::extract_facts(text);
        assert!(
            facts.iter().any(|f| f.rule == "intent"),
            "should detect future intent"
        );
    }

    #[test]
    fn test_correction_extraction() {
        let text = "Actually, the port should be 3737 not 3000. Correction: the API key is stored in .env.";
        let facts = FactExtractor::extract_facts(text);
        assert!(
            facts.iter().any(|f| f.rule == "correction"),
            "should detect corrections"
        );
    }

    #[test]
    fn test_deduplication() {
        let text = "I like Rust. I like Rust. I like Rust. I like Rust.";
        let facts = FactExtractor::extract_facts(text);
        // Should only have one "I like Rust" entry
        let rust_count = facts.iter().filter(|f| f.claim.contains("Rust")).count();
        assert_eq!(rust_count, 1, "should deduplicate by claim text");
    }

    #[test]
    fn test_cap_at_twelve() {
        // Generate text with many facts
        let mut text = String::new();
        for i in 0..20 {
            text.push_str(&format!(
                "I like activity {}. I decided to do task {}. ",
                i, i
            ));
        }
        let facts = FactExtractor::extract_facts(&text);
        assert!(facts.len() <= 12, "should cap at 12 facts: got {}", facts.len());
    }

    #[test]
    fn test_german_preferences() {
        let text = "Ich mag Rust für Systemprogrammierung. Ich liebe Wandern in den Bergen.";
        let facts = FactExtractor::extract_facts(text);
        assert!(
            facts.iter().any(|f| f.claim.contains("Rust")),
            "should extract German preference"
        );
    }

    #[test]
    fn test_fact_to_node() {
        let fact = ExtractedFact {
            claim: "Rust is the primary language".to_string(),
            reason: "performance".to_string(),
            rule: "decision".to_string(),
            confidence: 0.90,
            matched_span: "Rust is the primary language".to_string(),
        };
        let node = fact.to_fractal_node(
            Uuid::new_v4(),
            Some("session-1"),
            vec![0.0; 768],
        );
        assert_eq!(node.memory_type, MemoryType::Decision);
        assert_eq!(node.importance, 9);
        assert!(node.weight >= 2.0);
        assert!(node.metadata.contains_key("decision_what"));
        assert!(node.metadata.contains_key("decision_why"));
        assert!(node.metadata.contains_key("fact_extraction"));
    }
}

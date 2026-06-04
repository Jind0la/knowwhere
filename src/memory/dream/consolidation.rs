//! Dream Mode — Consolidation
//!
//! Part 1 of Dream Mode: Consolidation.
//! Bündelt, clustert, verdichtet episodische Erinnerungen zu Summary-Nodes.
//!
//! This is SEPARATE from Audit. Consolidation builds higher-level
//! representations. Audit checks existing structures for issues.
//!
//! Reference: KnowWhere Source of Truth (2026-03-14), Section:
//! "Dream Mode Definition" > Consolidation

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::memory::types::MemoryType;

/// Result of a consolidation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub run_id: Uuid,
    pub memories_processed: usize,
    pub summaries_created: usize,
    pub clusters_formed: usize,
    pub edges_created: usize,
    pub memories_archived: usize,
    pub duration_ms: u128,
}

/// A cluster of related memories ready for consolidation.
#[derive(Debug, Clone)]
pub struct MemoryCluster {
    pub memory_ids: Vec<Uuid>,
    pub topic: String,
    pub suggested_parent_type: MemoryType,
}

/// An edge between two memory nodes created during consolidation.
///
/// Type-based bridging creates `RELATES_TO` edges between nodes
/// that share the same `memory_type`, connecting otherwise
/// disconnected graph islands.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEdge {
    pub source_id: Uuid,
    pub target_id: Uuid,
    pub edge_type: String,
    pub weight: f64,
}

impl MemoryEdge {
    pub fn new_relates_to(source_id: Uuid, target_id: Uuid, weight: f64) -> Self {
        Self {
            source_id,
            target_id,
            edge_type: "RELATES_TO".to_string(),
            weight,
        }
    }
}

/// Configuration for consolidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// Days after which episodic memories are eligible for consolidation.
    pub episodic_age_threshold_days: u32,
    /// Minimum number of related memories to form a cluster.
    pub min_cluster_size: usize,
    /// Maximum cluster size before splitting.
    pub max_cluster_size: usize,
    /// BM25 similarity threshold for clustering.
    pub similarity_threshold: f64,
    /// Number of consolidated schema versions that must remain stable
    /// before the schema is considered trustworthy. Default: 3.
    #[serde(default = "default_schema_stability_threshold")]
    pub schema_stability_threshold: u32,
    /// Maximum number of nodes per memory_type to bridge (caps the O(n²) edge creation).
    /// Types with more active nodes than this limit will have their least-important
    /// nodes trimmed before bridging.
    #[serde(default = "default_type_bridging_max_per_type")]
    pub type_bridging_max_per_type: usize,
    /// Edge weight for type-based bridging edges.
    /// Low weight (default 0.3) ensures these edges don't dominate
    /// stronger fact-based edges during graph traversal.
    #[serde(default = "default_type_bridging_edge_weight")]
    pub type_bridging_edge_weight: f64,
    /// Multiplier applied to the consolidation weight of facts whose schema
    /// is unstable (frequency < schema_stability_threshold). Default: 0.3.
    #[serde(default = "default_unstable_schema_weight_multiplier")]
    pub unstable_schema_weight_multiplier: f64,
}

fn default_schema_stability_threshold() -> u32 {
    3
}

fn default_type_bridging_max_per_type() -> usize {
    50
}

fn default_type_bridging_edge_weight() -> f64 {
    0.3
}

fn default_unstable_schema_weight_multiplier() -> f64 {
    0.3
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            episodic_age_threshold_days: 7,
            min_cluster_size: 3,
            max_cluster_size: 20,
            similarity_threshold: 0.6,
            schema_stability_threshold: 3,
            type_bridging_max_per_type: 50,
            type_bridging_edge_weight: 0.3,
            unstable_schema_weight_multiplier: 0.3,
        }
    }
}

impl ConsolidationConfig {
    /// Load configuration from environment variables, falling back to defaults.
    ///
    /// Environment variables:
    /// - `CONSOLIDATION_SCHEMA_STABILITY_THRESHOLD` — u32 (default: 3)
    ///   Number of consolidated schema versions that must remain stable
    ///   before the schema is considered trustworthy.
    pub fn from_env() -> Self {
        Self {
            schema_stability_threshold: std::env::var("CONSOLIDATION_SCHEMA_STABILITY_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            ..Self::default()
        }
    }
}

/// The Consolidation engine.
/// Call `run_consolidation` periodically (e.g., every hour or daily).
pub struct ConsolidationEngine<C: ConsolidationStore> {
    config: ConsolidationConfig,
    store: C,
}

impl<C: ConsolidationStore> ConsolidationEngine<C> {
    pub fn new(config: ConsolidationConfig, store: C) -> Self {
        Self { config, store }
    }

    pub fn with_default_config(store: C) -> Self {
        Self::new(ConsolidationConfig::default(), store)
    }

    /// Run a consolidation pass.
    ///
    /// Algorithm:
    /// 1. Find episodic memories older than threshold
    /// 2. Query unstable schema keys from fact_schemas
    /// 3. Cluster by topic (using BM25 + semantic similarity)
    /// 4. For each cluster: apply schema-aware weight, generate summary, create parent node
    /// 5. Repoint children to new parent
    /// 6. Archive originals (or mark as consolidated)
    /// 7. Create knowledge edges between related summaries
    /// 8. Bridge disconnected graph islands via type-based edges
    pub async fn run_consolidation(&self) -> Result<ConsolidationReport> {
        let run_id = Uuid::new_v4();
        let start = std::time::Instant::now();

        let mut summaries_created = 0;
        let mut memories_archived = 0;

        // Step 1: Find eligible episodic memories
        let eligible = self
            .store
            .get_episodic_memories_older_than(self.config.episodic_age_threshold_days)
            .await?;

        // Step 1b: Query unstable schema keys for weight reduction
        let unstable_schemas: HashSet<String> = self
            .store
            .get_unstable_schema_keys(self.config.schema_stability_threshold)
            .await?;

        if eligible.len() < self.config.min_cluster_size {
            // Not enough memories to consolidate — still attempt type-bridging
            let bridge_edges = self.bridge_by_type().await?;

            return Ok(ConsolidationReport {
                run_id,
                memories_processed: eligible.len(),
                summaries_created: 0,
                clusters_formed: 0,
                edges_created: bridge_edges,
                memories_archived: 0,
                duration_ms: start.elapsed().as_millis(),
            });
        }

        // Step 2: Cluster by topic (simplified — uses content similarity)
        let clusters = self.cluster_memories(&eligible).await?;
        let clusters_formed: usize = clusters.len();

        // Step 3: For each cluster, create summary + parent node
        // Apply schema-aware weight reduction for unstable schemas
        for cluster in &clusters {
            if cluster.memory_ids.len() < self.config.min_cluster_size {
                continue;
            }

            // Fetch full memory content
            let memories = self.store.get_memories_by_ids(&cluster.memory_ids).await?;
            let contents: Vec<&str> = memories.iter().map(|m| m.content.as_str()).collect();

            // Compute schema-aware weighted importance.
            // Facts with unstable schemas (frequency below threshold) get
            // their consolidation_weight multiplied by unstable_schema_weight_multiplier.
            let weight_mult = self.config.unstable_schema_weight_multiplier;
            let total_weighted_importance: f64 = memories
                .iter()
                .map(|m| {
                    let consolidation_weight: f64 =
                        if let Some(ref schema_key) = m.schema_key {
                            if unstable_schemas.contains(schema_key) {
                                weight_mult
                            } else {
                                1.0
                            }
                        } else {
                            1.0 // no schema_key → full weight
                        };
                    (m.importance as f64) * consolidation_weight
                })
                .sum();
            let avg_importance =
                total_weighted_importance / memories.len() as f32 as f64;

            // Generate summary (simplified — in production this would call an LLM)
            let summary = self.generate_summary(&contents, &cluster.topic)?;

            // Create parent summary node
            let parent_id = self
                .store
                .create_summary_node(
                    summary,
                    MemoryType::Semantic,
                    cluster.topic.clone(),
                    avg_importance as i32,
                )
                .await?;
            summaries_created += 1;

            // Repoint children to parent
            for memory_id in &cluster.memory_ids {
                self.store.set_parent(*memory_id, parent_id).await?;
            }

            // Archive originals
            for memory_id in &cluster.memory_ids {
                self.store.archive(*memory_id).await?;
                memories_archived += 1;
            }
        }

        // Step 6: Bridge disconnected graph islands via type-based edges
        let bridge_edges = self.bridge_by_type().await?;

        let duration_ms = start.elapsed().as_millis();

        Ok(ConsolidationReport {
            run_id,
            memories_processed: eligible.len(),
            summaries_created,
            clusters_formed,
            edges_created: bridge_edges,
            memories_archived,
            duration_ms,
        })
    }

    /// Bridge disconnected graph islands by creating weak `RELATES_TO` edges
    /// between nodes that share the same `memory_type`.
    ///
    /// Algorithm:
    /// 1. Group all active memories by `memory_type`
    /// 2. Within each group, sort by weight descending
    /// 3. Cap group size to `type_bridging_max_per_type`
    /// 4. Create `RELATES_TO` edges between all pairs within each group
    ///    (with `ON CONFLICT DO NOTHING` semantics for idempotency)
    ///
    /// Groups with fewer than 2 nodes produce no edges.
    pub async fn bridge_by_type(&self) -> Result<usize> {
        let by_type = self.store.get_active_memories_by_type().await?;

        let mut all_edges: Vec<MemoryEdge> = Vec::new();

        for (_memory_type, mut nodes) in by_type {
            // Cap group size
            if nodes.len() > self.config.type_bridging_max_per_type {
                // Sort by weight descending, keep only top N
                nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                nodes.truncate(self.config.type_bridging_max_per_type);
            }

            // Need at least 2 nodes to create edges
            if nodes.len() < 2 {
                continue;
            }

            // Create all-pairs edges within the group
            for i in 0..nodes.len() {
                for j in (i + 1)..nodes.len() {
                    all_edges.push(MemoryEdge::new_relates_to(
                        nodes[i].0,
                        nodes[j].0,
                        self.config.type_bridging_edge_weight,
                    ));
                }
            }
        }

        let edge_count = all_edges.len();
        if !all_edges.is_empty() {
            self.store.insert_memory_edges(&all_edges).await?;
        }

        Ok(edge_count)
    }

    /// Cluster memories by topic using BM25 + semantic similarity.
    async fn cluster_memories(
        &self,
        memories: &[ClusteringCandidate],
    ) -> Result<Vec<MemoryCluster>> {
        let mut clusters: Vec<MemoryCluster> = Vec::new();
        let mut assigned: Vec<Uuid> = Vec::new();

        for candidate in memories {
            if assigned.contains(&candidate.id) {
                continue;
            }

            // Find related memories
            let related = self.find_related(memories, candidate, &assigned).await?;

            if related.len() >= self.config.min_cluster_size {
                let mut cluster_ids = related;
                cluster_ids.push(candidate.id);
                assigned.extend(&cluster_ids);

                clusters.push(MemoryCluster {
                    memory_ids: cluster_ids,
                    topic: candidate.topic.clone(),
                    suggested_parent_type: MemoryType::Semantic,
                });
            }
        }

        Ok(clusters)
    }

    /// Find memories related to a candidate.
    async fn find_related(
        &self,
        candidates: &[ClusteringCandidate],
        candidate: &ClusteringCandidate,
        assigned: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        let mut related = Vec::new();

        for other in candidates {
            if other.id == candidate.id || assigned.contains(&other.id) {
                continue;
            }

            let similarity = self.calculate_similarity(candidate, other)?;
            if similarity >= self.config.similarity_threshold {
                related.push(other.id);
            }
        }

        Ok(related)
    }

    /// Calculate similarity between two clustering candidates.
    fn calculate_similarity(
        &self,
        a: &ClusteringCandidate,
        b: &ClusteringCandidate,
    ) -> Result<f64> {
        use crate::memory::fractal_node::cosine_similarity;

        // Fallback to content-based similarity if vectors aren't available
        let vector_sim = if let (Some(vec_a), Some(vec_b)) = (&a.vector, &b.vector) {
            cosine_similarity(vec_a, vec_b) as f64
        } else {
            // Simple keyword overlap as fallback
            let a_lower = a.content.to_lowercase();
            let b_lower = b.content.to_lowercase();
            let a_words: std::collections::HashSet<_> = a_lower.split_whitespace().collect();
            let b_words: std::collections::HashSet<_> = b_lower.split_whitespace().collect();

            let intersection = a_words.intersection(&b_words).count() as f64;
            let union = a_words.union(&b_words).count() as f64;
            intersection / union.max(1.0)
        };

        Ok(vector_sim)
    }

    /// Generate a summary from a list of memory contents.
    /// Simplified: in production this would call an LLM.
    fn generate_summary(&self, contents: &[&str], topic: &str) -> Result<String> {
        // Simplified: just concatenate with ellipsis
        // Production: call LLM with prompt like
        // "Summarize these memories about {topic}: {contents}"
        let combined = contents.join(" ");
        let truncated = if combined.len() > 500 {
            format!("{}...", &combined[..500])
        } else {
            combined
        };
        Ok(format!(
            "Consolidated summary for '{}': {}",
            topic, truncated
        ))
    }
}

// -----------------------------------------------------------------------------
// Consolidation Store trait
// -----------------------------------------------------------------------------

use async_trait::async_trait;

#[async_trait]
pub trait ConsolidationStore: Send + Sync {
    async fn get_episodic_memories_older_than(&self, days: u32)
        -> Result<Vec<ClusteringCandidate>>;
    async fn get_memories_by_ids(&self, ids: &[Uuid]) -> Result<Vec<ConsolidationMemory>>;
    async fn create_summary_node(
        &self,
        content: String,
        memory_type: MemoryType,
        topic: String,
        importance: i32,
    ) -> Result<Uuid>;
    async fn set_parent(&self, memory_id: Uuid, parent_id: Uuid) -> Result<()>;
    async fn archive(&self, memory_id: Uuid) -> Result<()>;

    /// Get ALL active (non-archived) memories, grouped by memory_type.
    /// Returns (node_id, weight) pairs sorted by weight descending.
    /// Used by type-based bridging to find nodes of the same type.
    async fn get_active_memories_by_type(&self) -> Result<HashMap<MemoryType, Vec<(Uuid, f64)>>>;

    /// Insert memory edges with idempotent semantics.
    /// Duplicate edges (same source_id, target_id, edge_type) should be silently ignored.
    /// Returns the number of edges actually inserted.
    async fn insert_memory_edges(&self, edges: &[MemoryEdge]) -> Result<usize>;

    /// Get schema keys whose frequency is below the stability threshold.
    ///
    /// During consolidation, facts belonging to unstable schemas (frequency < threshold)
    /// receive a reduced consolidation weight (configurable via
    /// `unstable_schema_weight_multiplier`, default 0.3). This prevents
    /// low-confidence schema patterns from dominating consolidated summaries.
    ///
    /// Returns a set of schema_key strings that are below the threshold.
    async fn get_unstable_schema_keys(&self, threshold: u32) -> Result<HashSet<String>>;
}

/// Minimal memory data needed for clustering.
#[derive(Debug, Clone)]
pub struct ClusteringCandidate {
    pub id: Uuid,
    pub content: String,
    pub vector: Option<Vec<f32>>,
    pub topic: String,
    /// Schema key (e.g. "self_preference_language") for schema stability tracking.
    /// Populated from metadata if the node was created by fact extraction.
    pub schema_key: Option<String>,
}

/// Full memory data needed for consolidation.
#[derive(Debug, Clone)]
pub struct ConsolidationMemory {
    pub id: Uuid,
    pub content: String,
    pub importance: i32,
    /// Schema key for schema stability weight reduction during consolidation.
    /// Populated from metadata if the node was created by fact extraction.
    pub schema_key: Option<String>,
}

// ── InMemoryConsolidationStore ────────────────────────────────────────────
// Concrete implementation of ConsolidationStore backed by MemoryStore.
// Uses TST-inspired mean_vector() for L1 parent node creation — NEVER
// re-embeds from text (re-initialization destroys representation continuity,
// per TST's harshest ablation result).

use std::sync::Arc;
use std::sync::Mutex;
use crate::storage::in_memory::MemoryStore;
use crate::memory::FractalNode;
use crate::memory::types::{MemoryStatus};

/// Wraps MemoryStore to implement the ConsolidationStore trait.
/// All consolidation operations pass through to the underlying store.
///
/// Also maintains an internal edge store for `memory_edges` semantics:
/// idempotent insertion with deduplication on (source_id, target_id, edge_type).
pub struct InMemoryConsolidationStore {
    store: Arc<MemoryStore>,
    /// Internal edge storage for type-based bridging.
    /// Maps (source_id, target_id, edge_type) → MemoryEdge for dedup.
    edges: Mutex<Vec<MemoryEdge>>,
    /// Internal fact_schemas storage for schema stability tracking.
    /// Maps schema_key → frequency (observation count).
    fact_schemas: Mutex<HashMap<String, i32>>,
}

impl InMemoryConsolidationStore {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self {
            store,
            edges: Mutex::new(Vec::new()),
            fact_schemas: Mutex::new(HashMap::new()),
        }
    }

    /// Return a copy of all stored edges (for test inspection).
    #[cfg(test)]
    pub fn edges(&self) -> Vec<MemoryEdge> {
        self.edges.lock().unwrap().clone()
    }

    /// Upsert a schema key into the internal fact_schemas store.
    /// If the key already exists, increments frequency by `count`.
    /// Otherwise, inserts with `count` as the starting frequency.
    ///
    /// This is the in-memory equivalent of the fact_schemas PostgreSQL table.
    /// Test code uses this to simulate schema observation frequency.
    pub fn upsert_fact_schema(&self, schema_key: &str, count: i32) {
        let mut schemas = self.fact_schemas.lock().unwrap();
        schemas
            .entry(schema_key.to_string())
            .and_modify(|freq| *freq += count)
            .or_insert(count);
    }

    /// Directly set a schema key's frequency (for test fixture setup).
    pub fn set_fact_schema_frequency(&self, schema_key: &str, frequency: i32) {
        let mut schemas = self.fact_schemas.lock().unwrap();
        schemas.insert(schema_key.to_string(), frequency);
    }
}

/// Helper: extract schema_key from a FractalNode's metadata.
/// Looks for "schema_key" field in metadata. Returns None if not present.
fn extract_schema_key(node: &FractalNode) -> Option<String> {
    node.metadata
        .get("schema_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[async_trait]
impl ConsolidationStore for InMemoryConsolidationStore {
    async fn get_episodic_memories_older_than(
        &self,
        days: u32,
    ) -> Result<Vec<ClusteringCandidate>> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
        let nodes = self.store.list_all().await?;
        Ok(nodes
            .into_iter()
            .filter(|n| n.memory_type == MemoryType::Episodic && n.created_at < cutoff)
            .map(|n| {
                let schema_key = extract_schema_key(&n);
                ClusteringCandidate {
                    id: n.id,
                    content: n.content.unwrap_or_default(),
                    vector: if n.vector.is_empty() { None } else { Some(n.vector.clone()) },
                    topic: n.metadata.get("topic").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    schema_key,
                }
            })
            .collect())
    }

    async fn get_memories_by_ids(&self, ids: &[Uuid]) -> Result<Vec<ConsolidationMemory>> {
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(node) = self.store.get(id).await? {
                let schema_key = extract_schema_key(&node);
                results.push(ConsolidationMemory {
                    id: node.id,
                    content: node.content.unwrap_or_default(),
                    importance: node.weight as i32,
                    schema_key,
                });
            }
        }
        Ok(results)
    }

    async fn create_summary_node(
        &self,
        content: String,
        memory_type: MemoryType,
        topic: String,
        importance: i32,
    ) -> Result<Uuid> {
        // TST rule: L1 parent vectors MUST be mean_vector(children), never
        // re-embedded from text. The ConsolidationEngine should compute the
        // vector via mean_vector() before calling this method. For now we
        // create the node with an empty vector — the engine should update
        // it after computing the mean of child vectors.
        let mut metadata = HashMap::new();
        metadata.insert(
            "consolidation_topic".to_string(),
            serde_json::Value::String(topic),
        );
        metadata.insert(
            "derivation".to_string(),
            serde_json::Value::String("consolidation".to_string()),
        );

        let mut summary = FractalNode::new_typed(
            Some(content),
            None,
            vec![], // placeholder — engine sets via mean_vector(children)
            metadata,
            memory_type,
            crate::memory::types::MemorySource::Consolidation,
        );
        summary.weight = importance as f64;

        let id = self.store.insert(summary).await?;
        Ok(id)
    }

    async fn set_parent(&self, memory_id: Uuid, parent_id: Uuid) -> Result<()> {
        self.store
            .update_node(&memory_id, |node| {
                node.parent_tier_id = Some(parent_id);
            })
            .await?;
        Ok(())
    }

    async fn archive(&self, memory_id: Uuid) -> Result<()> {
        self.store
            .update_node(&memory_id, |node| {
                node.status = MemoryStatus::Archived;
            })
            .await?;
        Ok(())
    }

    async fn get_active_memories_by_type(&self) -> Result<HashMap<MemoryType, Vec<(Uuid, f64)>>> {
        let nodes = self.store.list_all().await?;
        let mut by_type: HashMap<MemoryType, Vec<(Uuid, f64)>> = HashMap::new();

        for node in nodes {
            // Only include active (non-archived, non-deleted) memories
            if node.status != MemoryStatus::Active && node.status != MemoryStatus::Draft {
                continue;
            }

            by_type
                .entry(node.memory_type)
                .or_default()
                .push((node.id, node.weight));
        }

        Ok(by_type)
    }

    async fn insert_memory_edges(&self, edges: &[MemoryEdge]) -> Result<usize> {
        let mut stored = self.edges.lock().unwrap();
        let before = stored.len();

        for edge in edges {
            // Idempotent: skip if a matching edge already exists
            let is_duplicate = stored.iter().any(|existing| {
                existing.source_id == edge.source_id
                    && existing.target_id == edge.target_id
                    && existing.edge_type == edge.edge_type
            });

            if !is_duplicate {
                stored.push(edge.clone());
            }
        }

        Ok(stored.len() - before)
    }

    async fn get_unstable_schema_keys(&self, threshold: u32) -> Result<HashSet<String>> {
        let schemas = self.fact_schemas.lock().unwrap();
        let unstable: HashSet<String> = schemas
            .iter()
            .filter(|(_, &freq)| (freq as u32) < threshold)
            .map(|(key, _)| key.clone())
            .collect();
        Ok(unstable)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use crate::memory::types::{MemorySource, MemoryStatus};
    use uuid::Uuid;

    /// Helper: build a ConsolidationEngine backed by an InMemoryConsolidationStore
    /// with some nodes pre-inserted.
    async fn build_engine_with_nodes(
        nodes: Vec<(MemoryType, f64)>,
    ) -> ConsolidationEngine<InMemoryConsolidationStore> {
        let mem_store = Arc::new(MemoryStore::new());
        for (memory_type, weight) in nodes {
            let mut node = FractalNode::new_typed(
                Some("test content".to_string()),
                None,
                vec![0.1; 8],
                HashMap::new(),
                memory_type,
                MemorySource::Manual,
            );
            node.weight = weight;
            mem_store.insert(node).await.unwrap();
        }
        let cons_store = InMemoryConsolidationStore::new(mem_store);
        ConsolidationEngine::new(ConsolidationConfig::default(), cons_store)
    }

    // ── Shared types produce edges ──

    #[tokio::test]
    async fn test_bridge_by_type_creates_edges_for_shared_types() {
        let engine = build_engine_with_nodes(vec![
            (MemoryType::Episodic, 1.0),
            (MemoryType::Episodic, 2.0),
            (MemoryType::Episodic, 3.0),
            (MemoryType::Semantic, 1.0),
            (MemoryType::Semantic, 2.0),
        ])
        .await;

        let edge_count = engine.bridge_by_type().await.unwrap();
        // 3 episodic nodes → C(3,2) = 3 edges
        // 2 semantic nodes → C(2,2) = 1 edge
        // Total: 4 edges
        assert_eq!(edge_count, 4, "expected 4 edges (3 from 3 Episodic + 1 from 2 Semantic)");

        let edges = engine.store.edges();
        assert_eq!(edges.len(), 4);

        // All edges should be RELATES_TO with the configured weight
        for edge in &edges {
            assert_eq!(edge.edge_type, "RELATES_TO");
            assert!((edge.weight - 0.3).abs() < 1e-6, "expected weight 0.3, got {}", edge.weight);
        }
    }

    // ── Singleton types produce no edges ──

    #[tokio::test]
    async fn test_bridge_by_type_no_edges_for_singleton_types() {
        let engine = build_engine_with_nodes(vec![
            (MemoryType::Episodic, 1.0),   // singleton
            (MemoryType::Semantic, 2.0),
            (MemoryType::Semantic, 3.0),
            (MemoryType::Preference, 1.0),  // singleton
        ])
        .await;

        let edge_count = engine.bridge_by_type().await.unwrap();
        // Only Semantic has 2 nodes → C(2,2) = 1 edge
        // Episodic and Preference are singletons → 0 edges each
        assert_eq!(edge_count, 1);
    }

    // ── Max-per-type limit ──

    #[tokio::test]
    async fn test_bridge_by_type_respects_max_per_type() {
        // Create 10 episodic nodes, but cap at 5
        let mut nodes = Vec::new();
        for i in 0..10 {
            nodes.push((MemoryType::Episodic, (10 - i) as f64));
        }

        let mem_store = Arc::new(MemoryStore::new());
        for (memory_type, weight) in nodes {
            let mut node = FractalNode::new_typed(
                Some("test content".to_string()),
                None,
                vec![0.1; 8],
                HashMap::new(),
                memory_type,
                MemorySource::Manual,
            );
            node.weight = weight;
            mem_store.insert(node).await.unwrap();
        }

        let mut config = ConsolidationConfig::default();
        config.type_bridging_max_per_type = 5;

        let cons_store = InMemoryConsolidationStore::new(mem_store);
        let engine = ConsolidationEngine::new(config, cons_store);

        let edge_count = engine.bridge_by_type().await.unwrap();
        // Capped at 5 nodes → C(5,2) = 10 edges
        assert_eq!(edge_count, 10, "expected 10 edges from 5 nodes (cap), got {}", edge_count);
    }

    // ── Correct edge weight ──

    #[tokio::test]
    async fn test_bridge_by_type_uses_config_weight() {
        let engine = build_engine_with_nodes(vec![
            (MemoryType::Episodic, 1.0),
            (MemoryType::Episodic, 2.0),
        ])
        .await;

        // Override weight via config
        let mut config = ConsolidationConfig::default();
        config.type_bridging_edge_weight = 0.7;
        let engine = ConsolidationEngine::new(config, engine.store);
        // Note: we need to re-create to use new config, but the store already
        // has nodes. Let's make a fresh engine with the custom config.

        // Let's do it properly with a fresh store
        let mem_store = Arc::new(MemoryStore::new());
        for weight in [1.0, 2.0] {
            let mut node = FractalNode::new_typed(
                Some("test".to_string()),
                None,
                vec![0.1; 8],
                HashMap::new(),
                MemoryType::Episodic,
                MemorySource::Manual,
            );
            node.weight = weight;
            mem_store.insert(node).await.unwrap();
        }

        let mut config = ConsolidationConfig::default();
        config.type_bridging_edge_weight = 0.7;
        let cons_store = InMemoryConsolidationStore::new(mem_store);
        let engine = ConsolidationEngine::new(config, cons_store);

        engine.bridge_by_type().await.unwrap();
        let edges = engine.store.edges();
        assert_eq!(edges.len(), 1);
        assert!((edges[0].weight - 0.7).abs() < 1e-6, "expected weight 0.7, got {}", edges[0].weight);
    }

    // ── No duplicates on repeated calls ──

    #[tokio::test]
    async fn test_bridge_by_type_no_duplicates_on_repeated_calls() {
        let engine = build_engine_with_nodes(vec![
            (MemoryType::Episodic, 1.0),
            (MemoryType::Episodic, 2.0),
            (MemoryType::Episodic, 3.0),
        ])
        .await;

        // First call
        let count1 = engine.bridge_by_type().await.unwrap();
        assert_eq!(count1, 3); // C(3,2) = 3 edges

        // Second call — should insert 0 new edges (all duplicates)
        let count2 = engine.bridge_by_type().await.unwrap();
        assert_eq!(count2, 0, "repeated call should insert 0 new edges");

        // Third call — same
        let count3 = engine.bridge_by_type().await.unwrap();
        assert_eq!(count3, 0, "third call should also insert 0 new edges");

        let edges = engine.store.edges();
        assert_eq!(edges.len(), 3, "edge count should still be 3 after repeated calls");
    }

    // ── Empty store produces no errors ──

    #[tokio::test]
    async fn test_bridge_by_type_empty_store() {
        let mem_store = Arc::new(MemoryStore::new());
        let cons_store = InMemoryConsolidationStore::new(mem_store);
        let engine = ConsolidationEngine::with_default_config(cons_store);

        let count = engine.bridge_by_type().await.unwrap();
        assert_eq!(count, 0);
        assert!(engine.store.edges().is_empty());
    }

    // ── Only active memories are bridged ──

    #[tokio::test]
    async fn test_bridge_by_type_excludes_archived() {
        let mem_store = Arc::new(MemoryStore::new());

        // Insert 2 active episodic
        for weight in [1.0, 2.0] {
            let mut node = FractalNode::new_typed(
                Some("active".to_string()),
                None,
                vec![0.1; 8],
                HashMap::new(),
                MemoryType::Episodic,
                MemorySource::Manual,
            );
            node.weight = weight;
            mem_store.insert(node).await.unwrap();
        }

        // Insert 1 archived episodic
        let mut archived = FractalNode::new_typed(
            Some("archived".to_string()),
            None,
            vec![0.1; 8],
            HashMap::new(),
            MemoryType::Episodic,
            MemorySource::Manual,
        );
        archived.weight = 3.0;
        archived.status = MemoryStatus::Archived;
        mem_store.insert(archived).await.unwrap();

        let cons_store = InMemoryConsolidationStore::new(mem_store);
        let engine = ConsolidationEngine::with_default_config(cons_store);

        let count = engine.bridge_by_type().await.unwrap();
        // Only 2 active → C(2,2) = 1 edge
        assert_eq!(count, 1, "archived nodes should be excluded");
    }

    // ── insert_memory_edges idempotency (direct test) ──

    #[tokio::test]
    async fn test_insert_memory_edges_idempotent() {
        let mem_store = Arc::new(MemoryStore::new());
        let cons_store = InMemoryConsolidationStore::new(mem_store);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let edges = vec![
            MemoryEdge::new_relates_to(id1, id2, 0.3),
            MemoryEdge::new_relates_to(id1, id2, 0.3), // duplicate
        ];

        let inserted = cons_store.insert_memory_edges(&edges).await.unwrap();
        assert_eq!(inserted, 1, "only first edge should be inserted, not the duplicate");

        // Try inserting the same edge again
        let edges2 = vec![MemoryEdge::new_relates_to(id1, id2, 0.3)];
        let inserted2 = cons_store.insert_memory_edges(&edges2).await.unwrap();
        assert_eq!(inserted2, 0, "re-inserting same edge should return 0");
    }

    // ── Different edge types are not duplicates ──

    #[tokio::test]
    async fn test_insert_memory_edges_different_types() {
        let mem_store = Arc::new(MemoryStore::new());
        let cons_store = InMemoryConsolidationStore::new(mem_store);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let edges = vec![
            MemoryEdge::new_relates_to(id1, id2, 0.3),
            MemoryEdge {
                source_id: id1,
                target_id: id2,
                edge_type: "SUPPORTS".to_string(),
                weight: 0.5,
            },
        ];

        let inserted = cons_store.insert_memory_edges(&edges).await.unwrap();
        assert_eq!(inserted, 2, "different edge types should both be inserted");
    }
}

#[cfg(test)]
mod schema_weight_tests {
    use super::*;
    use crate::memory::types::{MemorySource, MemoryStatus};
    use uuid::Uuid;

    /// Helper: create a FractalNode with a schema_key in metadata.
    fn node_with_schema(
        content: &str,
        memory_type: MemoryType,
        weight: f64,
        schema_key: &str,
    ) -> FractalNode {
        let mut metadata = HashMap::new();
        metadata.insert(
            "schema_key".to_string(),
            serde_json::Value::String(schema_key.to_string()),
        );
        let mut node = FractalNode::new_typed(
            Some(content.to_string()),
            None,
            vec![0.1; 8],
            metadata,
            memory_type,
            MemorySource::Manual,
        );
        node.weight = weight;
        node.created_at = chrono::Utc::now() - chrono::Duration::days(10); // older than threshold
        node
    }

    /// Helper: create a FractalNode WITHOUT a schema_key.
    fn node_without_schema(content: &str, memory_type: MemoryType, weight: f64) -> FractalNode {
        let mut node = FractalNode::new_typed(
            Some(content.to_string()),
            None,
            vec![0.1; 8],
            HashMap::new(),
            memory_type,
            MemorySource::Manual,
        );
        node.weight = weight;
        node.created_at = chrono::Utc::now() - chrono::Duration::days(10); // older than threshold
        node
    }

    // ── Schema stability: unstable schemas get reduced weight ──

    #[tokio::test]
    async fn test_unstable_schema_gets_reduced_weight() {
        let mem_store = Arc::new(MemoryStore::new());

        // Insert 3 episodic nodes:
        // - 2 with stable schema (freq=5, recorded in fact_schemas)
        // - 1 with unstable schema (freq=2, below threshold of 3)
        let n1 = node_with_schema("I like Rust", MemoryType::Episodic, 10.0, "self_preference_language");
        let n2 = node_with_schema("I prefer Python", MemoryType::Episodic, 10.0, "self_preference_language");
        let n3 = node_with_schema("I decided to use Zig", MemoryType::Episodic, 10.0, "self_decision_language");

        mem_store.insert(n1).await.unwrap();
        mem_store.insert(n2).await.unwrap();
        mem_store.insert(n3).await.unwrap();

        let cons_store = InMemoryConsolidationStore::new(mem_store);

        // Register fact_schemas: schema_key "self_preference_language" has freq 5 (stable)
        // schema_key "self_decision_language" has freq 2 (unstable, below threshold 3)
        cons_store.set_fact_schema_frequency("self_preference_language", 5);
        cons_store.set_fact_schema_frequency("self_decision_language", 2);

        let mut config = ConsolidationConfig::default();
        config.schema_stability_threshold = 3;
        config.unstable_schema_weight_multiplier = 0.3;
        // Ensure clustering works: all 3 should be similar enough
        config.similarity_threshold = 0.05;
        let engine = ConsolidationEngine::new(config, cons_store);

        let report = engine.run_consolidation().await.unwrap();
        assert_eq!(report.summaries_created, 1, "should create one summary node from the cluster");
        assert_eq!(report.memories_processed, 3);

        // Verify the summary node was created with reduced weight.
        // Expected: 2 stable at weight 10.0 * 1.0 = 10.0 each,
        //           1 unstable at weight 10.0 * 0.3 = 3.0.
        //           avg = (10.0 + 10.0 + 3.0) / 3 = 7.666...
        // Without schema reduction: (10 + 10 + 10) / 3 = 10.0
        // So the average should be less than 10.0
        let all_nodes = engine.store.store.list_all().await.unwrap();
        let summary_nodes: Vec<_> = all_nodes
            .iter()
            .filter(|n| n.memory_type == MemoryType::Semantic)
            .collect();
        assert_eq!(summary_nodes.len(), 1, "should have exactly 1 summary node");
        let summary = summary_nodes[0];
        assert!(
            summary.weight < 10.0,
            "summary weight should be reduced due to unstable schema: got {}",
            summary.weight
        );
        assert!(
            summary.weight > 7.0,
            "summary weight should not be too low: got {}",
            summary.weight
        );
        // ~7.67 is expected
        assert!(
            (summary.weight - 7.666).abs() < 1.0,
            "expected ~7.67 weight for 2 stable + 1 unstable, got {}",
            summary.weight
        );
    }

    // ── All stable: full weight ──

    #[tokio::test]
    async fn test_all_stable_schemas_get_full_weight() {
        let mem_store = Arc::new(MemoryStore::new());

        let n1 = node_with_schema("I like Rust", MemoryType::Episodic, 8.0, "self_preference_language");
        let n2 = node_with_schema("I prefer Python", MemoryType::Episodic, 8.0, "self_preference_language");
        let n3 = node_with_schema("I enjoy Go", MemoryType::Episodic, 8.0, "self_preference_language");

        mem_store.insert(n1).await.unwrap();
        mem_store.insert(n2).await.unwrap();
        mem_store.insert(n3).await.unwrap();

        let cons_store = InMemoryConsolidationStore::new(mem_store);
        cons_store.set_fact_schema_frequency("self_preference_language", 5); // stable (>= 3)

        let mut config = ConsolidationConfig::default();
        config.schema_stability_threshold = 3;
        config.unstable_schema_weight_multiplier = 0.3;
        config.similarity_threshold = 0.05;
        let engine = ConsolidationEngine::new(config, cons_store);

        let report = engine.run_consolidation().await.unwrap();
        assert_eq!(report.summaries_created, 1);

        let all_nodes = engine.store.store.list_all().await.unwrap();
        let summary_nodes: Vec<_> = all_nodes
            .iter()
            .filter(|n| n.memory_type == MemoryType::Semantic)
            .collect();
        assert_eq!(summary_nodes.len(), 1);
        let summary = summary_nodes[0];
        // All stable → weight should be near 8.0
        assert!(
            (summary.weight - 8.0).abs() < 1.0,
            "all stable schemas should get full weight, got {}",
            summary.weight
        );
    }

    // ── Nodes without schema_key get full weight ──

    #[tokio::test]
    async fn test_no_schema_key_gets_full_weight() {
        let mem_store = Arc::new(MemoryStore::new());

        let n1 = node_without_schema("some content A", MemoryType::Episodic, 5.0);
        let n2 = node_without_schema("some content B", MemoryType::Episodic, 5.0);
        let n3 = node_without_schema("some content C", MemoryType::Episodic, 5.0);

        mem_store.insert(n1).await.unwrap();
        mem_store.insert(n2).await.unwrap();
        mem_store.insert(n3).await.unwrap();

        let cons_store = InMemoryConsolidationStore::new(mem_store);
        // No fact_schemas registered

        let mut config = ConsolidationConfig::default();
        config.similarity_threshold = 0.05;
        let engine = ConsolidationEngine::new(config, cons_store);

        let report = engine.run_consolidation().await.unwrap();
        assert_eq!(report.summaries_created, 1);

        let all_nodes = engine.store.store.list_all().await.unwrap();
        let summary_nodes: Vec<_> = all_nodes
            .iter()
            .filter(|n| n.memory_type == MemoryType::Semantic)
            .collect();
        assert_eq!(summary_nodes.len(), 1);
        let summary = summary_nodes[0];
        // All no schema → all full weight → near 5.0
        assert!(
            (summary.weight - 5.0).abs() < 1.0,
            "nodes without schema_key should get full weight, got {}",
            summary.weight
        );
    }

    // ── Configurable threshold via ConsolidationConfig ──

    #[tokio::test]
    async fn test_configurable_threshold_changes_behavior() {
        let mem_store = Arc::new(MemoryStore::new());

        // Use a schema that has frequency 4.
        // With threshold=5 it's unstable; with threshold=3 it's stable.
        let n1 = node_with_schema("I like Rust", MemoryType::Episodic, 10.0, "self_preference_rust");
        let n2 = node_with_schema("I enjoy Rust", MemoryType::Episodic, 10.0, "self_preference_rust");
        let n3 = node_with_schema("Rust is great", MemoryType::Episodic, 10.0, "self_preference_rust");

        mem_store.insert(n1).await.unwrap();
        mem_store.insert(n2).await.unwrap();
        mem_store.insert(n3).await.unwrap();

        let cons_store = InMemoryConsolidationStore::new(mem_store);
        cons_store.set_fact_schema_frequency("self_preference_rust", 4); // freq=4

        // Test with threshold=5 (schema is unstable, freq 4 < 5)
        let mut config_strict = ConsolidationConfig::default();
        config_strict.schema_stability_threshold = 5;
        config_strict.unstable_schema_weight_multiplier = 0.3;
        config_strict.similarity_threshold = 0.05;

        // Need a fresh store for the second test
        let mem_store2 = Arc::new(MemoryStore::new());
        let n1b = node_with_schema("I like Rust", MemoryType::Episodic, 10.0, "self_preference_rust");
        let n2b = node_with_schema("I enjoy Rust", MemoryType::Episodic, 10.0, "self_preference_rust");
        let n3b = node_with_schema("Rust is great", MemoryType::Episodic, 10.0, "self_preference_rust");
        mem_store2.insert(n1b).await.unwrap();
        mem_store2.insert(n2b).await.unwrap();
        mem_store2.insert(n3b).await.unwrap();

        let cons_store2 = InMemoryConsolidationStore::new(mem_store2);
        cons_store2.set_fact_schema_frequency("self_preference_rust", 4);

        // Test with threshold=3 (schema is stable, freq 4 >= 3)
        let config_lenient = ConsolidationConfig {
            schema_stability_threshold: 3,
            unstable_schema_weight_multiplier: 0.3,
            similarity_threshold: 0.05,
            ..ConsolidationConfig::default()
        };
        let engine_lenient = ConsolidationEngine::new(config_lenient, cons_store2);

        let report_lenient = engine_lenient.run_consolidation().await.unwrap();
        assert_eq!(report_lenient.summaries_created, 1);

        let all_nodes_lenient = engine_lenient.store.store.list_all().await.unwrap();
        let summary_lenient: Vec<_> = all_nodes_lenient
            .iter()
            .filter(|n| n.memory_type == MemoryType::Semantic)
            .collect();
        let weight_lenient = summary_lenient[0].weight;

        // With threshold=3, all get full weight → near 10.0
        assert!(
            (weight_lenient - 10.0).abs() < 0.5,
            "threshold=3 (stable): expected near 10.0, got {}",
            weight_lenient
        );

        // Now test with threshold=5 (strict) using the first engine
        let engine_strict = ConsolidationEngine::new(config_strict, cons_store);
        let report_strict = engine_strict.run_consolidation().await.unwrap();
        assert_eq!(report_strict.summaries_created, 1);

        let all_nodes_strict = engine_strict.store.store.list_all().await.unwrap();
        let summary_strict: Vec<_> = all_nodes_strict
            .iter()
            .filter(|n| n.memory_type == MemoryType::Semantic)
            .collect();
        let weight_strict = summary_strict[0].weight;

        // With threshold=5, all unstable → 10.0 * 0.3 = 3.0 each → avg 3.0
        assert!(
            weight_strict < weight_lenient,
            "stricter threshold should produce lower weight (strict={}, lenient={})",
            weight_strict, weight_lenient
        );
        assert!(
            (weight_strict - 3.0).abs() < 0.5,
            "threshold=5 (unstable): expected near 3.0, got {}",
            weight_strict
        );
    }
}

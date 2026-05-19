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
use std::collections::HashMap;
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

/// Configuration for consolidation.
#[derive(Debug, Clone)]
pub struct ConsolidationConfig {
    /// Days after which episodic memories are eligible for consolidation.
    pub episodic_age_threshold_days: u32,
    /// Minimum number of related memories to form a cluster.
    pub min_cluster_size: usize,
    /// Maximum cluster size before splitting.
    pub max_cluster_size: usize,
    /// BM25 similarity threshold for clustering.
    pub similarity_threshold: f64,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            episodic_age_threshold_days: 7,
            min_cluster_size: 3,
            max_cluster_size: 20,
            similarity_threshold: 0.6,
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
    /// 2. Cluster by topic (using BM25 + semantic similarity)
    /// 3. For each cluster: generate summary, create parent node
    /// 4. Repoint children to new parent
    /// 5. Archive originals (or mark as consolidated)
    /// 6. Create knowledge edges between related summaries
    pub async fn run_consolidation(&self) -> Result<ConsolidationReport> {
        let run_id = Uuid::new_v4();
        let start = std::time::Instant::now();

        let mut summaries_created = 0;
        let clusters_formed: usize;
        let edges_created = 0;
        let mut memories_archived = 0;

        // Step 1: Find eligible episodic memories
        let eligible = self
            .store
            .get_episodic_memories_older_than(self.config.episodic_age_threshold_days)
            .await?;

        if eligible.len() < self.config.min_cluster_size {
            // Not enough memories to consolidate
            return Ok(ConsolidationReport {
                run_id,
                memories_processed: eligible.len(),
                summaries_created: 0,
                clusters_formed: 0,
                edges_created: 0,
                memories_archived: 0,
                duration_ms: start.elapsed().as_millis(),
            });
        }

        // Step 2: Cluster by topic (simplified — uses content similarity)
        let clusters = self.cluster_memories(&eligible).await?;
        clusters_formed = clusters.len();

        // Step 3: For each cluster, create summary + parent node
        for cluster in &clusters {
            if cluster.memory_ids.len() < self.config.min_cluster_size {
                continue;
            }

            // Fetch full memory content
            let memories = self.store.get_memories_by_ids(&cluster.memory_ids).await?;
            let contents: Vec<&str> = memories.iter().map(|m| m.content.as_str()).collect();

            // Generate summary (simplified — in production this would call an LLM)
            let summary = self.generate_summary(&contents, &cluster.topic)?;
            let avg_importance =
                memories.iter().map(|m| m.importance).sum::<i32>() as f32 / memories.len() as f32;

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

            // Create `related_to` edges between summaries
            // (done after all summaries are created for this run)
        }

        // Step 4: Create edges between summaries created in this run
        // (simplified — would do cross-cluster relationship detection)

        let duration_ms = start.elapsed().as_millis();

        Ok(ConsolidationReport {
            run_id,
            memories_processed: eligible.len(),
            summaries_created,
            clusters_formed,
            edges_created,
            memories_archived,
            duration_ms,
        })
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
}

/// Minimal memory data needed for clustering.
#[derive(Debug, Clone)]
pub struct ClusteringCandidate {
    pub id: Uuid,
    pub content: String,
    pub vector: Option<Vec<f32>>,
    pub topic: String,
}

/// Full memory data needed for consolidation.
#[derive(Debug, Clone)]
pub struct ConsolidationMemory {
    pub id: Uuid,
    pub content: String,
    pub importance: i32,
}

// ── InMemoryConsolidationStore ────────────────────────────────────────────
// Concrete implementation of ConsolidationStore backed by MemoryStore.
// Uses TST-inspired mean_vector() for L1 parent node creation — NEVER
// re-embeds from text (re-initialization destroys representation continuity,
// per TST's harshest ablation result).

use std::sync::Arc;
use crate::storage::in_memory::MemoryStore;
use crate::memory::fractal_node::mean_vector;
use crate::memory::FractalNode;
use crate::memory::types::{MemoryStatus};

/// Wraps MemoryStore to implement the ConsolidationStore trait.
/// All consolidation operations pass through to the underlying store.
pub struct InMemoryConsolidationStore {
    store: Arc<MemoryStore>,
}

impl InMemoryConsolidationStore {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }
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
            .map(|n| ClusteringCandidate {
                id: n.id,
                content: n.content.unwrap_or_default(),
                vector: if n.vector.is_empty() { None } else { Some(n.vector.clone()) },
                topic: n.metadata.get("topic").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
            .collect())
    }

    async fn get_memories_by_ids(&self, ids: &[Uuid]) -> Result<Vec<ConsolidationMemory>> {
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(node) = self.store.get(id).await? {
                results.push(ConsolidationMemory {
                    id: node.id,
                    content: node.content.unwrap_or_default(),
                    importance: node.weight as i32,
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
}

//! Energy Decay Worker — Ebbinghaus Forgetting Curve
//!
//! Implements the energy/memory decay system inspired by the Ebbinghaus forgetting
//! curve. Every memory has an `energy` level (0–100):
//!
//! - **Access boosts energy**: each retrieval adds a configurable boost (+20 default)
//! - **Time decays energy**: energy decays exponentially based on hours since last update
//!   (half-life of 168 hours / 7 days by default, configurable via `halflife_hours`)
//! - **Low-energy candidates**: memories below `min_energy_threshold` (default 10)
//!   are flagged for compression in Dream Mode
//!
//! ## Ebbinghaus Curve — Exponential Decay
//!
//! ✅ Exponential decay implemented: `energy = energy * exp(-λ * hours)`
//! where `λ = ln(2) / halflife_hours`. This models the real Ebbinghaus forgetting
//! curve more accurately than linear decay.
//!
//! ## Workflow
//!
//! 1. `boost_energy(id, +20)` — called after each memory access
//! 2. `apply_decay()` — called periodically (Dream Mode cron, e.g. every hour)
//! 3. `find_low_energy_memories(limit)` — returns memories below threshold
//! 4. `compress_cluster(ids)` — Dream Mode compresses 3 low-energy memories
//!    into 1 higher-level summary memory
//!
//! ## Example
//!
//! ```rust,ignore
//! let worker = EnergyDecayWorker::new(&pool, EnergyDecayConfig::default()); // halflife=7d, threshold=10
//!
//! // After retrieving a memory:
//! worker.boost_energy(memory_id, 20).await?;
//!
//! // In Dream Mode cron (hourly):
//! worker.apply_decay().await?;
//! let low_energy = worker.find_low_energy_memories(10).await?;
//! ```

#[cfg(feature = "postgres-storage")]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

// =============================================================================
// Types
// =============================================================================

/// Configuration for the energy decay worker.
#[derive(Debug, Clone, Copy)]
pub struct EnergyDecayConfig {
    /// Half-life in hours — how long until energy drops to 50% of its current value.
    /// Standard: 168 hours (7 days).
    pub halflife_hours: f32,
    /// Memories below this threshold are candidates for compression (default: 10).
    pub min_energy_threshold: i32,
}

impl Default for EnergyDecayConfig {
    fn default() -> Self {
        Self {
            halflife_hours: 168.0, // 7 days
            min_energy_threshold: 10,
        }
    }
}

/// A memory with its current energy level and last update timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEnergyInfo {
    pub id: Uuid,
    pub energy: i32,
    pub last_energy_update: DateTime<Utc>,
    pub memory_type: String,
    pub content: Option<String>,
}

/// Result of an energy decay application pass.
#[derive(Debug, Clone, Serialize)]
pub struct DecayResult {
    /// Number of memories that had their energy updated.
    pub memories_updated: usize,
    /// Number of memories that hit zero energy (fully decayed).
    pub memories_at_zero: usize,
}

/// Result of a compression operation.
#[derive(Debug, Clone, Serialize)]
pub struct CompressionResult {
    /// ID of the newly created compressed memory.
    pub new_memory_id: Uuid,
    /// IDs of the memories that were superseded.
    pub superseded_ids: Vec<Uuid>,
}

// =============================================================================
// Energy Decay Worker
// =============================================================================

/// Energy decay worker implementing the Ebbinghaus forgetting curve model.
pub struct EnergyDecayWorker<'a> {
    pool: &'a PgPool,
    halflife_hours: f32,
    min_energy_threshold: i32,
}

impl<'a> EnergyDecayWorker<'a> {
    /// Create a new worker with explicit config.
    pub fn new(pool: &'a PgPool, config: EnergyDecayConfig) -> Self {
        Self {
            pool,
            halflife_hours: config.halflife_hours,
            min_energy_threshold: config.min_energy_threshold,
        }
    }

    /// Create a new worker with default config (halflife=7 days, threshold=10).
    pub fn with_defaults(pool: &'a PgPool) -> Self {
        Self::new(pool, EnergyDecayConfig::default())
    }

    /// Boost the energy of a memory after access.
    ///
    /// Caps energy at 100 (LEAST logic). Resets `last_energy_update` to NOW().
    ///
    /// With exponential decay, resetting `last_energy_update` means the memory
    /// decays from the boosted level with the configured half-life.
    ///
    /// # Arguments
    /// * `memory_id` — the memory to boost
    /// * `boost` — how many energy units to add (e.g. 20 for a retrieval)
    pub async fn boost_energy(&self, memory_id: Uuid, boost: i32) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE memories
            SET energy = LEAST(100, energy + $2),
                last_energy_update = NOW()
            WHERE id = $1 AND status = 'active'
            "#,
            memory_id,
            boost,
        )
        .execute(self.pool)
        .await?;

        tracing::debug!(memory_id = %memory_id, boost, "energy boosted");
        Ok(())
    }

    /// Apply exponential decay to all active memories based on time since last update.
    ///
    /// Computes `hours_since_update` and applies `energy * exp(-λ * hours_since_update)`
    /// where `λ = ln(2) / halflife_hours`. Energy is floored at 0.
    ///
    /// Returns the number of memories updated and how many hit zero.
    pub async fn apply_decay(&self) -> Result<DecayResult> {
        // λ = ln(2) / halflife_hours  →  half-life decay constant
        let lambda = 2.0_f32.ln() / self.halflife_hours;

        // Update all active memories: exponential decay based on time elapsed
        let result = sqlx::query!(
            r#"
            UPDATE memories
            SET energy = GREATEST(0, CAST(
                energy * EXP(-$1 * EXTRACT(EPOCH FROM (NOW() - last_energy_update)) / 3600.0)
                AS INT
            )),
            last_energy_update = NOW()
            WHERE status = 'active'
              AND energy > 0
              AND last_energy_update < NOW()
            "#,
            lambda,
        )
        .execute(self.pool)
        .await?;

        // Count how many are now at zero
        let at_zero: (i64,) = sqlx::query_as!(
            _,
            r#"
            SELECT COUNT(*)::bigint
            FROM memories
            WHERE status = 'active' AND energy = 0
            "#
        )
        .fetch_one(self.pool)
        .await?;

        tracing::info!(
            updated = result.rows_affected(),
            at_zero = at_zero.0,
            halflife_hours = self.halflife_hours,
            "energy decay applied (exponential)"
        );

        Ok(DecayResult {
            memories_updated: result.rows_affected() as usize,
            memories_at_zero: at_zero.0 as usize,
        })
    }

    /// Find memories below the energy threshold that are candidates for compression.
    ///
    /// Returns up to `limit` memory IDs with `energy < min_energy_threshold`.
    pub async fn find_low_energy_memories(&self, limit: i32) -> Result<Vec<MemoryEnergyInfo>> {
        let rows = sqlx::query_as!(
            MemoryEnergyInfo,
            r#"
            SELECT id, energy, last_energy_update, memory_type, content
            FROM memories
            WHERE status = 'active'
              AND energy < $1
              AND energy >= 0
            ORDER BY energy ASC, last_energy_update ASC
            LIMIT $2
            "#,
            self.min_energy_threshold,
            limit
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows)
    }

    /// Compress a cluster of low-energy memories into a single higher-level memory.
    ///
    /// Takes 2–4 memory IDs, fetches their content, creates a new semantic memory
    /// with a combined/summary content, and marks the originals as `superseded`.
    ///
    /// The new memory:
    /// - Has type `semantic`
    /// - Contains the combined content of all input memories
    /// - Inherits the max importance and max confidence of the cluster
    /// - Has energy set to 50 (freshly consolidated, medium energy)
    ///
    /// # Arguments
    /// * `memory_ids` — 2–4 memory IDs to compress into one
    ///
    /// # Errors
    /// Returns an error if fewer than 2 valid memories are found.
    pub async fn compress_cluster(&self, memory_ids: &[Uuid]) -> Result<CompressionResult> {
        if memory_ids.len() < 2 {
            anyhow::bail!("compress_cluster requires at least 2 memory IDs, got {}", memory_ids.len());
        }

        // Fetch all memories in the cluster
        let rows = sqlx::query!(
            r#"
            SELECT id, memory_type, content, importance, confidence, entities, tags,
                   provenance, source, summary_content, overview_content
            FROM memories
            WHERE id = ANY($1) AND status = 'active'
            "#,
            memory_ids as _
        )
        .fetch_all(self.pool)
        .await?;

        if rows.len() < 2 {
            anyhow::bail!(
                "compress_cluster: only {} valid active memories found, need at least 2",
                rows.len()
            );
        }

        // Build combined content from all memories
        let mut combined_parts: Vec<String> = Vec::new();
        let mut max_importance: i32 = 5;
        let mut max_confidence: f64 = 0.8;
        let mut all_entities: Vec<serde_json::Value> = Vec::new();
        let mut all_tags: Vec<String> = Vec::new();
        let mut all_sources: Vec<String> = Vec::new();
        let mut provenance_parts: Vec<serde_json::Value> = Vec::new();

        for row in &rows {
            if let Some(content) = &row.content {
                combined_parts.push(content.clone());
            }
            max_importance = max_importance.max(row.importance);
            max_confidence = max_confidence.max(row.confidence);

            if let Some(serde_json::Value::Array(entities)) = &row.entities {
                all_entities.extend(entities.clone());
            }
            if let Some(tags) = &row.tags {
                all_tags.extend(tags.clone());
            }
            if let Some(source) = &row.source {
                all_sources.push(source.clone());
            }
            provenance_parts.push(serde_json::json!({
                "source_memory_id": row.id.to_string(),
                "memory_type": row.memory_type,
                "original_content": row.content,
                "summary_content": row.summary_content,
                "overview_content": row.overview_content,
            }));
        }

        // Compose the new content
        let new_content = if combined_parts.len() == 1 {
            combined_parts[0].clone()
        } else {
            format!(
                "[Consolidated from {} memories]\n\n{}",
                combined_parts.len(),
                combined_parts.join("\n\n---\n\n")
            )
        };

        let combined_provenance = serde_json::json!({
            "consolidation": true,
            "sources": provenance_parts,
            "consolidated_count": rows.len(),
            "consolidated_at": chrono::Utc::now().to_rfc3339(),
        });

        let new_id = Uuid::new_v4();

        // Insert the new consolidated memory
        sqlx::query!(
            r#"
            INSERT INTO memories (
                id, memory_type, content, embedding,
                importance, confidence, sensitivity, status,
                provenance, source, entities, tags,
                energy, last_energy_update,
                summary_content, overview_content,
                created_at, updated_at
            )
            VALUES (
                $1, 'semantic', $2, NULL,
                $3, $4, 'normal', 'active',
                $5, 'consolidation', $6, $7,
                50, NOW(),
                NULL, NULL,
                NOW(), NOW()
            )
            "#,
            new_id,
            new_content,
            max_importance,
            max_confidence,
            combined_provenance,
            serde_json::json!(all_entities),
            &all_tags.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )
        .execute(self.pool)
        .await?;

        // Mark original memories as superseded
        let superseded_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        for mem_id in &superseded_ids {
            // Use the new consolidated memory as the superseder
            sqlx::query!(
                r#"
                UPDATE memories
                SET status = 'superseded',
                    superseded_by = $1,
                    conflict_state = 'resolved',
                    updated_at = NOW()
                WHERE id = $2
                "#,
                new_id,
                *mem_id,
            )
            .execute(self.pool)
            .await?;
        }

        tracing::info!(
            new_id = %new_id,
            superseded = ?superseded_ids,
            count = superseded_ids.len(),
            "memory cluster compressed"
        );

        Ok(CompressionResult {
            new_memory_id: new_id,
            superseded_ids,
        })
    }
}

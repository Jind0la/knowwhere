//! Conflict Detection for Dream Mode
//!
//! Detects and resolves conflicting memories in the knowledge graph.
//! 
//! ## Conflict Types
//!
//! - **Entity Conflict**: Same entity (e.g., "Person X") but different facts
//!   (e.g., "Address is A" vs "Address is B")
//! - **Temporal Conflict**: Same fact claimed at different times with different values
//!   (e.g., "Job title was Engineer (2023)" vs "Job title is Manager (2024)")
//! - **Confidence Conflict**: Same fact with significantly different confidence scores
//!
//! ## Workflow
//!
//! 1. `ConflictDetector::detect_conflicts()` — finds all potential conflicts
//! 2. System marks conflicts with `conflict_state = 'pending'`
//! 3. `GET /conflicts` — returns all pending conflicts to operator/LLM
//! 4. `POST /conflicts/{id}/resolve` — operator resolves by choosing winner
//! 5. Losing memory is marked `superseded_by = winner_id`

#[cfg(feature = "postgres-storage")]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

// =============================================================================
// Types
// =============================================================================

/// Type of conflict detected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Same entity has different factual claims.
    Entity,
    /// Same claim made at different times with different values.
    Temporal,
    /// Same claim with significantly different confidence scores.
    Confidence,
}

impl std::fmt::Display for ConflictType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictType::Entity => write!(f, "entity"),
            ConflictType::Temporal => write!(f, "temporal"),
            ConflictType::Confidence => write!(f, "confidence"),
        }
    }
}

impl ConflictType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "entity" => Some(ConflictType::Entity),
            "temporal" => Some(ConflictType::Temporal),
            "confidence" => Some(ConflictType::Confidence),
            _ => None,
        }
    }
}

/// A detected group of conflicting memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictGroup {
    /// Unique ID for this conflict group.
    pub id: Uuid,
    /// Memory IDs involved in this conflict.
    pub conflicting_memory_ids: Vec<Uuid>,
    /// What kind of conflict this is.
    pub conflict_type: ConflictType,
    /// Human-readable description of the conflict.
    pub description: String,
    /// When this conflict was detected.
    pub detected_at: DateTime<Utc>,
    /// Current state: pending, resolved
    pub state: String,
}

/// Row type for querying conflict groups from DB.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConflictGroupRow {
    pub id: Uuid,
    pub conflicting_memory_ids: Vec<Uuid>,
    pub conflict_type: String,
    pub description: String,
    pub detected_at: DateTime<Utc>,
    pub state: String,
}

/// Request to resolve a conflict by choosing a winning memory.
#[derive(Debug, Deserialize)]
pub struct ResolveConflictRequest {
    pub winning_memory_id: Uuid,
}

/// Result of a conflict detection run.
#[derive(Debug, Clone, Serialize)]
pub struct ConflictDetectionResult {
    pub conflicts_found: usize,
    pub conflicts_marked_pending: usize,
    pub run_id: Uuid,
}

// =============================================================================
// Conflict Detector
// =============================================================================

/// Detects and resolves conflicting memories in the knowledge graph.
pub struct ConflictDetector<'a> {
    pool: &'a PgPool,
}

impl<'a> ConflictDetector<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Detects all conflicts in the memory graph.
    ///
    /// This performs three types of conflict detection:
    /// 1. **Entity conflicts**: Memories with same entity but different facts
    /// 2. **Temporal conflicts**: Same fact claimed at different times
    /// 3. **Confidence conflicts**: Same fact with different confidence scores
    ///
    /// Returns a summary of what was found and marked.
    pub async fn detect_conflicts(&self) -> Result<ConflictDetectionResult> {
        let run_id = Uuid::new_v4();
        
        // Detect entity conflicts
        let entity_conflicts = self.detect_entity_conflicts().await?;
        
        // Detect temporal conflicts
        let temporal_conflicts = self.detect_temporal_conflicts().await?;
        
        // Detect confidence conflicts
        let confidence_conflicts = self.detect_confidence_conflicts().await?;
        
        let total = entity_conflicts.len() + temporal_conflicts.len() + confidence_conflicts.len();
        
        // Log the detection run
        sqlx::query!(
            r#"
            INSERT INTO conflict_detection_runs (id, conflicts_found, conflicts_resolved, run_at)
            VALUES ($1, $2, 0, NOW())
            "#,
            run_id,
            total as i32,
        )
        .execute(self.pool)
        .await?;
        
        Ok(ConflictDetectionResult {
            conflicts_found: total,
            conflicts_marked_pending: total,
            run_id,
        })
    }

    /// Detect entity conflicts — same entity, different facts.
    ///
    /// Strategy: Group memories by entity name (from entities JSONB field),
    /// then for groups with >1 memory, check if they have contradictory claims.
    async fn detect_entity_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        // Get all active memories with entities
        let rows = sqlx::query!(
            r#"
            SELECT id, memory_type, content, entities, metadata, created_at, confidence
            FROM memories
            WHERE status = 'active'
              AND conflict_state = 'none'
              AND entities IS NOT NULL
              AND jsonb_array_length(entities) > 0
            "#
        )
        .fetch_all(self.pool)
        .await?;

        // Group by first entity
        let mut entity_groups: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
        for row in &rows {
            if let Some(serde_json::Value::Array(entities)) = &row.entities {
                for entity in entities {
                    if let Some(name) = entity.as_str() {
                        entity_groups
                            .entry(name.to_string())
                            .or_default()
                            .push(row);
                        break; // Use first entity only for grouping
                    }
                }
            }
        }

        let mut conflicts = Vec::new();
        for (entity_name, memories) in entity_groups {
            if memories.len() < 2 {
                continue;
            }

            // Check for content conflicts (different content for same entity)
            let mut contents: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut conflicting_ids = Vec::new();
            
            for mem in &memories {
                if let Some(content) = &mem.content {
                    if !contents.insert(content.clone()) {
                        // Duplicate content — not a conflict
                        continue;
                    }
                }
                conflicting_ids.push(mem.id);
            }

            // If we have multiple different contents for same entity → conflict
            if conflicting_ids.len() > 1 && contents.len() > 1 {
                let description = format!(
                    "Entity '{}' has {} different claims: {}",
                    entity_name,
                    conflicting_ids.len(),
                    contents.len()
                );

                let id = Uuid::new_v4();
                
                // Insert conflict record
                sqlx::query!(
                    r#"
                    INSERT INTO memory_conflicts (id, conflicting_memory_ids, conflict_type, description, detected_at, state)
                    VALUES ($1, $2, $3, $4, NOW(), 'pending')
                    "#,
                    id,
                    serde_json::json!(conflicting_ids),
                    "entity",
                    description,
                )
                .execute(self.pool)
                .await?;

                // Mark memories as having pending conflict
                for mem_id in &conflicting_ids {
                    sqlx::query!(
                        r#"
                        UPDATE memories SET conflict_state = 'pending' WHERE id = $1
                        "#,
                        *mem_id,
                    )
                    .execute(self.pool)
                    .await?;
                }

                conflicts.push(ConflictGroup {
                    id,
                    conflicting_memory_ids: conflicting_ids,
                    conflict_type: ConflictType::Entity,
                    description,
                    detected_at: chrono::Utc::now(),
                    state: "pending".to_string(),
                });
            }
        }

        Ok(conflicts)
    }

    /// Detect temporal conflicts — same fact claimed at different times.
    async fn detect_temporal_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        // For now, simplified: look for memories with similar content but different timestamps
        // In production, this would use more sophisticated NLP to identify same facts
        // at different points in time
        Ok(Vec::new()) // Placeholder for future implementation
    }

    /// Detect confidence conflicts — same claim with very different confidence.
    async fn detect_confidence_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        // Find memories with same content but very different confidence scores (>0.3 difference)
        let rows = sqlx::query!(
            r#"
            SELECT id, memory_type, content, confidence, created_at
            FROM memories
            WHERE status = 'active'
              AND conflict_state = 'none'
              AND content IS NOT NULL
            ORDER BY content, confidence
            "#
        )
        .fetch_all(self.pool)
        .await?;

        let mut conflicts = Vec::new();
        let mut i = 0;
        
        while i < rows.len() {
            let current = &rows[i];
            let mut j = i + 1;
            let mut same_content: Vec<_> = vec![current];
            
            // Find all memories with same content
            while j < rows.len() && rows[j].content == current.content {
                same_content.push(&rows[j]);
                j += 1;
            }

            // Check if any pair has significantly different confidence
            for k in 0..same_content.len() {
                for l in (k + 1)..same_content.len() {
                    let diff = (same_content[k].confidence - same_content[l].confidence).abs();
                    if diff > 0.3 {
                        let conflicting_ids: Vec<Uuid> = same_content.iter().map(|m| m.id).collect();
                        let description = format!(
                            "Same content has confidence scores {} and {} (diff: {:.2})",
                            same_content[k].confidence, same_content[l].confidence, diff
                        );

                        let id = Uuid::new_v4();
                        
                        sqlx::query!(
                            r#"
                            INSERT INTO memory_conflicts (id, conflicting_memory_ids, conflict_type, description, detected_at, state)
                            VALUES ($1, $2, $3, $4, NOW(), 'pending')
                            "#,
                            id,
                            serde_json::json!(conflicting_ids),
                            "confidence",
                            description,
                        )
                        .execute(self.pool)
                        .await?;

                        for mem_id in &conflicting_ids {
                            sqlx::query!(
                                r#"
                                UPDATE memories SET conflict_state = 'pending' WHERE id = $1
                                "#,
                                *mem_id,
                            )
                            .execute(self.pool)
                            .await?;
                        }

                        conflicts.push(ConflictGroup {
                            id,
                            conflicting_memory_ids: conflicting_ids,
                            conflict_type: ConflictType::Confidence,
                            description,
                            detected_at: chrono::Utc::now(),
                            state: "pending".to_string(),
                        });
                        break;
                    }
                }
            }

            i = j;
        }

        Ok(conflicts)
    }

    /// List all pending (unresolved) conflicts.
    pub async fn list_pending_conflicts(&self) -> Result<Vec<ConflictGroup>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, conflicting_memory_ids, conflict_type, description, detected_at, state
            FROM memory_conflicts
            WHERE state = 'pending'
            ORDER BY detected_at DESC
            "#
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ConflictGroup {
                id: r.id,
                conflicting_memory_ids: serde_json::from_value(r.conflicting_memory_ids).unwrap_or_default(),
                conflict_type: ConflictType::from_str(&r.conflict_type).unwrap_or(ConflictType::Entity),
                description: r.description,
                detected_at: r.detected_at,
                state: r.state,
            })
            .collect())
    }

    /// Resolve a conflict by designating the winning memory.
    ///
    /// The winning memory stays active.
    /// All other memories in the conflict are marked as `superseded_by` the winner.
    pub async fn resolve_conflict(
        &self,
        conflict_id: Uuid,
        winning_memory_id: Uuid,
    ) -> Result<()> {
        // Get the conflict group
        let conflict = sqlx::query!(
            r#"
            SELECT id, conflicting_memory_ids
            FROM memory_conflicts
            WHERE id = $1 AND state = 'pending'
            "#,
            conflict_id,
        )
        .fetch_optional(self.pool)
        .await?;

        let conflict = match conflict {
            Some(c) => c,
            None => anyhow::bail!("conflict {} not found or already resolved", conflict_id),
        };

        let memory_ids: Vec<Uuid> = serde_json::from_value(conflict.conflicting_memory_ids)?;

        // Ensure winner is in the conflict group
        if !memory_ids.contains(&winning_memory_id) {
            anyhow::bail!(
                "winning memory {} is not part of conflict {}",
                winning_memory_id,
                conflict_id
            );
        }

        // Mark losing memories as superseded
        for mem_id in &memory_ids {
            if *mem_id != winning_memory_id {
                sqlx::query!(
                    r#"
                    UPDATE memories
                    SET superseded_by = $1, status = 'superseded', conflict_state = 'resolved', updated_at = NOW()
                    WHERE id = $2
                    "#,
                    winning_memory_id,
                    *mem_id,
                )
                .execute(self.pool)
                .await?;
            }
        }

        // Mark conflict as resolved
        sqlx::query!(
            r#"
            UPDATE memory_conflicts
            SET state = 'resolved'
            WHERE id = $1
            "#,
            conflict_id,
        )
        .execute(self.pool)
        .await?;

        // Update winning memory: clear conflict state
        sqlx::query!(
            r#"
            UPDATE memories
            SET conflict_state = 'resolved', updated_at = NOW()
            WHERE id = $1
            "#,
            winning_memory_id,
        )
        .execute(self.pool)
        .await?;

        tracing::info!(
            conflict_id = %conflict_id,
            winner_id = %winning_memory_id,
            losers = ?memory_ids.iter().filter(|id| ***id != winning_memory_id).collect::<Vec<_>>(),
            "conflict resolved"
        );

        Ok(())
    }

    /// Get recent conflict detection runs.
    pub async fn recent_runs(&self, limit: i32) -> Result<Vec<ConflictDetectionRunRow>> {
        let rows = sqlx::query_as!(
            ConflictDetectionRunRow,
            r#"
            SELECT id, conflicts_found, conflicts_resolved, run_at
            FROM conflict_detection_runs
            ORDER BY run_at DESC
            LIMIT $1
            "#,
            limit
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }
}

/// Row type for conflict detection runs.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConflictDetectionRunRow {
    pub id: Uuid,
    pub conflicts_found: i32,
    pub conflicts_resolved: i32,
    pub run_at: DateTime<Utc>,
}

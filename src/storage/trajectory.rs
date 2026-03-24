//! Trajectory storage for retrieval tracking.
//!
//! Tracks how memories are accessed, used, and how their relevance decays over time.
//! Provides both in-memory step accumulation (for fractal zoom) and PostgreSQL
//! persistence (for audit and analytics).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "postgres-storage")]
use sqlx::PgPool;

/// Step type labels used in [`RetrievalStep`].
pub mod step_type {
    /// Node was visited during fractal zoom.
    pub const VISITED: &str = "visited";
    /// Traversed from parent to child during zoom.
    pub const DESCENDED: &str = "descended";
    /// A branch was pruned (similarity below threshold or no children).
    pub const PRUNED: &str = "pruned";
    /// Informational step (fallback triggered, summary, etc.).
    pub const INFO: &str = "info";
    /// Initial search step.
    pub const INITIAL: &str = "initial";
    /// Governance filter applied.
    pub const FILTERED: &str = "filtered";
    /// Final result in top-k.
    pub const RESULT: &str = "result";
}

// ---------------------------------------------------------------------------
// RetrievalStep — in-memory log entry emitted during fractal zoom
// ---------------------------------------------------------------------------

/// A single log step emitted during fractal zoom retrieval.
///
/// Each step records what happened at a particular node (visited,
/// descended into, or pruned) along with the similarity score and
/// optional metadata. These steps are accumulated in a `Vec<RetrievalStep>`
/// and can be persisted or inspected after retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalStep {
    /// Unique identifier of the memory/node this step refers to.
    pub memory_id: Uuid,
    /// Cosine similarity of this node to the query at the time of the step.
    pub similarity: f32,
    /// Step type label (`visited`, `descended`, `pruned`, `info`).
    pub step_type: String,
    /// Remaining depth when this node was visited (for `visited` steps).
    pub remaining_depth: Option<usize>,
    /// Parent node ID (for `descended` steps).
    pub parent_id: Option<Uuid>,
    /// Human-readable reason for pruning or info (for `pruned` / `info` steps).
    pub filter_reason: Option<String>,
    /// Timestamp when this step was recorded.
    pub recorded_at: DateTime<Utc>,
}

impl RetrievalStep {
    /// Records a node visit during fractal zoom.
    pub fn visited(memory_id: Uuid, similarity: f32, remaining_depth: usize) -> Self {
        Self {
            memory_id,
            similarity,
            step_type: step_type::VISITED.to_string(),
            remaining_depth: Some(remaining_depth),
            parent_id: None,
            filter_reason: None,
            recorded_at: Utc::now(),
        }
    }

    /// Records a descent from a parent node into a child during zoom.
    pub fn descended(child_id: Uuid, child_similarity: f32, parent_id: Uuid) -> Self {
        Self {
            memory_id: child_id,
            similarity: child_similarity,
            step_type: step_type::DESCENDED.to_string(),
            remaining_depth: None,
            parent_id: Some(parent_id),
            filter_reason: None,
            recorded_at: Utc::now(),
        }
    }

    /// Records that a node's sub-tree was pruned.
    pub fn pruned(memory_id: Uuid, similarity: f32, reason: &str) -> Self {
        Self {
            memory_id,
            similarity,
            step_type: step_type::PRUNED.to_string(),
            remaining_depth: None,
            parent_id: None,
            filter_reason: Some(reason.to_string()),
            recorded_at: Utc::now(),
        }
    }

    /// Records an informational step (e.g., fallback triggered, summary note).
    pub fn info(memory_id: Uuid, message: impl Into<String>) -> Self {
        Self {
            memory_id,
            similarity: 0.0,
            step_type: step_type::INFO.to_string(),
            remaining_depth: None,
            parent_id: None,
            filter_reason: Some(message.into()),
            recorded_at: Utc::now(),
        }
    }

    /// Records an initial search step.
    pub fn initial(memory_id: Uuid, score: f32, decision: &str) -> Self {
        Self {
            memory_id,
            similarity: score,
            step_type: step_type::INITIAL.to_string(),
            remaining_depth: None,
            parent_id: None,
            filter_reason: Some(decision.to_string()),
            recorded_at: Utc::now(),
        }
    }

    /// Records a governance filter step.
    pub fn filtered(memory_id: Uuid, score: f32, reason: &str) -> Self {
        Self {
            memory_id,
            similarity: score,
            step_type: step_type::FILTERED.to_string(),
            remaining_depth: None,
            parent_id: None,
            filter_reason: Some(reason.to_string()),
            recorded_at: Utc::now(),
        }
    }

    /// Returns `true` if this step represents a pruned branch.
    pub fn is_pruned(&self) -> bool {
        self.step_type == step_type::PRUNED
    }
}

// ---------------------------------------------------------------------------
// RetrievalTrajectory — complete trajectory wrapper
// ---------------------------------------------------------------------------

/// A complete retrieval trajectory for one query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalTrajectory {
    /// Unique ID for this run (generated on insert).
    pub run_id: Uuid,
    /// The query text (if provided).
    pub query_text: String,
    /// The query embedding vector.
    pub query_embedding: Vec<f32>,
    /// All steps in the trajectory.
    pub steps: Vec<RetrievalStep>,
    /// Total candidates considered.
    pub total_candidates: usize,
    /// Final retrieved count.
    pub retrieved_count: usize,
    /// Total execution time in milliseconds.
    pub execution_time_ms: u64,
    /// Maximum fractal zoom depth used.
    pub max_depth_used: usize,
}

impl RetrievalTrajectory {
    /// Start a new trajectory (run_id will be assigned on log_retrieval).
    pub fn new(query_text: String, query_embedding: Vec<f32>) -> Self {
        Self {
            run_id: Uuid::nil(),
            query_text,
            query_embedding,
            steps: Vec::new(),
            total_candidates: 0,
            retrieved_count: 0,
            execution_time_ms: 0,
            max_depth_used: 0,
        }
    }

    /// Add a step with auto-incremented index metadata.
    pub fn add_step(&mut self, step: RetrievalStep) {
        self.steps.push(step);
    }

    /// Add an initial search step.
    pub fn log_search(&mut self, memory_id: Uuid, score: f32, decision: &str) {
        self.add_step(RetrievalStep::initial(memory_id, score, decision));
    }

    /// Add a fractal zoom step.
    pub fn log_zoom(&mut self, child_id: Uuid, child_similarity: f32, parent_id: Uuid) {
        self.add_step(RetrievalStep::descended(child_id, child_similarity, parent_id));
    }

    /// Add a pruned step.
    pub fn log_pruned(&mut self, memory_id: Uuid, similarity: f32, reason: &str) {
        self.add_step(RetrievalStep::pruned(memory_id, similarity, reason));
    }

    /// Add an info step.
    pub fn log_info(&mut self, message: impl Into<String>) {
        self.add_step(RetrievalStep::info(Uuid::nil(), message));
    }

    /// Add a filtered step.
    pub fn log_filtered(&mut self, memory_id: Uuid, score: f32, reason: &str) {
        self.add_step(RetrievalStep::filtered(memory_id, score, reason));
    }
}

// ---------------------------------------------------------------------------
// Database persistence (PostgreSQL)
// ---------------------------------------------------------------------------

/// Row type returned by GET /retrieval/runs
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RetrievalRunRow {
    pub id: Uuid,
    pub query_text: String,
    pub embedding: Option<Vec<f32>>,
    pub run_at: DateTime<Utc>,
    pub total_candidates: Option<i32>,
    pub retrieved_count: Option<i32>,
    pub execution_time_ms: Option<i32>,
    pub max_depth_used: Option<i32>,
    pub metadata: serde_json::Value,
}

/// Row type for trajectory steps
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrajectoryStepRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_index: i32,
    pub step_type: String,
    pub memory_id: Option<Uuid>,
    pub score_before: Option<f64>,
    pub score_after: Option<f64>,
    pub rank: Option<i32>,
    pub decision: Option<String>,
    pub filter_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Persistence operations for retrieval trajectories.
pub struct TrajectoryStore<'a> {
    pool: &'a PgPool,
}

impl<'a> TrajectoryStore<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Returns a reference to the underlying PostgreSQL connection pool.
    pub fn pool(&self) -> &'a PgPool {
        self.pool
    }

    /// Log a complete retrieval trajectory to PostgreSQL.
    ///
    /// Returns the run_id of the inserted retrieval_runs row.
    pub async fn log_retrieval(&self, trajectory: &RetrievalTrajectory) -> sqlx::Result<Uuid> {
        let run_id = Uuid::new_v4();

        // Insert the run row
        sqlx::query!(
            r#"
            INSERT INTO retrieval_runs (
                id, query_text, embedding, run_at,
                total_candidates, retrieved_count, execution_time_ms, max_depth_used, metadata
            )
            VALUES ($1, $2, $3, NOW(), $4, $5, $6, $7, $8)
            "#,
            run_id,
            &trajectory.query_text,
            trajectory.query_embedding.clone() as _,
            trajectory.total_candidates as i32,
            trajectory.retrieved_count as i32,
            trajectory.execution_time_ms as i32,
            trajectory.max_depth_used as i32,
            serde_json::json!({}),
        )
        .execute(self.pool)
        .await?;

        // Insert all steps
        for (i, step) in trajectory.steps.iter().enumerate() {
            sqlx::query!(
                r#"
                INSERT INTO retrieval_trajectory (
                    run_id, step_index, step_type, memory_id,
                    score_before, score_after, rank, decision, filter_reason
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
                run_id,
                i as i32,
                &step.step_type,
                step.memory_id,
                step.similarity as _,  // similarity maps to score_after
                step.similarity as _,  // we don't have score_before, use same
                step.remaining_depth.map(|d| d as i32),  // rank from remaining_depth if available
                step.filter_reason.clone(),
                step.filter_reason.clone(),
            )
            .execute(self.pool)
            .await?;
        }

        Ok(run_id)
    }

    /// List recent retrieval runs (cursor-based pagination).
    pub async fn list_runs(&self, limit: i32, after_id: Option<Uuid>) -> sqlx::Result<Vec<RetrievalRunRow>> {
        let rows = if let Some(after) = after_id {
            sqlx::query_as!(
                RetrievalRunRow,
                r#"
                SELECT id as "id!", query_text as "query_text!",
                       run_at as "run_at!",
                       embedding as "embedding: _",
                       total_candidates, retrieved_count, execution_time_ms, max_depth_used,
                       metadata
                FROM retrieval_runs
                WHERE id < $1
                ORDER BY run_at DESC
                LIMIT $2
                "#,
                after,
                limit
            )
            .fetch_all(self.pool)
            .await?
        } else {
            sqlx::query_as!(
                RetrievalRunRow,
                r#"
                SELECT id as "id!", query_text as "query_text!",
                       run_at as "run_at!",
                       embedding as "embedding: _",
                       total_candidates, retrieved_count, execution_time_ms, max_depth_used,
                       metadata
                FROM retrieval_runs
                ORDER BY run_at DESC
                LIMIT $1::bigint
                "#,
                limit as i64
            )
            .fetch_all(self.pool)
            .await?
        };
        Ok(rows)
    }

    /// Get a single retrieval run by ID.
    pub async fn get_run(&self, run_id: Uuid) -> sqlx::Result<Option<RetrievalRunRow>> {
        let row = sqlx::query_as!(
            RetrievalRunRow,
            r#"
            SELECT id as "id!", query_text as "query_text!",
                   run_at as "run_at!",
                   embedding as "embedding: _",
                   total_candidates, retrieved_count, execution_time_ms, max_depth_used,
                   metadata
            FROM retrieval_runs
            WHERE id = $1
            "#,
            run_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    /// Get all trajectory steps for a retrieval run.
    pub async fn get_trajectory(&self, run_id: Uuid) -> sqlx::Result<Vec<TrajectoryStepRow>> {
        let rows = sqlx::query_as!(
            TrajectoryStepRow,
            r#"
            SELECT id as "id!", run_id as "run_id!", step_index as "step_index!",
                   step_type as "step_type!",
                   memory_id, score_before, score_after, rank, decision, filter_reason,
                   created_at as "created_at!"
            FROM retrieval_trajectory
            WHERE run_id = $1
            ORDER BY step_index
            "#,
            run_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }
}

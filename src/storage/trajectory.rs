//! Retrieval Trajectory Logging
//!
//! Tracks the full decision path of each retrieval operation for:
//! - Observability: understand HOW context was found
//! - Debugging: trace retrieval failures
//! - Optimization: identify bottlenecks via RAGAs metrics
//!
//! Each retrieval produces a `RetrievalTrajectory` containing `RetrievalStep`s
//! that log every search attempt, filter, zoom, and rerank decision.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// -----------------------------------------------------------------------------
// Trajectory Types
// -----------------------------------------------------------------------------

/// A single step within a retrieval trajectory.
///
/// Records what happened at each stage of retrieval:
/// - `initial_search`: First vector/BM25 search
/// - `fractal_zoom`: Following parent-child relationships
/// - `bm25_search`: Full-text keyword search
/// - `governance_filter`: Stage 2 governance filtering
/// - `rerank`: Score adjustment
/// - `result`: Final included result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalStep {
    /// Zero-based step index within the trajectory.
    pub step_index: usize,
    /// What kind of step this was.
    pub step_type: String,
    /// Which memory was involved (None for aggregate/logging steps).
    pub memory_id: Option<Uuid>,
    /// Score before this step's transformation (e.g., before rerank).
    pub score_before: Option<f32>,
    /// Score after this step's transformation.
    pub score_after: Option<f32>,
    /// Final rank in results (1 = best).
    pub rank: Option<usize>,
    /// Human-readable explanation of the decision.
    pub decision: String,
    /// Why something was filtered out (if applicable).
    pub filter_reason: Option<String>,
}

impl RetrievalStep {
    /// Create an initial search step.
    pub fn initial_search(memory_id: Uuid, score: f32, decision: &str) -> Self {
        Self {
            step_index: 0,
            step_type: "initial_search".to_string(),
            memory_id: Some(memory_id),
            score_before: None,
            score_after: Some(score),
            rank: None,
            decision: decision.to_string(),
            filter_reason: None,
        }
    }

    /// Create a fractal zoom step.
    pub fn fractal_zoom(memory_id: Uuid, score_before: f32, score_after: f32, decision: &str) -> Self {
        Self {
            step_index: 0,
            step_type: "fractal_zoom".to_string(),
            memory_id: Some(memory_id),
            score_before: Some(score_before),
            score_after: Some(score_after),
            rank: None,
            decision: decision.to_string(),
            filter_reason: None,
        }
    }

    /// Create a filter step (node excluded).
    pub fn filtered(memory_id: Uuid, score_before: f32, reason: &str) -> Self {
        Self {
            step_index: 0,
            step_type: "governance_filter".to_string(),
            memory_id: Some(memory_id),
            score_before: Some(score_before),
            score_after: None,
            rank: None,
            decision: "filtered".to_string(),
            filter_reason: Some(reason.to_string()),
        }
    }

    /// Create a final result step.
    pub fn result(memory_id: Uuid, rank: usize, score: f32, decision: &str) -> Self {
        Self {
            step_index: 0,
            step_type: "result".to_string(),
            memory_id: Some(memory_id),
            score_before: Some(score),
            score_after: Some(score),
            rank: Some(rank),
            decision: decision.to_string(),
            filter_reason: None,
        }
    }
}

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

    /// Add a step with auto-incremented index.
    pub fn add_step(&mut self, mut step: RetrievalStep) {
        step.step_index = self.steps.len();
        self.steps.push(step);
    }

    /// Log an initial search.
    pub fn log_search(&mut self, memory_id: Uuid, score: f32, decision: &str) {
        self.add_step(RetrievalStep::initial_search(memory_id, score, decision));
    }

    /// Log a fractal zoom.
    pub fn log_zoom(&mut self, memory_id: Uuid, score_before: f32, score_after: f32, decision: &str) {
        self.add_step(RetrievalStep::fractal_zoom(memory_id, score_before, score_after, decision));
    }

    /// Log a filtered node.
    pub fn log_filtered(&mut self, memory_id: Uuid, score: f32, reason: &str) {
        self.add_step(RetrievalStep::filtered(memory_id, score, reason));
    }

    /// Log a final result.
    pub fn log_result(&mut self, memory_id: Uuid, rank: usize, score: f32, decision: &str) {
        self.add_step(RetrievalStep::result(memory_id, rank, score, decision));
    }
}

// -----------------------------------------------------------------------------
// Storage Operations
// -----------------------------------------------------------------------------

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

    /// Log a complete retrieval trajectory to PostgreSQL.
    ///
    /// Returns the run_id of the inserted retrieval_runs row.
    pub async fn log_retrieval(&self, trajectory: &RetrievalTrajectory) -> Result<Uuid> {
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
            trajectory.total_candidates as _,
            trajectory.retrieved_count as _,
            trajectory.execution_time_ms as i32,
            trajectory.max_depth_used as _,
            serde_json::json!({}),
        )
        .execute(self.pool)
        .await?;

        // Insert all steps
        for step in &trajectory.steps {
            sqlx::query!(
                r#"
                INSERT INTO retrieval_trajectory (
                    run_id, step_index, step_type, memory_id,
                    score_before, score_after, rank, decision, filter_reason
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
                run_id,
                step.step_index as i32,
                &step.step_type,
                step.memory_id,
                step.score_before as _,
                step.score_after as _,
                step.rank as _,
                &step.decision,
                step.filter_reason,
            )
            .execute(self.pool)
            .await?;
        }

        Ok(run_id)
    }

    /// List recent retrieval runs (cursor-based pagination).
    pub async fn list_runs(&self, limit: i32, after_id: Option<Uuid>) -> Result<Vec<RetrievalRunRow>> {
        let rows = if let Some(after) = after_id {
            sqlx::query_as!(
                RetrievalRunRow,
                r#"
                SELECT id, query_text, embedding, run_at,
                       total_candidates, retrieved_count, execution_time_ms, max_depth_used, metadata
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
                SELECT id, query_text, embedding, run_at,
                       total_candidates, retrieved_count, execution_time_ms, max_depth_used, metadata
                FROM retrieval_runs
                ORDER BY run_at DESC
                LIMIT $1
                "#,
                limit
            )
            .fetch_all(self.pool)
            .await?
        };
        Ok(rows)
    }

    /// Get a single retrieval run by ID.
    pub async fn get_run(&self, run_id: Uuid) -> Result<Option<RetrievalRunRow>> {
        let row = sqlx::query_as!(
            RetrievalRunRow,
            r#"
            SELECT id, query_text, embedding, run_at,
                   total_candidates, retrieved_count, execution_time_ms, max_depth_used, metadata
            FROM retrieval_runs
            WHERE id = $1
            "#,
            run_id
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    /// Get all trajectory steps for a run.
    pub async fn get_trajectory(&self, run_id: Uuid) -> Result<Vec<TrajectoryStepRow>> {
        let rows = sqlx::query_as!(
            TrajectoryStepRow,
            r#"
            SELECT id, run_id, step_index, step_type, memory_id,
                   score_before, score_after, rank, decision, filter_reason, created_at
            FROM retrieval_trajectory
            WHERE run_id = $1
            ORDER BY step_index ASC
            "#,
            run_id
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }
}

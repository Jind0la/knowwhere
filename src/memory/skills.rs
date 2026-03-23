//! Agent Skills Management — explicit registry of what the agent can do.
//!
//! Skills track:
//! - What capabilities the agent possesses
//! - How proficient it is (1–10 scale)
//! - When it was last used
//! - Which memories document or relate to that skill
//!
//! Skills are organized into categories: `language`, `tool`, `domain`, `framework`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::namespaces::MemoryNamespace;

/// Skill categories following a capability taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillCategory {
    Language,
    Tool,
    Domain,
    Framework,
}

impl SkillCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillCategory::Language => "language",
            SkillCategory::Tool => "tool",
            SkillCategory::Domain => "domain",
            SkillCategory::Framework => "framework",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "language" => Some(SkillCategory::Language),
            "tool" => Some(SkillCategory::Tool),
            "domain" => Some(SkillCategory::Domain),
            "framework" => Some(SkillCategory::Framework),
            _ => None,
        }
    }
}

/// A skill the agent possesses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "postgres-storage", derive(utoipa::ToSchema))]
pub struct AgentSkill {
    /// Unique identifier.
    pub id: Uuid,
    /// Human-readable skill name, e.g. `Rust async/await`.
    pub skill_name: String,
    /// Category, e.g. `language`, `tool`, `domain`, `framework`.
    pub category: String,
    /// Proficiency on a 1–10 scale.
    pub proficiency: i32,
    /// When this skill was last invoked.
    pub last_used: Option<DateTime<Utc>>,
    /// Historical success rate 0.0–1.0.
    pub success_rate: Option<f64>,
    /// Named sub-components, e.g. `["tokio", "sqlx", "axum"]`.
    pub components: Vec<String>,
    /// Other skills or knowledge areas required before this one.
    pub prerequisites: Vec<String>,
    /// Namespace where related memories live (defaults to `agent/skills`).
    pub namespace_id: Option<Uuid>,
    /// Arbitrary extra data.
    pub metadata: serde_json::Value,
}

/// Request to create a new skill.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "postgres-storage", derive(utoipa::ToSchema))]
pub struct CreateSkillRequest {
    pub skill_name: String,
    pub category: String,
    #[serde(default = "default_proficiency")]
    pub proficiency: i32,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    /// Optional namespace ID (defaults to the `agent/skills` namespace).
    #[serde(default)]
    pub namespace_id: Option<Uuid>,
    /// Extra metadata (optional).
    #[serde(default)]
    pub metadata: serde_json::Value,
}

fn default_proficiency() -> i32 {
    5
}

/// Request to update an existing skill.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "postgres-storage", derive(utoipa::ToSchema))]
pub struct UpdateSkillRequest {
    #[serde(default)]
    pub skill_name: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub proficiency: Option<i32>,
    #[serde(default)]
    pub components: Option<Vec<String>>,
    #[serde(default)]
    pub prerequisites: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Response for skill creation.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "postgres-storage", derive(utoipa::ToSchema))]
pub struct CreateSkillResponse {
    pub id: Uuid,
    pub message: String,
}

/// Response for skill update or delete.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "postgres-storage", derive(utoipa::ToSchema))]
pub struct UpdateSkillResponse {
    pub message: String,
}

/// Result from matching a task query to relevant skills.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "postgres-storage", derive(utoipa::ToSchema))]
pub struct MatchedSkill {
    pub skill: AgentSkill,
    pub relevance_score: f64,
}

/// Database-backed store for agent skills.
#[cfg(feature = "postgres-storage")]
pub struct SkillsStore<'a> {
    pool: &'a PgPool,
}

#[cfg(feature = "postgres-storage")]
impl<'a> SkillsStore<'a> {
    /// Create a new SkillsStore.
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Create a new skill and return its UUID.
    pub async fn create(&self, req: &CreateSkillRequest) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();

        sqlx::query!(
            r#"
            INSERT INTO agent_skills (
                id, skill_name, category, proficiency, components, prerequisites, namespace_id, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            id,
            req.skill_name,
            req.category,
            req.proficiency,
            &req.components,
            &req.prerequisites,
            req.namespace_id,
            req.metadata,
        )
        .execute(self.pool)
        .await?;

        Ok(id)
    }

    /// Get a skill by ID.
    pub async fn get(&self, id: Uuid) -> anyhow::Result<Option<AgentSkill>> {
        let row = sqlx::query_as!(
            AgentSkillRow,
            r#"
            SELECT id, skill_name, category, proficiency, last_used,
                   success_rate, components, prerequisites, namespace_id, metadata
            FROM agent_skills
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    /// List skills, optionally filtered by category and minimum proficiency.
    pub async fn list(
        &self,
        category: Option<&str>,
        min_proficiency: Option<i32>,
    ) -> anyhow::Result<Vec<AgentSkill>> {
        let rows = match (category, min_proficiency) {
            (Some(cat), Some(min_prof)) => {
                sqlx::query_as!(
                    AgentSkillRow,
                    r#"
                    SELECT id, skill_name, category, proficiency, last_used,
                           success_rate, components, prerequisites, namespace_id, metadata
                    FROM agent_skills
                    WHERE category = $1 AND proficiency >= $2
                    ORDER BY proficiency DESC, skill_name ASC
                    "#,
                    cat,
                    min_prof,
                )
                .fetch_all(self.pool)
                .await?
            }
            (Some(cat), None) => {
                sqlx::query_as!(
                    AgentSkillRow,
                    r#"
                    SELECT id, skill_name, category, proficiency, last_used,
                           success_rate, components, prerequisites, namespace_id, metadata
                    FROM agent_skills
                    WHERE category = $1
                    ORDER BY proficiency DESC, skill_name ASC
                    "#,
                    cat,
                )
                .fetch_all(self.pool)
                .await?
            }
            (None, Some(min_prof)) => {
                sqlx::query_as!(
                    AgentSkillRow,
                    r#"
                    SELECT id, skill_name, category, proficiency, last_used,
                           success_rate, components, prerequisites, namespace_id, metadata
                    FROM agent_skills
                    WHERE proficiency >= $1
                    ORDER BY proficiency DESC, skill_name ASC
                    "#,
                    min_prof,
                )
                .fetch_all(self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as!(
                    AgentSkillRow,
                    r#"
                    SELECT id, skill_name, category, proficiency, last_used,
                           success_rate, components, prerequisites, namespace_id, metadata
                    FROM agent_skills
                    ORDER BY proficiency DESC, skill_name ASC
                    "#,
                )
                .fetch_all(self.pool)
                .await?
            }
        };

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Update an existing skill.
    pub async fn update(&self, id: Uuid, updates: &UpdateSkillRequest) -> anyhow::Result<()> {
        // Fetch existing to merge
        let existing = self.get(id).await?.ok_or_else(|| anyhow::anyhow!("skill {id} not found"))?;

        let skill_name = updates.skill_name.as_ref().unwrap_or(&existing.skill_name);
        let category = updates.category.as_ref().unwrap_or(&existing.category);
        let proficiency = updates.proficiency.unwrap_or(existing.proficiency);
        let components = updates.components.clone().unwrap_or(existing.components);
        let prerequisites = updates.prerequisites.clone().unwrap_or(existing.prerequisites);
        let metadata = updates.metadata.clone().unwrap_or(existing.metadata);

        sqlx::query!(
            r#"
            UPDATE agent_skills
            SET skill_name = $2,
                category = $3,
                proficiency = $4,
                components = $5,
                prerequisites = $6,
                metadata = $7,
                updated_at = NOW()
            WHERE id = $1
            "#,
            id,
            skill_name,
            category,
            proficiency,
            &components,
            &prerequisites,
            metadata,
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Delete a skill by ID.
    pub async fn delete(&self, id: Uuid) -> anyhow::Result<()> {
        let result = sqlx::query!("DELETE FROM agent_skills WHERE id = $1", id)
            .execute(self.pool)
            .await?;

        if result.rows_affected() == 0 {
            anyhow::bail!("skill {id} not found");
        }
        Ok(())
    }

    /// Record a skill usage event.
    ///
    /// Updates `last_used` and recalculates the rolling `success_rate`.
    pub async fn mark_used(&self, id: Uuid, success: bool) -> anyhow::Result<()> {
        // Fetch current success_rate for rolling average
        let current = sqlx::query_scalar!(
            "SELECT success_rate FROM agent_skills WHERE id = $1",
            id,
        )
        .fetch_optional(self.pool)
        .await?;

        let current_rate = current.unwrap_or(0.0);
        // Simple rolling average: new = (old * 3 + (1 if success else 0)) / 4
        let new_rate = if success {
            current_rate * 0.75 + 0.25
        } else {
            current_rate * 0.75
        };

        sqlx::query!(
            r#"
            UPDATE agent_skills
            SET last_used = NOW(),
                success_rate = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
            id,
            new_rate,
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Find skills relevant to a task description.
    ///
    /// Uses a simple text match against `skill_name`, `category`, and `components`.
    /// Returns up to `top_k` results ordered by relevance.
    pub async fn match_task(&self, task_query: &str, top_k: usize) -> anyhow::Result<Vec<AgentSkill>> {
        let pattern = format!("%{task_query}%");

        let rows = sqlx::query_as!(
            AgentSkillRow,
            r#"
            SELECT id, skill_name, category, proficiency, last_used,
                   success_rate, components, prerequisites, namespace_id, metadata
            FROM agent_skills
            WHERE skill_name ILIKE $1
               OR category ILIKE $1
               OR $2 = ANY(components)
            ORDER BY proficiency DESC
            LIMIT $3
            "#,
            pattern,
            task_query,
            top_k as i32,
        )
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Link a memory to a skill.
    pub async fn link_memory(
        &self,
        skill_id: Uuid,
        memory_id: Uuid,
        relation_type: &str,
    ) -> anyhow::Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO skill_memories (skill_id, memory_id, relation_type)
            VALUES ($1, $2, $3)
            ON CONFLICT (skill_id, memory_id) DO UPDATE SET relation_type = EXCLUDED.relation_type
            "#,
            skill_id,
            memory_id,
            relation_type,
        )
        .execute(self.pool)
        .await?;

        Ok(())
    }

    /// Get the agent/skills namespace ID, creating it if it doesn't exist.
    pub async fn get_or_create_agent_skills_namespace(&self) -> anyhow::Result<Uuid> {
        let row = sqlx::query_scalar!(
            r#"
            SELECT id FROM memory_namespaces WHERE path = 'agent/skills'
            "#,
        )
        .fetch_optional(self.pool)
        .await?;

        if let Some(id) = row {
            return Ok(id);
        }

        // Create it
        let id = Uuid::new_v4();
        sqlx::query!(
            r#"
            INSERT INTO memory_namespaces (id, path, depth, description)
            VALUES ($1, 'agent/skills', 2, 'Agent capabilities and skills')
            ON CONFLICT (path) DO NOTHING
            "#,
            id,
        )
        .execute(self.pool)
        .await?;

        // Fetch again in case of race
        sqlx::query_scalar!(
            r#"SELECT id FROM memory_namespaces WHERE path = 'agent/skills'"#,
        )
        .fetch_one(self.pool)
        .await
        .map_err(Into::into)
    }
}

// Internal row type matching the SQLx query.
#[cfg(feature = "postgres-storage")]
sqlx::FromRow! {
    #[derive(Debug)]
    struct AgentSkillRow {
        id: Uuid,
        skill_name: String,
        category: String,
        proficiency: i32,
        last_used: Option<DateTime<Utc>>,
        success_rate: Option<f64>,
        components: Vec<String>,
        prerequisites: Vec<String>,
        namespace_id: Option<Uuid>,
        metadata: serde_json::Value,
    }
}

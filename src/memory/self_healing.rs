//! Self-Healing Service for External Nodes.
//!
//! If an external file is moved (URI/path changes), the pointer becomes dangling.
//! This service can recover the file location using:
//!
//! 1. **Content Hash (BLAKE3)**: A cryptographic hash of the file content.
//!    Does NOT change when the file is moved.
//!
//! 2. **Semantic Thumbnail**: First 100 words of the text content.
//!    Enables semantic search as a fallback when hash lookup fails.
//!
//! ## Workflow
//!
//! 1. `index_external_node(path)` — when storing: compute hash + thumbnail
//! 2. `check_and_repair(memory_id, uri)` — check if pointer is still valid;
//!    if broken, try hash lookup, then semantic search
//! 3. `find_by_hash(hash)` — search file_root recursively for a file with this hash
//! 4. `update_pointer(memory_id, old_uri, new_uri)` — update DB when file is found

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use blake3::Hash;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of a self-healing check.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct HealthCheckResult {
    pub memory_id: Uuid,
    pub uri: String,
    pub pointer_valid: bool,
    pub repair_status: Option<RepairStatus>,
    pub repaired_uri: Option<String>,
}

/// How a pointer was repaired (or that it could not be repaired).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepairStatus {
    RepairedHash,
    RepairedSemantic,
    Unrepaired,
}

/// Statistics about broken vs. repaired pointers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct HealingStats {
    pub total_checked: i64,
    pub currently_broken: i64,
    pub repaired_via_hash: i64,
    pub repaired_via_semantic: i64,
    pub unrepaired: i64,
}

// ---------------------------------------------------------------------------
// SelfHealingService
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SelfHealingService {
    pool: PgPool,
    /// Root directory to search for files when healing.
    file_root: PathBuf,
}

impl SelfHealingService {
    /// Create a new SelfHealingService.
    pub fn new(pool: PgPool, file_root: PathBuf) -> Self {
        Self { pool, file_root }
    }

    /// Index an external node: compute BLAKE3 hash + semantic thumbnail from file.
    ///
    /// Call this when storing a new external node, or when re-indexing.
    pub async fn index_external_node(&self, memory_id: Uuid, file_path: &Path) -> Result<()> {
        let content = tokio::fs::read(file_path)
            .await
            .with_context(|| format!("failed to read file for indexing: {}", file_path.display()))?;

        // 1. Compute BLAKE3 hash
        let hash = blake3::hash(&content);
        let hash_hex = hash.to_hex().to_string();

        // 2. Generate semantic thumbnail (first 100 words of text)
        let text_content = extract_text_from_bytes(&content);
        let thumbnail: String = text_content
            .split_whitespace()
            .take(100)
            .collect::<Vec<_>>()
            .join(" ");

        // 3. Store in DB
        sqlx::query!(
            r#"
            UPDATE memories
            SET content_hash = $1, semantic_thumbnail = $2
            WHERE id = $3
            "#,
            hash_hex,
            thumbnail,
            memory_id,
        )
        .execute(&self.pool)
        .await
        .context("failed to update content_hash/semantic_thumbnail for memory")?;

        tracing::debug!(%memory_id, hash = %hash_hex, thumbnail_len = thumbnail.len(), "external node indexed");
        Ok(())
    }

    /// Check whether a pointer (URI/path) is still valid.
    /// If broken, attempt self-healing via hash lookup then semantic search.
    ///
    /// Returns `Some(new_uri)` if repaired, `None` if pointer is still valid or could not be repaired.
    pub async fn check_and_repair(
        &self,
        memory_id: Uuid,
        uri: &str,
    ) -> Result<Option<(String, RepairStatus)>> {
        let path = self.uri_to_path(uri);

        if path.exists() {
            return Ok(None); // Pointer still valid
        }

        tracing::info!(%memory_id, uri, "pointer broken, attempting self-healing");

        // Fetch memory row to get hash + thumbnail
        let row = sqlx::query!(
            r#"
            SELECT content_hash, semantic_thumbnail
            FROM memories
            WHERE id = $1 AND status != 'deleted'
            "#,
            memory_id,
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch memory for self-healing")?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        // Attempt 1: Hash-based recovery
        if let Some(hash) = row.content_hash {
            if let Some(new_path) = self.find_by_hash(&hash).await? {
                let new_uri = self.path_to_uri(&new_path);
                self.update_pointer(memory_id, uri, &new_uri, RepairStatus::RepairedHash)
                    .await?;
                return Ok(Some((new_uri, RepairStatus::RepairedHash)));
            }
        }

        // Attempt 2: Semantic thumbnail fallback
        if let Some(thumbnail) = row.semantic_thumbnail {
            if let Some(new_path) = self.find_by_semantic(&thumbnail).await? {
                let new_uri = self.path_to_uri(&new_path);
                self.update_pointer(memory_id, uri, &new_uri, RepairStatus::RepairedSemantic)
                    .await?;
                return Ok(Some((new_uri, RepairStatus::RepairedSemantic)));
            }
        }

        // Could not repair
        self.log_healing(memory_id, uri, RepairStatus::Unrepaired, None)
            .await?;
        Ok(None)
    }

    /// Get full health check result for a memory (for the /health endpoint).
    pub async fn health_check(&self, memory_id: Uuid) -> Result<HealthCheckResult> {
        let row = sqlx::query!(
            r#"
            SELECT original_pointer
            FROM memories
            WHERE id = $1 AND status != 'deleted'
            "#,
            memory_id,
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch memory for health check")?;

        let uri = row
            .and_then(|r| r.original_pointer)
            .unwrap_or_default();

        let path = self.uri_to_path(&uri);
        let pointer_valid = path.exists();

        let (repair_status, repaired_uri) = if pointer_valid {
            (None, None)
        } else {
            match self.check_and_repair(memory_id, &uri).await? {
                Some((new_uri, method)) => (Some(method), Some(new_uri)),
                None => (Some(RepairStatus::Unrepaired), None),
            }
        };

        Ok(HealthCheckResult {
            memory_id,
            uri,
            pointer_valid,
            repair_status,
            repaired_uri,
        })
    }

    /// Get self-healing statistics.
    pub async fn stats(&self) -> Result<HealingStats> {
        let total_checked: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM self_healing_log",
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count total self-healing log entries")?;

        let currently_broken: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT memory_id) FROM self_healing_log WHERE repair_status = 'unrepaired'",
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count broken pointers")?;

        let repaired_via_hash: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM self_healing_log WHERE repair_status = 'repaired_hash'",
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count hash repairs")?;

        let repaired_via_semantic: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM self_healing_log WHERE repair_status = 'repaired_semantic'",
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count semantic repairs")?;

        let unrepaired: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM self_healing_log WHERE repair_status = 'unrepaired'",
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count unrepaired")?;

        Ok(HealingStats {
            total_checked,
            currently_broken,
            repaired_via_hash,
            repaired_via_semantic,
            unrepaired,
        })
    }

    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    /// Search file_root recursively for a file whose BLAKE3 hash matches.
    async fn find_by_hash(&self, target_hash: &str) -> Result<Option<PathBuf>> {
        let target: Hash = target_hash
            .parse()
            .context("invalid blake3 hash string")?;

        let mut candidate: Option<PathBuf> = None;
        let mut visited = 0usize;

        let mut stack = vec![self.file_root.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Some(entry) = entries.next_entry().await.transpose() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                visited += 1;
                if visited > 100_000 {
                    tracing::warn!("find_by_hash: visited limit reached, stopping search");
                    break;
                }

                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    // Only hash regular files (not directories/symlinks)
                    if let Ok(content) = tokio::fs::read(&path).await {
                        let file_hash = blake3::hash(&content);
                        if file_hash == target {
                            candidate = Some(path);
                            break;
                        }
                    }
                }
            }
            if candidate.is_some() {
                break;
            }
        }

        if candidate.is_some() {
            tracing::info!(hash = %target_hash, path = ?candidate, "found file by hash");
        } else {
            tracing::debug!(hash = %target_hash, "no file found with matching hash");
        }

        Ok(candidate)
    }

    /// Fallback: search for a file by semantic similarity of its content to the thumbnail.
    /// This is a simple keyword-overlap approach (not full embedding similarity).
    async fn find_by_semantic(&self, thumbnail: &str) -> Result<Option<PathBuf>> {
        let thumbnail_words: std::collections::HashSet<&str> =
            thumbnail.split_whitespace().collect();

        if thumbnail_words.is_empty() {
            return Ok(None);
        }

        let mut best_match: Option<(PathBuf, usize)> = None;
        let mut visited = 0usize;

        let mut stack = vec![self.file_root.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Some(entry) = entries.next_entry().await.transpose() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                visited += 1;
                if visited > 50_000 {
                    tracing::warn!("find_by_semantic: visited limit reached");
                    break;
                }

                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    if let Ok(content) = tokio::fs::read(&path).await {
                        let text = extract_text_from_bytes(&content);
                        let file_words: std::collections::HashSet<&str> =
                            text.split_whitespace().collect();

                        let overlap: usize = thumbnail_words
                            .intersection(&file_words)
                            .count();

                        // Require at least 10 common words to be considered a match
                        if overlap >= 10 {
                            if let Some((_, best_overlap)) = &best_match {
                                if overlap > *best_overlap {
                                    best_match = Some((path.clone(), overlap));
                                }
                            } else {
                                best_match = Some((path.clone(), overlap));
                            }
                        }
                    }
                }
            }
            if best_match.is_some() {
                break;
            }
        }

        if let Some((ref path, overlap)) = best_match {
            tracing::info!(overlap, path = %path.display(), "found file by semantic thumbnail");
        }

        Ok(best_match.map(|(p, _)| p))
    }

    /// Update the `original_pointer` field in the DB after a successful repair.
    async fn update_pointer(
        &self,
        memory_id: Uuid,
        old_uri: &str,
        new_uri: &str,
        status: RepairStatus,
    ) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE memories
            SET original_pointer = $1, updated_at = NOW()
            WHERE id = $2
            "#,
            new_uri,
            memory_id,
        )
        .execute(&self.pool)
        .await
        .context("failed to update repaired pointer in DB")?;

        self.log_healing(memory_id, old_uri, status, Some(new_uri))
            .await?;

        tracing::info!(%memory_id, old_uri, new_uri, ?status, "pointer repaired");
        Ok(())
    }

    /// Log a self-healing attempt to the audit table.
    async fn log_healing(
        &self,
        memory_id: Uuid,
        broken_uri: &str,
        status: RepairStatus,
        new_uri: Option<&str>,
    ) -> Result<()> {
        let status_str = match status {
            RepairStatus::RepairedHash => "repaired_hash",
            RepairStatus::RepairedSemantic => "repaired_semantic",
            RepairStatus::Unrepaired => "unrepaired",
        };

        sqlx::query!(
            r#"
            INSERT INTO self_healing_log (memory_id, broken_uri, repair_status, new_uri)
            VALUES ($1, $2, $3, $4)
            "#,
            memory_id,
            broken_uri,
            status_str,
            new_uri,
        )
        .execute(&self.pool)
        .await
        .context("failed to log self-healing event")?;

        Ok(())
    }

    /// Convert a URI string to a file path.
    /// Handles both absolute paths and `file://` URIs.
    fn uri_to_path(&self, uri: &str) -> PathBuf {
        if uri.starts_with("file://") {
            PathBuf::from(&uri[7..])
        } else {
            PathBuf::from(uri)
        }
    }

    /// Convert a file path back to a URI string.
    fn path_to_uri(&self, path: &Path) -> String {
        format!("file://{}", path.display())
    }
}

// ---------------------------------------------------------------------------
// Text extraction helper
// ---------------------------------------------------------------------------

/// Extract readable text from file bytes.
///
/// Currently supports:
/// - UTF-8 plain text (most common for code/docs)
/// - Falls back to lossy UTF-8 string for binary data
fn extract_text_from_bytes(bytes: &[u8]) -> String {
    // Try UTF-8 first
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    // Fallback: lossy UTF-8 (replaces invalid sequences)
    String::from_utf8_lossy(bytes).to_string()
}

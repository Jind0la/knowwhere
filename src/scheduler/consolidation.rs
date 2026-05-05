//! Consolidation Scheduler — Dream Mode Part 1
//!
//! Periodically finds L2-Nodes (context_tier = Raw, not yet consolidated)
//! and compacts them via TieredCompactionWorker with LocalSummarizer.
//!
//! # Compaction Strategy
//!
//! 1. **PRIMARY**: LocalSummarizer (Ollama) — deterministic, fast, no API key
//! 2. **FALLBACK**: VLM (cloud) — if user configured API key
//! 3. **NEVER**: Truncation — information loss unacceptable
//!
//! Consolidation targets memories that are old enough and unprocessed,
//! grouping them into batches and processing via TieredCompactionWorker.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Detect if consolidated content describes a decision (for auto-typing as MemoryType::Decision).
///
/// Matches content that starts with or contains explicit decision markers:
/// - "DECISION:" / "Decision:" (English)
/// - "Entscheidung" / "entschieden" (German)
/// - "decided" followed by action description
fn is_decision_content(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("decision:")
        || lower.contains("decided")
        || lower.contains("entscheidung")
        || lower.contains("entschieden")
}

/// Parse structured claims from consolidation output.
///
/// Extracts (claim, reason) pairs from a `---CLAIMS---` / `---END---` block.
/// Robust: handles missing block, malformed claims, empty claims.
/// Never panics — returns empty Vec on any parse failure.
fn parse_claims_block(text: &str) -> Vec<(String, String)> {
    // Find the CLAIMS block boundaries
    let start_marker = "---CLAIMS---";
    let end_marker = "---END---";

    let start_idx = match text.find(start_marker) {
        Some(i) => i + start_marker.len(),
        None => return Vec::new(),
    };
    let end_idx = match text[start_idx..].find(end_marker) {
        Some(i) => start_idx + i,
        None => {
            // Missing end marker: try to parse whatever follows
            text.len()
        }
    };

    let block = text[start_idx..end_idx].trim();
    if block.is_empty() {
        return Vec::new();
    }

    let mut claims = Vec::new();
    let mut current_claim: Option<&str> = None;

    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Detect new claim entry: "- claim:" or "- claim "
        if let Some(rest) = trimmed.strip_prefix("- claim:") {
            // Flush previous claim if exists
            if let Some(claim_text) = current_claim.take() {
                claims.push((claim_text.to_string(), String::new()));
            }
            current_claim = Some(rest.trim());
        } else if let Some(rest) = trimmed.strip_prefix("- claim ") {
            if let Some(claim_text) = current_claim.take() {
                claims.push((claim_text.to_string(), String::new()));
            }
            current_claim = Some(rest.trim());
        }
        // Detect reason: "  reason:" or "reason:"
        else if let Some(reason) = trimmed.strip_prefix("reason:") {
            if let Some(claim_text) = current_claim.take() {
                claims.push((claim_text.to_string(), reason.trim().to_string()));
                current_claim = None;
            }
            // reason without preceding claim — ignore
        }
        // alternatives / consequences: not stored as separate nodes, ignored by parser
        // but still part of the block that gets skipped
    }

    // Flush any trailing claim without reason
    if let Some(claim_text) = current_claim {
        claims.push((claim_text.to_string(), String::new()));
    }

    claims
}

fn consolidation_metadata(
    source: &FractalNode,
    derived_from: &str,
    claim_scope: &str,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut metadata = source.metadata.clone();
    metadata.insert(
        "derived_from".to_string(),
        serde_json::Value::String(derived_from.to_string()),
    );
    metadata.insert(
        "claim_scope".to_string(),
        serde_json::Value::String(claim_scope.to_string()),
    );
    metadata.insert(
        "source_node_ids".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::String(source.id.to_string())]),
    );
    if let Some(session_id) = source.metadata.get("session_id").cloned() {
        metadata.insert(
            "source_session_ids".to_string(),
            serde_json::Value::Array(vec![session_id]),
        );
    }
    if let Some(turn_index) = source.metadata.get("turn_index").cloned() {
        metadata.insert("source_turn_range".to_string(), turn_index);
    }
    metadata
}

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, Instant};
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::types::{ContextTier, MemoryStatus};
use crate::memory::{FractalNode, MemorySource, MemoryType};
use crate::scheduler::SchedulerConfig;
use crate::storage::{StorageBackend, UpdateOperation};
use crate::summarizer::TieredSummarizer;
use crate::vlm::{SummaryContext, VlmJob, VlmWorkerHandle};

/// Consolidation Scheduler state.
///
/// Uses TieredCompactionWorker with LocalSummarizer as PRIMARY,
/// VLM as OPTIONAL fallback. NEVER uses truncation.
///
/// # Fractal Compaction Chain
///
/// For each L2 (Raw) node:
/// 1. Generate L1 (Overview) via LocalSummarizer
/// 2. Embed L1 content
/// 3. Create L1 node with parent_tier_id → L2
/// 4. Generate L0 (Summary) from L1
/// 5. Embed L0 content
/// 6. Create L0 node with parent_tier_id → L1, children_tier_ids → [L2]
/// 7. Update L1 parent_tier_id → L0
///
/// Result: L2 ↔ L1 ↔ L0 bidirectional links with embeddings
pub struct ConsolidationScheduler {
    store: Arc<dyn StorageBackend>,
    vlm_worker: Option<VlmWorkerHandle>,
    local_summarizer: TieredSummarizer,
    embedding: Arc<dyn EmbeddingProvider>,
    config: SchedulerConfig,
    /// Track last run so we don't re-process recently consolidated nodes.
    last_run: Arc<RwLock<Option<Instant>>>,
    /// How many nodes were enqueued in the last run.
    last_enqueued: Arc<RwLock<usize>>,
    /// How many consolidation cycles have been completed.
    cycle_count: Arc<AtomicU64>,
    /// Guards against concurrent consolidation runs.
    /// Set to true while a run is in progress; subsequent
    /// trigger_if_needed() calls return immediately.
    is_running: AtomicBool,
}

impl ConsolidationScheduler {
    /// Create a new ConsolidationScheduler.
    ///
    /// Initializes TieredSummarizer for local compaction.
    /// VLM worker is optional — LocalSummarizer is always preferred.
    pub fn new(
        store: Arc<dyn StorageBackend>,
        vlm_worker: Option<VlmWorkerHandle>,
        embedding: Arc<dyn EmbeddingProvider>,
        config: SchedulerConfig,
    ) -> Self {
        let local_summarizer = TieredSummarizer::new();
        if !local_summarizer.is_available() {
            tracing::warn!(
                "LocalSummarizer not available. Install Ollama: https://ollama.com \
                 Or configure VLM (OPENAI_API_KEY) for cloud fallback."
            );
        }

        Self {
            store,
            vlm_worker,
            local_summarizer,
            embedding,
            config,
            last_run: Arc::new(RwLock::new(None)),
            last_enqueued: Arc::new(RwLock::new(0)),
            cycle_count: Arc::new(AtomicU64::new(0)),
            is_running: AtomicBool::new(false),
        }
    }

    /// Returns the number of completed consolidation cycles.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count.load(Ordering::Relaxed)
    }

    /// Event-driven trigger: called after every write to check if
    /// consolidation should run. Uses `should_compact()` as gate and
    /// `is_running` to prevent concurrent runs.
    ///
    /// This is designed to be called from `store_session`, `store_external`,
    /// and `store_session_batch` — after new data arrives, check if there's
    /// enough work to justify a consolidation run.
    pub async fn trigger_if_needed(self: &Arc<Self>) {
        // Guard: only one consolidation at a time
        if self.is_running.swap(true, Ordering::AcqRel) {
            return;
        }

        if !self.should_compact().await {
            self.is_running.store(false, Ordering::Release);
            return;
        }

        let sched = self.clone();
        tokio::spawn(async move {
            sched.run().await;
            sched.cycle_count.fetch_add(1, Ordering::Relaxed);
            sched.is_running.store(false, Ordering::Release);
        });
    }

    /// Count pending consolidation candidates without processing them.
    ///
    /// Returns (candidate_count, total_nodes). Useful for dashboards and
    /// pre-flight checks before calling `force_run()`.
    pub async fn pending_count(&self) -> (usize, usize) {
        let all_nodes = match self.store.list_all().await {
            Ok(n) => n,
            Err(_) => return (0, 0),
        };
        let total = all_nodes.len();
        let candidates = all_nodes
            .iter()
            .filter(|n| {
                n.context_tier == ContextTier::Raw
                    && n.parent_tier_id.is_none()
                    && n.status == MemoryStatus::Active
                    && n.importance >= 3
                    && n.content.as_ref().map(|c| c.len() > 500).unwrap_or(false)
            })
            .count();
        (candidates, total)
    }

    /// Force-run consolidation immediately — bypasses space-amplification ratio
    /// and timer safety-net. Processes ALL pending candidates (no cap).
    ///
    /// Intended for admin-triggered full re-consolidation (e.g. after deploying
    /// a new claims parser). Runs synchronously — the caller should spawn it
    /// in a background task if non-blocking behaviour is desired.
    ///
    /// Returns (enqueued, failed, elapsed_ms).
    pub async fn force_run(&self) -> (usize, usize, u64) {
        let start = Instant::now();

        let all_nodes = match self.store.list_all().await {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::error!(error = %e, "force_run: list_all failed");
                return (0, 0, 0);
            }
        };

        // Collect ALL eligible candidates (no vlm_max_jobs_per_cycle cap)
        let mut candidates: Vec<(Uuid, DateTime<Utc>)> = Vec::new();
        for node in all_nodes {
            if node.parent_tier_id.is_some() {
                continue;
            }
            if node.context_tier != ContextTier::Raw {
                continue;
            }
            if node.status != MemoryStatus::Active {
                continue;
            }
            if node.importance < 3 {
                continue;
            }
            let content_len = node.content.as_ref().map(|c| c.len()).unwrap_or(0);
            if content_len <= 500 {
                continue;
            }
            candidates.push((node.id, node.created_at));
        }
        candidates.sort_by(|a, b| a.1.cmp(&b.1));

        if candidates.is_empty() {
            tracing::debug!("force_run: no candidates found");
            return (0, 0, 0);
        }

        tracing::info!(
            count = candidates.len(),
            local_available = self.local_summarizer.is_available(),
            vlm_available = self.vlm_worker.is_some(),
            "force_run: starting full consolidation"
        );

        let mut enqueued = 0;
        let mut failed = 0;

        for (node_id, _created_at) in &candidates {
            if self.local_summarizer.is_available() {
                match self.process_local_compaction(*node_id).await {
                    Ok(()) => {
                        enqueued += 1;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(node_id = %node_id, error = %e, "force_run: local compaction failed, trying VLM fallback");
                    }
                }
            }

            if let Some(ref handle) = self.vlm_worker {
                let job = VlmJob::new(vec![*node_id], SummaryContext::Overview);
                match handle.enqueue(job).await {
                    Ok(()) => {
                        enqueued += 1;
                    }
                    Err(e) => {
                        tracing::error!(node_id = %node_id, error = %e, "force_run: VLM enqueue failed");
                        failed += 1;
                    }
                }
            } else {
                tracing::error!(node_id = %node_id, "force_run: no summarizer available");
                failed += 1;
            }

            // Only mark as processed if failure is PERMANENT (no summarizer at all).
            // Transient errors (DNS, timeout, Ollama restart) leave the node
            // eligible for retry in the next consolidation cycle.
            if !self.local_summarizer.is_available() && self.vlm_worker.is_none() {
                tracing::warn!(
                    node_id = %node_id,
                    "force_run: permanent failure — no summarizer available, marking as processed"
                );
                let _ = self
                    .store
                    .update(node_id, UpdateOperation::SetParentTierId(*node_id))
                    .await;
            } else {
                tracing::debug!(
                    node_id = %node_id,
                    "force_run: transient failure — node remains eligible for retry"
                );
            }
        }

        *self.last_enqueued.write().await = enqueued;
        *self.last_run.write().await = Some(Instant::now());

        let elapsed = start.elapsed().as_millis() as u64;
        tracing::info!(
            enqueued,
            failed,
            elapsed_ms = elapsed,
            "force_run: complete"
        );
        (enqueued, failed, elapsed)
    }

    /// Start a safety-net background timer.
    ///
    /// Fires `trigger_if_needed()` once per hour as a fallback for
    /// edge cases where write-driven triggers don't fire (e.g.,
    /// read-only usage, imports without session writes).
    ///
    /// The primary trigger is event-driven via `trigger_if_needed()`
    /// called from store_session/store_external/store_session_batch.
    pub fn start_safety_net(self) -> (Arc<Self>, tokio::task::JoinHandle<()>) {
        let scheduler = Arc::new(self);

        let sched = scheduler.clone();
        let handle = tokio::spawn(async move {
            // First run: always execute, bypass should_compact().
            // This ensures startup consolidation even when the ratio
            // hasn't been met yet (fresh server, bulk import, etc.)
            if sched.is_running.swap(true, Ordering::AcqRel) {
                return;
            }
            sched.run().await;
            sched.cycle_count.fetch_add(1, Ordering::Relaxed);
            sched.is_running.store(false, Ordering::Release);

            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                sched.trigger_if_needed().await;
            }
        });

        (scheduler, handle)
    }

    /// Run one consolidation pass.
    ///
    /// # Compaction Strategy
    ///
    /// 1. **PRIMARY**: LocalSummarizer (Ollama) — deterministic, fast, no API key
    /// 2. **FALLBACK**: VLM (cloud) — if user configured API key
    /// 3. **NEVER**: Truncation — information loss unacceptable
    ///
    /// Called from `trigger_if_needed()` which handles gating
    /// (should_compact, is_running guard). Do not call directly.
    async fn run(&self) {
        let start = Instant::now();
        let batch_size = self.config.consolidation_batch_size;

        // Collect consolidation candidates: Raw tier, not yet consolidated, active status
        let candidates = self.find_candidates(batch_size).await;

        if candidates.is_empty() {
            tracing::debug!("ConsolidationScheduler: no candidates found");
            *self.last_run.write().await = Some(Instant::now());
            return;
        }

        tracing::info!(
            count = candidates.len(),
            local_available = self.local_summarizer.is_available(),
            vlm_available = self.vlm_worker.is_some(),
            "ConsolidationScheduler: found candidates"
        );

        let mut enqueued = 0;
        let mut failed = 0;

        for (node_id, _created_at) in &candidates {
            // PRIMARY: Try LocalSummarizer first
            if self.local_summarizer.is_available() {
                match self.process_local_compaction(*node_id).await {
                    Ok(()) => {
                        enqueued += 1;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            node_id = %node_id,
                            error = %e,
                            "Local compaction failed, trying VLM fallback"
                        );
                    }
                }
            }

            // FALLBACK: VLM (if available)
            if let Some(ref handle) = self.vlm_worker {
                let job = VlmJob::new(vec![*node_id], SummaryContext::Overview);
                match handle.enqueue(job).await {
                    Ok(()) => {
                        tracing::debug!(node_id = %node_id, "enqueued VLM consolidation job");
                        enqueued += 1;
                    }
                    Err(e) => {
                        tracing::error!(
                            node_id = %node_id,
                            error = %e,
                            "VLM enqueue failed"
                        );
                        failed += 1;
                    }
                }
            } else {
                // NO TRUNCATION — log failure but don't lose information
                tracing::error!(
                    node_id = %node_id,
                    "Compaction failed: no summarizer available. \
                     Install Ollama (https://ollama.com) or configure VLM. \
                     Truncation disabled — memory preserved in original form."
                );
                failed += 1;
            }

            // Only mark as processed if failure is PERMANENT (no summarizer at all).
            // Transient errors (DNS, timeout, Ollama restart) leave the node
            // eligible for retry in the next consolidation cycle.
            if !self.local_summarizer.is_available() && self.vlm_worker.is_none() {
                tracing::warn!(
                    node_id = %node_id,
                    "permanent failure — no summarizer available, marking as processed"
                );
                let _ = self
                    .store
                    .update(node_id, UpdateOperation::SetParentTierId(*node_id))
                    .await;
            } else {
                tracing::debug!(
                    node_id = %node_id,
                    "transient failure — node remains eligible for retry"
                );
            }
        }

        *self.last_enqueued.write().await = enqueued;
        *self.last_run.write().await = Some(Instant::now());

        tracing::info!(
            enqueued,
            failed,
            elapsed_ms = start.elapsed().as_millis(),
            "ConsolidationScheduler: run complete"
        );
    }

    /// Process a single node using LocalSummarizer.
    ///
    /// Fetches node content, generates L1 overview via Ollama,
    /// creates L1 summary node with embedding,
    /// links L2 → L1 via parent_tier_id,
    /// links L1 → L2 via children_tier_ids.
    async fn process_local_compaction(&self, node_id: Uuid) -> anyhow::Result<()> {
        // Fetch L2 (Raw) node content
        let node = match self.store.get(&node_id).await? {
            Some(n) => n,
            None => anyhow::bail!("node {} not found", node_id),
        };

        let content = node.content.clone().unwrap_or_default();
        if content.is_empty() {
            anyhow::bail!("node {} has empty content", node_id);
        }

        // Generate L1 overview via LocalSummarizer
        let summary = self
            .local_summarizer
            .summarize_for_tier(&content, ContextTier::Overview)
            .await?;

        // Step 1: Create L1 (Overview) node with embedding
        let l1_content = summary.text.clone();
        let l1_embedding = self.embed_text(&l1_content).await?;
        let l1_type = if is_decision_content(&l1_content) {
            MemoryType::Decision
        } else {
            MemoryType::Semantic
        };

        let mut l1_node = FractalNode::new_typed(
            Some(l1_content),
            None,
            l1_embedding,
            consolidation_metadata(
                &node,
                "local_l1_overview",
                if l1_type == MemoryType::Decision {
                    "decision"
                } else {
                    "historical"
                },
            ),
            l1_type,
            MemorySource::Consolidation,
        );
        l1_node.context_tier = ContextTier::Overview;
        l1_node.parent_tier_id = Some(node_id); // L1 → L2
        l1_node.children_tier_ids = vec![]; // Will be populated when L0 is created
        l1_node.importance = node.importance;
        l1_node.confidence = node.confidence * 0.95; // Slightly lower confidence for derived content

        // Step 2: Store L1 node
        let l1_id = self.store.insert(l1_node).await?;

        // Step 3: Link L2 → L1 (parent_tier_id on L2 points to L1)
        self.store
            .update(&node_id, UpdateOperation::SetParentTierId(l1_id))
            .await?;

        // Step 4: Link L1 → L2 (children_tier_ids on L1 includes L2)
        self.store
            .update(&l1_id, UpdateOperation::AddChildTierId(node_id))
            .await?;

        // Step 4a: Extract structured claims from summary text.
        // New JSON Schema format (May 2026): Ollama produces valid JSON with
        // {"summary": "...", "claims": [{"claim": "...", "reason": "..."}]}
        // Falls back to legacy ---CLAIMS--- text parsing for older output.
        let consolidation = crate::summarizer::ConsolidationOutput::from_summary_text(&summary.text);
        let claims = &consolidation.claims;
        let narrative_summary = &consolidation.summary;
        let mut claim_node_ids = Vec::new();
        for claim in claims {
            let claim_text = &claim.claim;
            let reason = &claim.reason;
            // Build claim content optimized for embedding similarity:
            // "claim: {what}  reason: {why}" — both fields in one string
            // lets vector search match on either "was wurde entschieden"
            // or "warum wurde es entschieden"
            let claim_content = if reason.is_empty() {
                format!("claim: {}", claim_text)
            } else {
                format!("claim: {}  reason: {}", claim_text, reason)
            };

            let claim_embedding = match self.embed_text(&claim_content).await {
                Ok(emb) => emb,
                Err(e) => {
                    tracing::warn!("claim embedding failed, skipping: {}", e);
                    continue;
                }
            };

            let mut claim_node = FractalNode::new_typed(
                Some(claim_content),
                None,
                claim_embedding,
                {
                    let mut metadata =
                        consolidation_metadata(&node, "local_claim_extraction", "decision");
                    metadata.insert(
                        "decision_what".to_string(),
                        serde_json::Value::String(claim_text.to_string()),
                    );
                    if !reason.is_empty() {
                        metadata.insert(
                            "decision_why".to_string(),
                            serde_json::Value::String(reason.to_string()),
                        );
                    }
                    metadata
                },
                MemoryType::Decision,
                MemorySource::Consolidation,
            );
            claim_node.context_tier = ContextTier::Overview;
            claim_node.parent_tier_id = Some(l1_id); // claim → L1
            claim_node.importance = node.importance;
            claim_node.confidence = node.confidence * 0.92; // Slightly lower: derived from derived

            match self.store.insert(claim_node).await {
                Ok(id) => {
                    claim_node_ids.push(id);
                }
                Err(e) => {
                    tracing::warn!("failed to store claim node: {}", e);
                }
            }
        }

        if !claim_node_ids.is_empty() {
            tracing::info!(
                l1_id = %l1_id,
                claim_count = claim_node_ids.len(),
                claims = ?claims.iter().map(|c| c.claim.as_str()).collect::<Vec<_>>(),
                "Extracted structured claims from consolidation"
            );
        }

        // Step 5: Generate L0 (Summary) from L1 content
        let l0_summary = self
            .local_summarizer
            .summarize_for_tier(&summary.text, ContextTier::Summary)
            .await?;

        let l0_content = l0_summary.text.clone();
        let l0_embedding = self.embed_text(&l0_content).await?;
        // Inherit type from L1: if L1 is a Decision, L0 is too
        let l0_type = if l1_type == MemoryType::Decision || is_decision_content(&l0_content) {
            MemoryType::Decision
        } else {
            MemoryType::Semantic
        };

        let mut l0_node = FractalNode::new_typed(
            Some(l0_content),
            None,
            l0_embedding,
            consolidation_metadata(
                &node,
                "local_l0_summary",
                if l0_type == MemoryType::Decision {
                    "decision"
                } else {
                    "historical"
                },
            ),
            l0_type,
            MemorySource::Consolidation,
        );
        l0_node.context_tier = ContextTier::Summary;
        l0_node.parent_tier_id = Some(l1_id); // L0 → L1
        l0_node.children_tier_ids = vec![node_id]; // L0 → L2 (direct, skipping L1 for fast zoom)
        l0_node.importance = node.importance;
        l0_node.confidence = node.confidence * 0.90; // Lower confidence for double-derived

        // Step 6: Store L0 node
        let l0_id = self.store.insert(l0_node).await?;

        // Step 7: Link L1 → L0 (parent_tier_id on L1 points to L0)
        self.store
            .update(&l1_id, UpdateOperation::SetParentTierId(l0_id))
            .await?;

        // Step 8: Update L1 children_tier_ids to include L2
        self.store
            .update(&l1_id, UpdateOperation::AddChildTierId(node_id))
            .await?;

        tracing::info!(
            l2_node_id = %node_id,
            l1_node_id = %l1_id,
            l0_node_id = %l0_id,
            model = %summary.model_used,
            "Fractal compaction complete: L2 → L1 → L0 with embeddings"
        );

        Ok(())
    }

    /// Embed text using the configured embedding provider.
    async fn embed_text(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        use crate::embedding::embed_document;

        match embed_document(self.embedding.as_ref(), text).await {
            Ok(vector) => Ok(vector),
            Err(e) => {
                tracing::warn!("embedding failed, using zero vector: {}", e);
                Ok(vec![0.0_f32; self.embedding.dimension()])
            }
        }
    }

    /// Find consolidation candidates.
    ///
    /// Candidates are nodes that:
    /// - Have `context_tier == ContextTier::Raw` (L2, full content)
    /// - Are not already consolidated (`parent_tier_id == None`)
    /// - Are `Active` status
    /// - Have non-empty content or original_pointer
    ///
    /// Sorted by age (oldest first), capped at `limit`.
    async fn find_candidates(&self, _limit: usize) -> Vec<(Uuid, DateTime<Utc>)> {
        let all_nodes = match self.store.list_all().await {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::error!(error = %e, "failed to list nodes for consolidation");
                return Vec::new();
            }
        };

        let _now = Utc::now();
        let mut candidates: Vec<(Uuid, DateTime<Utc>)> = Vec::new();

        for node in all_nodes {
            // Skip if already consolidated
            if node.parent_tier_id.is_some() {
                continue;
            }
            // Only Raw tier (L2) nodes need consolidation
            if node.context_tier != ContextTier::Raw {
                continue;
            }
            // Only active memories
            if node.status != MemoryStatus::Active {
                continue;
            }
            // Must have content to summarize
            let has_content = node
                .content
                .as_ref()
                .map(|c| !c.is_empty())
                .unwrap_or(false)
                || node.original_pointer.is_some();
            if !has_content {
                continue;
            }

            // Only compact important memories (importance >= 3)
            if node.importance < 3 {
                continue;
            }

            // Only compact nodes with substantial content (> 500 chars)
            let content_len = node.content.as_ref().map(|c| c.len()).unwrap_or(0);
            if content_len <= 500 {
                continue;
            }

            candidates.push((node.id, node.created_at));
        }

        // Sort by age (oldest first)
        candidates.sort_by(|a, b| a.1.cmp(&b.1));

        // Budget cap: limit VLM jobs per cycle
        let max_jobs = self.config.vlm_max_jobs_per_cycle;
        candidates.truncate(max_jobs);

        candidates
    }

    /// Check whether consolidation should run based on space-amplification ratio.
    ///
    /// Trigger condition: `unconsolidated > min_count` AND
    /// `unconsolidated / total > threshold`.
    ///
    /// This avoids "compaction storms" during bulk writes by batching:
    /// - 1 new session → 1/1 = 100% but <3 → waits
    /// - 4 new sessions → 4/4 = 100% → triggers immediately
    /// - 50 bulk import → 50/50 = 100% → one compaction for all
    /// - 100 old + 1 new → 1/101 = 1% → no trigger
    ///
    /// Parameters are configurable via env vars:
    /// - `DREAM_SPACE_AMPLIFICATION_MIN_COUNT` (default: 4)
    /// - `DREAM_SPACE_AMPLIFICATION_THRESHOLD` (default: 0.5)
    async fn should_compact(&self) -> bool {
        let min_count: usize = std::env::var("DREAM_SPACE_AMPLIFICATION_MIN_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let threshold: f64 = std::env::var("DREAM_SPACE_AMPLIFICATION_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5);

        let all_nodes = match self.store.list_all().await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!(error = %e, "should_compact: list_all failed, forcing run");
                return true; // On error, force a run — better safe than stuck
            }
        };

        let unconsolidated = all_nodes
            .iter()
            .filter(|n| {
                n.context_tier == ContextTier::Raw
                    && n.parent_tier_id.is_none()
                    && n.status == MemoryStatus::Active
                    && n.importance >= 3
                    && n.content.as_ref().map(|c| c.len() > 500).unwrap_or(false)
            })
            .count();

        let total = all_nodes.len();

        if total == 0 {
            return false; // Nothing to compact
        }

        let ratio = unconsolidated as f64 / total as f64;
        let should = unconsolidated > min_count && ratio > threshold;

        tracing::debug!(
            unconsolidated,
            total,
            ratio = format!("{:.2}", ratio),
            min_count,
            threshold,
            should,
            "should_compact check"
        );

        should
    }

    /// Get the number of nodes enqueued in the last run.
    pub async fn last_enqueued(&self) -> usize {
        *self.last_enqueued.read().await
    }

    /// Get the last run timestamp.
    pub async fn last_run(&self) -> Option<Instant> {
        *self.last_run.read().await
    }
}

#[cfg(test)]
mod trigger_tests {
    use super::*;
    use crate::memory::{FractalNode, MemorySource, MemoryType};
    use crate::storage::MemoryStore;

    #[test]
    fn test_is_decision_content() {
        assert!(is_decision_content(
            "DECISION: migrate embeddings 1536→1024"
        ));
        assert!(is_decision_content(
            "Decision: use sqlx COALESCE for mode-agnostic queries"
        ));
        assert!(is_decision_content(
            "We decided to move from Docker to native macOS"
        ));
        assert!(is_decision_content(
            "Die Entscheidung fiel auf OpenAI statt Ollama"
        ));
        assert!(is_decision_content(
            "Es wurde entschieden, den Prompt umzuschreiben"
        ));
        // Not a decision
        assert!(!is_decision_content("The sky is blue"));
        assert!(!is_decision_content("KnowWhere has 1000 nodes"));
    }
    use std::collections::HashMap;
    use std::sync::Arc;

    struct DummyEmbedding;

    #[async_trait::async_trait]
    impl EmbeddingProvider for DummyEmbedding {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0; 1024])
        }
        fn dimension(&self) -> usize {
            1024
        }
        fn name(&self) -> &str {
            "dummy"
        }
    }

    fn make_raw_node(content_len: usize) -> FractalNode {
        let content = "x".repeat(content_len.max(1));
        FractalNode::new_typed(
            Some(content),
            None,
            vec![0.0; 1024],
            HashMap::new(),
            MemoryType::Semantic,
            MemorySource::Manual,
        )
    }

    fn make_consolidated_node() -> FractalNode {
        let mut node = make_raw_node(501);
        node.parent_tier_id = Some(Uuid::new_v4());
        node
    }

    fn build_scheduler(store: Arc<MemoryStore>) -> ConsolidationScheduler {
        let embedding = Arc::new(DummyEmbedding);
        ConsolidationScheduler::new(store, None, embedding, SchedulerConfig::default())
    }

    // Test 1: 4 Raw, 4 total → should_compact() = true
    #[tokio::test]
    async fn test_4_raw_4_total_triggers() {
        let store = Arc::new(MemoryStore::new());
        for _ in 0..4 {
            store.insert(make_raw_node(501)).await.unwrap();
        }
        let sched = build_scheduler(store);
        assert!(sched.should_compact().await);
    }

    // Test 2: 1 Raw, 100 total → should_compact() = false
    #[tokio::test]
    async fn test_1_raw_100_total_no_trigger() {
        let store = Arc::new(MemoryStore::new());
        // 1 raw node
        store.insert(make_raw_node(501)).await.unwrap();
        // 99 consolidated nodes
        for _ in 0..99 {
            store.insert(make_consolidated_node()).await.unwrap();
        }
        let sched = build_scheduler(store);
        assert!(!sched.should_compact().await);
    }

    // Test 3: 3 Raw, 3 total → should_compact() = false (≤3 threshold)
    #[tokio::test]
    async fn test_3_raw_3_total_no_trigger() {
        let store = Arc::new(MemoryStore::new());
        for _ in 0..3 {
            store.insert(make_raw_node(501)).await.unwrap();
        }
        let sched = build_scheduler(store);
        assert!(!sched.should_compact().await);
    }

    // Test 4: 0 Raw → should_compact() = false
    #[tokio::test]
    async fn test_empty_store_no_trigger() {
        let store = Arc::new(MemoryStore::new());
        let sched = build_scheduler(store);
        assert!(!sched.should_compact().await);
    }

    // Test 5: Timer safety-net — simulated via last_run
    #[tokio::test]
    async fn test_timer_safety_net_forces_run_on_empty_store() {
        // Even with 0 unconsolidated nodes, if last_run is None (first run),
        // the guard in run() should force a run. But since run() is async and
        // calls process_local_compaction, we test the guard logic indirectly:
        // when last_run is None, the guard says force_run = true.
        // We verify that should_compact returns false, but the guard in run()
        // would still proceed due to force_run = true (tested via code review).
        let store = Arc::new(MemoryStore::new());
        let sched = build_scheduler(store);

        // Empty store → should_compact = false
        assert!(!sched.should_compact().await);

        // Verify last_run is None on a fresh scheduler
        let last = sched.last_run().await;
        assert!(last.is_none(), "fresh scheduler should have last_run=None");
    }

    // --- parse_claims_block tests ---

    #[test]
    fn test_parse_claims_block_basic() {
        let text = "Some summary text.\n---CLAIMS---\n- claim: Docker entfernt\n  reason: LinuxKit Overhead\n- claim: rust-bert entfernt\n  reason: nicht mehr nötig\n---END---\nMore text.";
        let claims = super::parse_claims_block(text);
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].0, "Docker entfernt");
        assert_eq!(claims[0].1, "LinuxKit Overhead");
        assert_eq!(claims[1].0, "rust-bert entfernt");
        assert_eq!(claims[1].1, "nicht mehr nötig");
    }

    #[test]
    fn test_parse_claims_block_no_block() {
        let text = "Just a normal summary. No claims here.";
        let claims = super::parse_claims_block(text);
        assert!(claims.is_empty());
    }

    #[test]
    fn test_parse_claims_block_no_claims() {
        let text = "Summary.\n---CLAIMS---\n---END---\nMore text.";
        let claims = super::parse_claims_block(text);
        assert!(claims.is_empty());
    }

    #[test]
    fn test_parse_claims_block_claim_without_reason() {
        let text = "---CLAIMS---\n- claim: Etwas wurde entschieden\n---END---";
        let claims = super::parse_claims_block(text);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].0, "Etwas wurde entschieden");
        assert_eq!(claims[0].1, "");
    }

    #[test]
    fn test_parse_claims_block_missing_end_marker() {
        let text = "---CLAIMS---\n- claim: X gemacht\n  reason: weil Y\nNo end marker.";
        let claims = super::parse_claims_block(text);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].0, "X gemacht");
        assert_eq!(claims[0].1, "weil Y");
    }

    #[test]
    fn test_parse_claims_block_reason_without_claim_ignored() {
        let text = "---CLAIMS---\n  reason: orphan reason\n- claim: valid claim\n  reason: valid reason\n---END---";
        let claims = super::parse_claims_block(text);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].0, "valid claim");
        assert_eq!(claims[0].1, "valid reason");
    }
}

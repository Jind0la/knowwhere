//! VLM Worker — Async Background Worker for Contextthread Summarization
//!
//! Implements a 3-stage fallback hierarchy for LLM-based summarization:
//! 1. Primary:   GPT-5-nano Batch (gpt-5-nano-2025-08-07) — $0.025/1M input
//! 2. Fallback:  GPT-4o-mini Batch                       — $0.075/1M input
//! 3. Failover:  Grok-4-1-fast Batch                    — $0.20/1M input
//!
//! Design goals:
//! - Non-blocking: UI/retrieval must never block on VLM calls
//! - Fault-tolerant: Automatic fallback on 429/500/timeout
//! - Config-driven: API keys and model choices from .env
//! - Storage-integrated: Summary nodes written back as L1/L2 FractalNodes

pub use worker::VlmWorker;

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::embedding::{embed_document, EmbeddingProvider};
use crate::memory::types::{ContextTier, MemorySource, MemoryType};
use crate::memory::FractalNode;
use crate::storage::{StorageBackend, UpdateOperation};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public API — Enqueue a summarization job
// ---------------------------------------------------------------------------

/// VLM Worker handle — safe to clone and share across tasks.
#[derive(Clone)]
pub struct VlmWorkerHandle {
    job_tx: mpsc::Sender<VlmJob>,
    status: Arc<RwLock<VlmWorkerStatus>>,
}

impl VlmWorkerHandle {
    /// Enqueue a summarization job. Returns immediately (non-blocking).
    pub async fn enqueue(&self, job: VlmJob) -> Result<()> {
        self.job_tx.send(job).await?;
        Ok(())
    }

    /// Get current queue depth and worker status.
    pub async fn status(&self) -> VlmWorkerStatus {
        self.status.read().await.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct VlmWorkerStatus {
    pub queue_depth: usize,
    pub jobs_processed: u64,
    pub jobs_failed: u64,
    pub last_model_used: Option<String>,
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Job definition
// ---------------------------------------------------------------------------

/// A single summarization job in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VlmJob {
    /// Unique job ID.
    pub id: Uuid,
    /// Target memory node IDs to summarize.
    pub node_ids: Vec<Uuid>,
    /// What kind of summary to produce.
    pub context: SummaryContext,
    /// Priority (higher = processed first). Default 5.
    pub priority: u8,
    /// Unix timestamp when job was created.
    pub created_at: i64,
}

impl VlmJob {
    pub fn new(node_ids: Vec<Uuid>, context: SummaryContext) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_ids,
            context,
            priority: 5,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
}

/// What kind of summary to generate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SummaryContext {
    #[default]
    /// L0 — One-sentence summary (~20–50 tokens).
    Summary,
    /// L1 — Paragraph overview (~100–300 tokens).
    Overview,
    /// L2 — Detailed summary (~500–1000 tokens, preserves key facts).
    Detailed,
}

impl SummaryContext {
    /// Target token count for this context level.
    pub fn target_tokens(&self) -> usize {
        match self {
            SummaryContext::Summary => 50,
            SummaryContext::Overview => 200,
            SummaryContext::Detailed => 700,
        }
    }

    /// Which FractalNode content field to fill.
    pub fn target_tier(&self) -> ContextTier {
        match self {
            SummaryContext::Summary => ContextTier::Summary,
            SummaryContext::Overview => ContextTier::Overview,
            SummaryContext::Detailed => ContextTier::Raw,
        }
    }

    /// System prompt directive for this context level.
    pub fn system_directive(&self) -> &'static str {
        match self {
            SummaryContext::Summary => {
                "Compress memory for later retrieval. Output one sentence (≤20 words) capturing the single most retrievable fact. Preserve: key facts, entities, decisions, timestamps. No preamble. No filler."
            }
            SummaryContext::Overview => {
                "Compress memory for later retrieval. Preserve: key facts, named entities, decisions, timestamps. Output: 2–3 dense sentences. No preamble. No filler. No commentary."
            }
            SummaryContext::Detailed => {
                "You are a detailed summarizer. Write a thorough but concise summary (300–600 words) \
                that preserves all important facts, decisions, and relationships from the input. \
                Structure with light paragraph breaks. No preamble, no commentary."
            }
        }
    }

    /// User prompt template for this context level.
    pub fn prompt_template(&self) -> &'static str {
        match self {
            SummaryContext::Summary => {
                "Core fact:\n\n{content}"
            }
            SummaryContext::Overview => {
                "Compress for retrieval:\n\n{content}"
            }
            SummaryContext::Detailed => {
                "Create a detailed summary of the following, preserving all key facts and decisions:\n\n{content}"
            }
        }
    }
}

impl std::fmt::Display for SummaryContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SummaryContext::Summary => write!(f, "summary"),
            SummaryContext::Overview => write!(f, "overview"),
            SummaryContext::Detailed => write!(f, "detailed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Model configuration
// ---------------------------------------------------------------------------

/// Available VLM models in fallback order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VlmModel {
    /// GPT-5-nano Batch — $0.025/1M input, ~50ms latency, no temperature param
    Gpt5Nano,
    /// GPT-4o-mini Batch — $0.075/1M input, ~100ms latency
    Gpt4oMini,
    /// Grok-4-1-fast Batch — $0.20/1M input, ~150ms latency, auto-caching
    Grok4Fast,
    /// Ollama local LLM (e.g., llama3.2) — no API key needed
    Ollama,
}

impl VlmModel {
    /// All models in fallback order.
    pub fn fallback_chain() -> [Self; 4] {
        [
            Self::Gpt5Nano,
            Self::Gpt4oMini,
            Self::Grok4Fast,
            Self::Ollama,
        ]
    }

    /// API model identifier string.
    /// For Ollama, this is loaded from OLLAMA_VLM_MODEL env var at runtime.
    pub fn model_id(&self) -> &'static str {
        match self {
            VlmModel::Gpt5Nano => "gpt-5-nano-2025-08-07",
            VlmModel::Gpt4oMini => "gpt-4o-mini-2024-07-18",
            VlmModel::Grok4Fast => "grok-4-1-fast",
            VlmModel::Ollama => "llama3.2", // Default, overridden by OLLAMA_VLM_MODEL
        }
    }

    /// Human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            VlmModel::Gpt5Nano => "GPT-5-nano Batch",
            VlmModel::Gpt4oMini => "GPT-4o-mini Batch",
            VlmModel::Grok4Fast => "Grok-4-1-fast Batch",
            VlmModel::Ollama => "Ollama (local)",
        }
    }

    /// Base URL for the API.
    /// For Ollama, this is loaded from OLLAMA_URL env var at runtime (default: http://localhost:11434).
    pub fn base_url(&self) -> &'static str {
        match self {
            VlmModel::Gpt5Nano | VlmModel::Gpt4oMini => "https://api.openai.com",
            VlmModel::Grok4Fast => "https://api.x.ai",
            VlmModel::Ollama => "http://localhost:11434", // Default, overridden by OLLAMA_URL
        }
    }

    /// Timeout for this model in seconds.
    pub fn timeout_secs(&self) -> u64 {
        match self {
            VlmModel::Gpt5Nano => 15,
            VlmModel::Gpt4oMini => 20,
            VlmModel::Grok4Fast => 30,
            VlmModel::Ollama => 60, // Local models are slower
        }
    }
}

/// VLM configuration loaded from environment.
#[derive(Debug, Clone)]
pub struct VlmConfig {
    pub openai_api_key: Option<String>,
    pub grok_api_key: Option<String>,
    /// Ollama base URL (e.g., http://localhost:11434)
    pub ollama_url: Option<String>,
    /// Ollama VLM model for chat completions (e.g., llama3.2, mistral)
    pub ollama_vlm_model: Option<String>,
}

impl VlmConfig {
    /// Load from environment variables.
    pub fn from_env() -> Self {
        Self {
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            grok_api_key: std::env::var("GROK_API_KEY").ok(),
            ollama_url: std::env::var("OLLAMA_URL").ok(),
            ollama_vlm_model: std::env::var("OLLAMA_VLM_MODEL").ok(),
        }
    }

    /// Whether we have at least one API key configured.
    pub fn is_configured(&self) -> bool {
        self.openai_api_key.is_some()
            || self.grok_api_key.is_some()
            || self.ollama_vlm_model.is_some()
    }
}

// ---------------------------------------------------------------------------
// VLM Client — Handles HTTP calls to OpenAI/xAI responses endpoint
// ---------------------------------------------------------------------------

/// Errors that can occur during a VLM call.
#[derive(Debug)]
pub enum VlmError {
    Http(reqwest::Error),
    Api(String),
    NoApiKey(String),
    RateLimited(&'static str),
    Timeout(u64, &'static str),
    NoResponse(&'static str),
    AllModelsFailed(Vec<String>),
}

impl std::fmt::Display for VlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VlmError::Http(e) => write!(f, "HTTP request failed: {}", e),
            VlmError::Api(s) => write!(f, "API error: {}", s),
            VlmError::NoApiKey(m) => write!(f, "API key not configured for {}", m),
            VlmError::RateLimited(m) => write!(
                f,
                "Rate limited (429) on {}, no more fallbacks available",
                m
            ),
            VlmError::Timeout(s, m) => write!(f, "Timeout after {}s on {}", s, m),
            VlmError::NoResponse(m) => write!(f, "No valid response from {}", m),
            VlmError::AllModelsFailed(errs) => write!(f, "All models failed: {}", errs.join("; ")),
        }
    }
}

impl std::error::Error for VlmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VlmError::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for VlmError {
    fn from(e: reqwest::Error) -> Self {
        VlmError::Http(e)
    }
}

/// Result of a successful VLM summarization call.
#[derive(Debug)]
pub struct VlmSummary {
    pub text: String,
    pub model_used: VlmModel,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

/// VLM HTTP client for the OpenAI `/v1/responses` endpoint.
pub struct VlmClient {
    http: reqwest::Client,
}

impl VlmClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("knowwhere-server/0.1")
                .build()
                .expect("reqwest client must build"),
        }
    }

    /// Call the VLM with a prompt, trying each model in the fallback chain.
    pub async fn summarize_with_fallback(
        &self,
        prompt: &str,
        context: SummaryContext,
        config: &VlmConfig,
    ) -> Result<VlmSummary, VlmError> {
        let mut errors = Vec::new();

        for model in VlmModel::fallback_chain() {
            match self
                .call_model(prompt, context.clone(), model, config)
                .await
            {
                Ok(summary) => return Ok(summary),
                Err(e) => {
                    let msg = format!("{}: {}", model.name(), e);
                    tracing::warn!("VLM call failed, trying next model: {}", msg);
                    errors.push(msg);

                    // Don't retry the same model
                    match &e {
                        VlmError::RateLimited(_) | VlmError::Timeout(..) => {}
                        _ => {}
                    }
                }
            }
        }

        Err(VlmError::AllModelsFailed(errors))
    }

    /// Call a specific model. Returns error on 429/500/timeout.
    async fn call_model(
        &self,
        prompt: &str,
        context: SummaryContext,
        model: VlmModel,
        config: &VlmConfig,
    ) -> Result<VlmSummary, VlmError> {
        // For Ollama, no API key needed — use model from config or default
        let ollama_model = config
            .ollama_vlm_model
            .clone()
            .unwrap_or_else(|| "llama3.2".to_string());
        let ollama_url = config
            .ollama_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let api_key: &str = match model {
            VlmModel::Gpt5Nano | VlmModel::Gpt4oMini => config
                .openai_api_key
                .as_ref()
                .ok_or_else(|| VlmError::NoApiKey(model.name().to_string()))?,
            VlmModel::Grok4Fast => config
                .grok_api_key
                .as_ref()
                .ok_or_else(|| VlmError::NoApiKey(model.name().to_string()))?,
            VlmModel::Ollama => "", // No API key needed for local Ollama
        };

        let url = match model {
            VlmModel::Gpt5Nano | VlmModel::Gpt4oMini => {
                format!("{}/v1/responses", model.base_url())
            }
            VlmModel::Grok4Fast => {
                format!("{}/v1/responses", model.base_url())
            }
            VlmModel::Ollama => {
                format!("{}/api/chat", ollama_url)
            }
        };

        // Build request body — no temperature for gpt-5-nano (not supported)
        let system_msg = context.system_directive();
        let user_prompt = context.prompt_template().replace("{content}", prompt);

        // Handle Ollama separately — uses /api/chat with different format
        if model == VlmModel::Ollama {
            let ollama_body = serde_json::json!({
                "model": ollama_model,
                "messages": [
                    {"role": "system", "content": system_msg},
                    {"role": "user", "content": user_prompt},
                ],
                "stream": false,
            });

            let request = self
                .http
                .post(&url)
                .timeout(std::time::Duration::from_secs(model.timeout_secs()))
                .json(&ollama_body);

            let resp = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    if e.is_timeout() {
                        return Err(VlmError::Timeout(model.timeout_secs(), model.name()));
                    }
                    return Err(VlmError::Http(e));
                }
            };

            let status = resp.status();

            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                let msg = format!("HTTP {status}: {body_text}");
                if status.is_server_error() {
                    return Err(VlmError::Api(msg));
                }
                return Err(VlmError::Api(msg));
            }

            #[derive(serde::Deserialize)]
            struct OllamaResponse {
                message: OllamaMessage,
                #[serde(default)]
                prompt_eval_count: Option<u64>,
                #[serde(default)]
                eval_count: Option<u64>,
            }

            #[derive(serde::Deserialize)]
            struct OllamaMessage {
                content: String,
            }

            let ollama_resp: OllamaResponse = match resp.json().await {
                Ok(b) => b,
                Err(e) => return Err(VlmError::Http(e)),
            };

            let text = ollama_resp.message.content;
            if text.is_empty() {
                return Err(VlmError::NoResponse(model.name()));
            }

            let input_tokens = ollama_resp.prompt_eval_count.unwrap_or(0) as u32;
            let output_tokens = ollama_resp.eval_count.unwrap_or(0) as u32;

            tracing::debug!(
                model = %model.name(),
                input_tokens,
                output_tokens,
                text_len = text.len(),
                "VLM summarization successful (Ollama)"
            );

            return Ok(VlmSummary {
                text,
                model_used: model,
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
            });
        }

        // OpenAI / xAI format (gpt-5-nano, gpt-4o-mini, grok-4-fast)
        let mut body = serde_json::json!({
            "model": model.model_id(),
            "input": [
                {"role": "system", "content": system_msg},
                {"role": "user", "content": user_prompt},
            ],
            "max_output_tokens": context.target_tokens(),
        });

        // Only add temperature for models that support it (not gpt-5-nano)
        if model != VlmModel::Gpt5Nano {
            body["temperature"] = serde_json::json!(0.3);
        }

        let request = self
            .http
            .post(&url)
            .bearer_auth(api_key)
            .timeout(std::time::Duration::from_secs(model.timeout_secs()))
            .json(&body);

        let resp = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    return Err(VlmError::Timeout(model.timeout_secs(), model.name()));
                }
                return Err(VlmError::Http(e));
            }
        };

        let status = resp.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(VlmError::RateLimited(model.name()));
        }

        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let msg = format!("HTTP {status}: {body_text}");
            // Retry on 500-level errors
            if status.is_server_error() {
                return Err(VlmError::Api(msg));
            }
            return Err(VlmError::Api(msg));
        }

        #[derive(serde::Deserialize)]
        struct ResponseBody {
            output: Vec<serde_json::Value>,
            #[serde(default)]
            usage: Option<serde_json::Value>,
        }

        let body: ResponseBody = match resp.json().await {
            Ok(b) => b,
            Err(e) => return Err(VlmError::Http(e)),
        };

        // Extract text from output
        let text = body
            .output
            .iter()
            .find(|o| o.get("type").and_then(|t| t.as_str()) == Some("message"))
            .and_then(|o| o.get("content"))
            .and_then(|c| c.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|item| item.get("type").and_then(|t| t.as_str()) == Some("output_text"))
            })
            .and_then(|item| item.get("text").and_then(|t| t.as_str()))
            .map(str::to_string)
            .unwrap_or_default();

        if text.is_empty() {
            return Err(VlmError::NoResponse(model.name()));
        }

        let (input_tokens, output_tokens) = body
            .usage
            .as_ref()
            .and_then(|u| {
                let obj = u.as_object()?;
                let i = obj.get("input_tokens")?.as_u64()? as u32;
                let o = obj.get("output_tokens")?.as_u64()? as u32;
                Some((i, o))
            })
            .unwrap_or((0, 0));

        tracing::debug!(
            model = %model.name(),
            input_tokens,
            output_tokens,
            text_len = text.len(),
            "VLM summarization successful"
        );

        Ok(VlmSummary {
            text,
            model_used: model,
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
        })
    }
}

impl Default for VlmClient {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Worker implementation
// ---------------------------------------------------------------------------

mod worker {
    use super::*;

    /// Priority queue of VlmJobs (max-heap via reverse ordering).
    type JobQueue = std::collections::BinaryHeap<VlmJob>;

    /// Full VLM background worker (not Clone).
    pub struct VlmWorker {
        store: Arc<dyn StorageBackend>,
        embedding: Arc<dyn EmbeddingProvider>,
        client: VlmClient,
        config: VlmConfig,
        job_rx: mpsc::Receiver<VlmJob>,
        queue: Arc<RwLock<JobQueue>>,
        stats: Arc<RwLock<VlmWorkerStatus>>,
    }

    impl VlmWorker {
        pub fn new(
            store: Arc<dyn StorageBackend>,
            embedding: Arc<dyn EmbeddingProvider>,
            client: VlmClient,
            config: VlmConfig,
            job_rx: mpsc::Receiver<VlmJob>,
        ) -> Self {
            Self {
                store,
                embedding,
                client,
                config,
                job_rx,
                queue: Arc::new(RwLock::new(JobQueue::new())),
                stats: Arc::new(RwLock::new(VlmWorkerStatus {
                    queue_depth: 0,
                    jobs_processed: 0,
                    jobs_failed: 0,
                    last_model_used: None,
                    active: true,
                })),
            }
        }

        /// Build a handle and start the worker in the background.
        pub fn spawn(
            store: Arc<dyn StorageBackend>,
            embedding: Arc<dyn EmbeddingProvider>,
            config: VlmConfig,
        ) -> (VlmWorkerHandle, tokio::task::JoinHandle<()>) {
            let (job_tx, job_rx) = mpsc::channel(256);

            let worker = Self::new(
                store.clone(),
                embedding.clone(),
                VlmClient::new(),
                config.clone(),
                job_rx,
            );

            let handle = VlmWorkerHandle {
                job_tx: job_tx.clone(),
                status: worker.stats.clone(),
            };

            let join_handle = tokio::spawn(worker.run());

            tracing::info!("VLM worker started");

            (handle, join_handle)
        }

        /// Main worker loop — processes jobs from queue.
        async fn run(mut self) {
            loop {
                // Receive from channel (with a short timeout so we can also check the queue)
                let job =
                    tokio::time::timeout(std::time::Duration::from_millis(50), self.job_rx.recv())
                        .await;

                match job {
                    Ok(Some(job)) => self.push_job(job).await,
                    // Channel closed — drain queue then exit
                    Ok(None) => {
                        tracing::info!("VLM job channel closed, draining queue");
                        while self.queue_depth().await > 0 {
                            self.process_queue_tick().await;
                        }
                        tracing::info!("VLM worker shut down");
                        break;
                    }
                    // Timeout — no job received, just check queue
                    Err(_) => {}
                }

                // Process one job from the queue
                if self.queue_depth().await > 0 {
                    self.process_queue_tick().await;
                }
            }
        }

        async fn queue_depth(&self) -> usize {
            self.queue.read().await.len()
        }

        async fn push_job(&self, job: VlmJob) {
            // Push to queue
            {
                let mut q = self.queue.write().await;
                q.push(job);
            }
            // Update stats
            let depth = self.queue.read().await.len();
            let mut s = self.stats.write().await;
            s.queue_depth = depth;
        }

        /// Called periodically to drain and process one job.
        async fn process_queue_tick(&self) {
            let job = {
                let mut q = self.queue.write().await;
                q.pop()
            };

            let Some(job) = job else {
                return;
            };

            {
                let mut s = self.stats.write().await;
                s.queue_depth = self.queue.read().await.len();
            }

            tracing::info!(job_id = %job.id, nodes = job.node_ids.len(), context = %job.context, "processing VLM job");

            match self.process_job(job).await {
                Ok(()) => {
                    let mut s = self.stats.write().await;
                    s.jobs_processed += 1;
                }
                Err(e) => {
                    tracing::error!("VLM job failed: {}", e);
                    let mut s = self.stats.write().await;
                    s.jobs_failed += 1;
                }
            }

            {
                let mut s = self.stats.write().await;
                s.queue_depth = self.queue.read().await.len();
            }
        }

        /// Process a single summarization job.
        async fn process_job(&self, job: VlmJob) -> Result<()> {
            // Step 1: Fetch source nodes from store
            let mut raw_contents: Vec<(Uuid, String)> = Vec::new();
            for node_id in &job.node_ids {
                if let Some(node) = self.store.get(node_id).await? {
                    if let Some(content) = &node.content {
                        if !content.is_empty() {
                            raw_contents.push((*node_id, content.clone()));
                        }
                    } else if let Some(pointer) = &node.original_pointer {
                        raw_contents.push((*node_id, pointer.clone()));
                    }
                }
            }

            if raw_contents.is_empty() {
                tracing::warn!(job_id = %job.id, "no content found for VLM job");
                return Ok(());
            }

            // Step 2: Combine content for the prompt
            let combined: String = raw_contents
                .iter()
                .enumerate()
                .map(|(i, (_, c))| {
                    if c.len() > 4000 {
                        format!("[Item {}]: {}...", i + 1, &c[..4000])
                    } else {
                        format!("[Item {}]: {}", i + 1, c)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            // Step 3: Call VLM with fallback
            let summary_result = self
                .client
                .summarize_with_fallback(&combined, job.context, &self.config)
                .await;

            let summary_text = match summary_result {
                Ok(s) => {
                    let mut stats = self.stats.write().await;
                    stats.last_model_used = Some(s.model_used.name().to_string());
                    tracing::info!(
                        job_id = %job.id,
                        model = %s.model_used.name(),
                        input_tokens = s.input_tokens.unwrap_or(0),
                        output_tokens = s.output_tokens.unwrap_or(0),
                        "VLM summarization complete"
                    );
                    s.text
                }
                Err(e) => {
                    // All models failed — use truncation fallback
                    tracing::warn!(job_id = %job.id, "VLM unavailable, using truncation fallback: {}", e);
                    Self::truncation_fallback_text(&combined, &job.context)
                }
            };

            // Step 4: Embed the summary
            let summary_vector = embed_document(self.embedding.as_ref(), &summary_text)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to embed summary, using zero vector: {}", e);
                    vec![0.0_f32; self.embedding.dimension()]
                });

            // Step 5: Build the summary node
            let tier = job.context.target_tier();
            let memory_type = MemoryType::Semantic;
            let source = MemorySource::Consolidation;

            let mut metadata = HashMap::new();
            metadata.insert(
                "vlm_job_id".to_string(),
                serde_json::json!(job.id.to_string()),
            );
            metadata.insert(
                "source_node_ids".to_string(),
                serde_json::json!(job
                    .node_ids
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()),
            );
            metadata.insert(
                "context_level".to_string(),
                serde_json::json!(job.context.to_string()),
            );

            let mut summary_node = FractalNode::new_typed(
                Some(summary_text.clone()),
                None,
                summary_vector,
                metadata,
                memory_type,
                source,
            );
            summary_node.context_tier = tier;
            summary_node.set_metadata_text(FractalNode::DERIVATION_KEY, "system_summary");
            summary_node.set_metadata_text(FractalNode::TRUST_TIER_KEY, FractalNode::TRUST_DERIVED);

            // Step 6: Store the summary node
            let summary_id = self.store.insert(summary_node).await?;

            // Step 7: Update source nodes with parent_tier_id pointing to summary
            let mut updates = 0;
            for node_id in &job.node_ids {
                if let Err(e) = self
                    .store
                    .update(node_id, UpdateOperation::SetParentTierId(summary_id))
                    .await
                {
                    tracing::warn!(node_id = %node_id, "failed to set parent_tier_id: {}", e);
                } else {
                    updates += 1;
                }
            }

            tracing::info!(
                job_id = %job.id,
                summary_node_id = %summary_id,
                source_nodes_updated = updates,
                "VLM job complete — summary node stored"
            );

            Ok(())
        }

        /// Fallback text compression when VLM is unavailable.
        /// Uses token-count limits from SummaryContext::target_tokens().
        pub(crate) fn truncation_fallback_text(
            combined_content: &str,
            context: &SummaryContext,
        ) -> String {
            let limit = context.target_tokens();
            // Rough: ~4 chars per token
            let char_limit = limit * 4;

            if combined_content.len() <= char_limit {
                return combined_content.to_string();
            }

            // Find a good break point (sentence or clause boundary)
            let truncated = &combined_content[..char_limit];

            // Try to break at sentence end, clause, or comma
            if let Some(pos) = truncated.rfind(['.', '!', '?', ';', ',', '\n']) {
                let pos = if truncated.chars().nth(pos) == Some(',') && pos > char_limit / 2 {
                    // Prefer sentence end over mid-clause comma
                    truncated[..pos].rfind(['.', '!', '?']).unwrap_or(pos)
                } else {
                    pos
                };
                format!("{}...", truncated[..pos].trim())
            } else {
                format!("{}...", truncated.trim())
            }
        }
    }

    // Make VlmJob usable in BinaryHeap (reverse priority = max-heap behavior)
    impl PartialEq for VlmJob {
        fn eq(&self, other: &Self) -> bool {
            self.priority == other.priority
        }
    }

    impl Eq for VlmJob {}

    impl PartialOrd for VlmJob {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for VlmJob {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            // Higher priority first; for equal priority, earlier creation time first
            other
                .priority
                .cmp(&self.priority)
                .then_with(|| self.created_at.cmp(&other.created_at))
        }
    }
}

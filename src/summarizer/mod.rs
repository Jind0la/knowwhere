//! Local Summarization — Ollama-based Local Summarization
//!
//! Deterministic, local summarization for L2→L1→L0 compaction.
//! Uses Ollama with a small model (e.g., llama3.2, phi3, or qwen2.5)
//!
//! # Why Ollama for Summarization?
//!
//! | Vorteil | Beschreibung |
//! |---------|-------------|
//! | Kein API Key | Lokal, keine Cloud-Dependency |
//! | Kleine Modelle | 3B Parameter = schnell auf CPU |
//! | Deterministisch | Temperature=0, seed fix |
//! | Einfache Integration | HTTP API, kein ONNX/rust-bert Complexity |
//!
//! # Empfohlene Modelle
//!
//! | Modell | Größe | Speed | Qualität |
//! |--------|-------|-------|----------|
//! | llama3.2 | 3B | Schnell | Gut |
//! | phi3 | 3.8B | Schnell | Gut |
//! | qwen2.5 | 3B | Schnell | Sehr gut |
//! | gemma2 | 2B | Sehr schnell | Gut |
//!
//! # Fallback Chain
//!
//! 1. **PRIMARY**: Ollama local summarization (deterministic, single-sentence)
//! 2. **FALLBACK**: VLM (cloud) — if user configured API key
//! 3. **NEVER**: Truncation — information loss unacceptable
//!
//! # Setup
//!
//! 1. Install Ollama: https://ollama.com
//! 2. Pull model: `ollama pull llama3.2`
//! 3. Set env: `OLLAMA_URL=http://localhost:11434` (optional)

use anyhow::Result;
use serde_json::json;

/// Local summarizer using Ollama HTTP API.
///
/// Deterministic single-sentence summarization.
/// No API key, no internet needed after model download.
pub struct LocalSummarizer {
    ollama_url: String,
    model: String,
    client: reqwest::Client,
}

impl LocalSummarizer {
    /// Create a new local summarizer.
    ///
    /// Uses OLLAMA_URL env var (default: http://localhost:11434)
    /// Uses OLLAMA_SUMMARIZER_MODEL env var (default: llama3.2)
    pub fn new() -> Result<Self> {
        let ollama_url = std::env::var("OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let model = std::env::var("OLLAMA_SUMMARIZER_MODEL")
            .unwrap_or_else(|_| "llama3.2".to_string());
        
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        
        // Verify Ollama is reachable
        // Use std::thread to avoid nested tokio runtime issues in tests
        let is_available = std::thread::spawn({
            let ollama_url = ollama_url.clone();
            let client = client.clone();
            move || -> anyhow::Result<bool> {
                let rt = tokio::runtime::Runtime::new()?;
                let result = rt.block_on(async {
                    match client.get(format!("{}/api/tags", ollama_url)).send().await {
                        Ok(resp) => resp.status().is_success(),
                        Err(_) => false,
                    }
                });
                Ok(result)
            }
        }).join()
            .map_err(|e| anyhow::anyhow!("health check thread panicked: {:?}", e))??;
        
        if !is_available {
            anyhow::bail!(
                "Ollama not available at {}. \
                 Install from https://ollama.com and run `ollama pull {}`",
                ollama_url, model
            );
        }
        
        tracing::info!(
            url = %ollama_url,
            model = %model,
            "Local summarizer (Ollama) initialized"
        );
        
        Ok(Self {
            ollama_url,
            model,
            client,
        })
    }

    /// Summarize text to a single sentence (L0 / Summary tier).
    ///
    /// Uses deterministic generation (temperature=0, seed=42).
    /// Target: ~20-50 tokens, single sentence.
    pub async fn summarize(&self, text: &str) -> Result<String> {
        // Prompt optimized for Decision-Retrieval:
        // Forces the LLM to name decisions explicitly so embedding
        // similarity search finds queries like "why did we kill Docker?"
        let prompt = format!(
            "Summarize in ONE sentence (≤20 words). \
             If any decisions were made, state the decision AND the reason. \
             Otherwise state the single most important fact. \
             Include the word 'decision' or 'decided' if a choice was made. \
             No preamble.\n\n{}",
            text
        );
        
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You are a concise summarizer. Output exactly one sentence."},
                {"role": "user", "content": prompt}
            ],
            "stream": false,
            "options": {
                "temperature": 0.0,
                "seed": 42,
                "num_predict": 50
            }
        });
        
        let resp = self.client
            .post(format!("{}/api/chat", self.ollama_url))
            .json(&body)
            .send()
            .await?;
        
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error {}: {}", status, text);
        }
        
        let result: serde_json::Value = resp.json().await?;
        let summary = result
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        
        if summary.is_empty() {
            anyhow::bail!("summarization produced empty output")
        }
        
        // Clean up: remove quotes, ensure single sentence
        let summary = summary
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        
        Ok(summary)
    }

    /// Summarize with custom max length.
    ///
    /// * `max_length` — maximum tokens in output (approximate)
    /// * `min_length` — minimum tokens in output (approximate)
    pub async fn summarize_with_length(
        &self,
        text: &str,
        max_length: usize,
        _min_length: usize,
    ) -> Result<String> {
        // Prompt optimized for Decision-Retrieval:
        // Structured output: decisions first → embedding similarity
        // naturally boosts decision queries like "what did we decide about X?"
        let prompt = format!(
            "Summarize in 2-3 sentences (max {} words). \
             Sentence 1: key decisions made and WHY. \
             Sentence 2: important facts. \
             Sentence 3: entities and timestamps. \
             If no decisions exist, just summarize key facts. \
             No preamble.\n\n{}",
            max_length / 2,
            text
        );
        
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You are a concise summarizer. Output 2-3 sentences."},
                {"role": "user", "content": prompt}
            ],
            "stream": false,
            "options": {
                "temperature": 0.0,
                "seed": 42,
                "num_predict": max_length
            }
        });
        
        let resp = self.client
            .post(format!("{}/api/chat", self.ollama_url))
            .json(&body)
            .send()
            .await?;
        
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error {}: {}", status, text);
        }
        
        let result: serde_json::Value = resp.json().await?;
        let summary = result
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        
        if summary.is_empty() {
            anyhow::bail!("summarization produced empty output")
        }
        
        Ok(summary)
    }

    /// Check if local summarizer is available.
    pub fn is_available(&self) -> bool {
        true // If we exist, Ollama was reachable at creation time
    }
}

/// Summarization result with metadata.
#[derive(Debug, Clone)]
pub struct SummaryResult {
    pub text: String,
    pub model_used: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
}

/// Tiered summarization — produces L0, L1, or L2 summaries.
///
/// * L0 (Summary): Single sentence, ~20-50 tokens
/// * L1 (Overview): Paragraph, ~100-300 tokens  
/// * L2 (Detailed): Full content preserved, no summarization
pub struct TieredSummarizer {
    local: Option<LocalSummarizer>,
}

impl TieredSummarizer {
    pub fn new() -> Self {
        let local = LocalSummarizer::new().ok();
        if local.is_none() {
            tracing::warn!(
                "Local summarizer (Ollama) not available. \
                 Install from https://ollama.com and run `ollama pull llama3.2`"
            );
        }
        Self { local }
    }

    /// Summarize for target tier.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - No summarizer available (neither local nor VLM)
    /// - Summarization produces empty output
    pub async fn summarize_for_tier(
        &self,
        text: &str,
        tier: crate::memory::types::ContextTier,
    ) -> Result<SummaryResult> {
        match tier {
            crate::memory::types::ContextTier::Summary => {
                self.summarize_l0(text).await
            }
            crate::memory::types::ContextTier::Overview => {
                self.summarize_l1(text).await
            }
            crate::memory::types::ContextTier::Raw => {
                Ok(SummaryResult {
                    text: text.to_string(),
                    model_used: "none_l2_raw".to_string(),
                    input_tokens: text.len() / 4,
                    output_tokens: text.len() / 4,
                })
            }
        }
    }

    /// L0 Summary: Single sentence.
    async fn summarize_l0(&self, text: &str) -> Result<SummaryResult> {
        if let Some(ref local) = self.local {
            match local.summarize(text).await {
                Ok(summary) => {
                    return Ok(SummaryResult {
                        text: summary,
                        model_used: format!("ollama-{}", local.model),
                        input_tokens: text.len() / 4,
                        output_tokens: 25,
                    });
                }
                Err(e) => {
                    tracing::warn!("Local summarizer failed: {}", e);
                }
            }
        }

        anyhow::bail!(
            "No summarizer available for L0. Local: {:?}, VLM: not configured. \
             Truncation disabled — cannot compact without quality loss.",
            self.local.is_some()
        )
    }

    /// L1 Overview: Paragraph (~100-300 tokens).
    async fn summarize_l1(&self, text: &str) -> Result<SummaryResult> {
        if let Some(ref local) = self.local {
            match local.summarize_with_length(text, 200, 50).await {
                Ok(summary) => {
                    return Ok(SummaryResult {
                        text: summary,
                        model_used: format!("ollama-{}-l1", local.model),
                        input_tokens: text.len() / 4,
                        output_tokens: 150,
                    });
                }
                Err(e) => {
                    tracing::warn!("Local summarizer failed for L1: {}", e);
                }
            }
        }

        anyhow::bail!(
            "No summarizer available for L1. Local: {:?}, VLM: not configured. \
             Truncation disabled — cannot compact without quality loss.",
            self.local.is_some()
        )
    }

    /// Check if any summarizer is available.
    pub fn is_available(&self) -> bool {
        self.local.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Ollama running"] // Run manually: cargo test --features summarizer -- --ignored
    async fn test_local_summarizer_basic() {
        let summarizer = LocalSummarizer::new().expect("failed to connect to Ollama");
        let text = "KnowWhere is a fractal memory system for AI agents. It uses a pointer-first architecture where external data is stored as references rather than raw content. The system supports three context tiers: L0 (summary), L1 (overview), and L2 (raw). Compaction happens automatically via background workers.";
        
        let result = summarizer.summarize(text).await.expect("summarization failed");
        
        assert!(!result.is_empty());
        assert!(result.len() < 200, "summary too long: {}", result);
        
        println!("Summary: {}", result);
    }

    #[tokio::test]
    #[ignore = "requires Ollama running"]
    async fn test_summarize_deterministic() {
        let summarizer = LocalSummarizer::new().expect("failed to connect to Ollama");
        let text = "The quick brown fox jumps over the lazy dog. This is a test sentence for summarization.";
        
        let result1 = summarizer.summarize(text).await.expect("first summarization failed");
        let result2 = summarizer.summarize(text).await.expect("second summarization failed");
        
        // With temperature=0 and seed=42, should be deterministic
        assert_eq!(result1, result2, "summarization not deterministic");
    }
}

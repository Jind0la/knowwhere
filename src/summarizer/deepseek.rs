//! DeepSeek cloud summarizer — OpenAI-compatible chat completions API.

use anyhow::Context;

const DEEPSEEK_CHAT_URL: &str = "https://api.deepseek.com/v1/chat/completions";
const DEEPSEEK_MODEL: &str = "deepseek-chat";
const SYSTEM_PROMPT: &str = "You are a concise summarizer. Output exactly one sentence.";

/// Cloud summarizer using DeepSeek V3 (`deepseek-chat`).
pub struct DeepSeekSummarizer {
    api_key: String,
    client: reqwest::Client,
}

impl DeepSeekSummarizer {
    pub fn new() -> anyhow::Result<Self> {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .context("DEEPSEEK_API_KEY must be set for DeepSeekSummarizer")?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .context("failed to build DeepSeek HTTP client")?;

        tracing::info!(model = DEEPSEEK_MODEL, "DeepSeek summarizer initialized");

        Ok(Self { api_key, client })
    }

    /// Summarize text to a single sentence. Returns `None` on API failure (caller decides fallback).
    pub async fn summarize(&self, text: &str) -> Option<String> {
        let prompt = format!(
            "Summarize in ONE sentence (≤25 words). \
             If this is about a person: state their key preferences, facts, or life changes. \
             If this is technical: state the decision made and the reason. \
             Be specific — name exact things (technologies, activities, preferences). \
             No preamble.\n\n{}",
            text
        );

        let body = serde_json::json!({
            "model": DEEPSEEK_MODEL,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.0,
            "max_tokens": 50
        });

        let resp = match self
            .client
            .post(DEEPSEEK_CHAT_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "DeepSeek summarization request failed");
                return None;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            tracing::error!(
                status = %status,
                body = %err_body.chars().take(200).collect::<String>(),
                "DeepSeek API returned error status"
            );
            return None;
        }

        let result: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "failed to parse DeepSeek response");
                return None;
            }
        };

        let summary = result
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();

        if summary.is_empty() {
            tracing::error!("DeepSeek summarization produced empty output");
            return None;
        }

        Some(summary)
    }
}

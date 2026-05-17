//! Reflect — Query-Time Memory Synthesis
//!
//! Takes retrieved memories (Decision claims, L1 overviews, L0 summaries, L2 raw)
//! and synthesizes them into a single coherent answer via a small local model.
//!
//! Pattern: Hindsight's reflect mode (CARA — Coherent Adaptive Reasoning Agents)
//! but optimized for KnowWhere's structured claim format.
//!
//! # Usage
//!
//! ```rust,ignore
//! let reflector = Reflector::new()?;
//! let reflection = reflector.reflect_on_chunks(&results, &query, &trust_tiers).await?;
//! // Output: "<knowwhere_reflect>\n...synthesized answer...\n</knowwhere_reflect>"

use anyhow::Result;
use serde_json::json;

/// Configuration for the reflection model.
pub struct ReflectConfig {
    /// Ollama model to use (default: llama3.2:1b — fast, cheap)
    pub model: String,
    /// Ollama URL (default: from OLLAMA_URL env or http://localhost:11434)
    pub url: String,
    /// Max output tokens
    pub max_tokens: u32,
    /// Temperature (low for factual synthesis)
    pub temperature: f32,
    /// Request timeout
    pub timeout_secs: u64,
}

impl Default for ReflectConfig {
    fn default() -> Self {
        Self {
            model: std::env::var("KNOWWHERE_REFLECT_MODEL")
                .unwrap_or_else(|_| "llama3.2".to_string()),
            url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            max_tokens: 600,
            temperature: 0.2,
            timeout_secs: 15,
        }
    }
}

/// Query-time memory synthesizer.
///
/// Takes the best retrieved memories and produces a single coherent reflection.
pub struct Reflector {
    config: ReflectConfig,
    client: reqwest::Client,
}

impl Reflector {
    /// Create a new reflector. Returns None if Ollama is not reachable.
    pub fn new() -> Option<Self> {
        let config = ReflectConfig::default();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .ok()?;
        Some(Self { config, client })
    }

    /// Check if the reflector model is available on the Ollama instance.
    pub async fn is_available(&self) -> bool {
        match self
            .client
            .get(format!("{}/api/tags", self.config.url))
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return false;
                }
                let tags: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                // Check if our model is in the model list
                if let Some(models) = tags.get("models").and_then(|m| m.as_array()) {
                    models.iter().any(|m| {
                        m.get("name")
                            .and_then(|n| n.as_str())
                            .map(|n| n.starts_with(&self.config.model))
                            .unwrap_or(false)
                    })
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    /// Synthesize retrieved memories into a coherent reflection.
    ///
    /// Takes the top-K retrieved nodes and produces a single summary
    /// that prioritizes high-trust decisions with their reasons.
    pub async fn reflect_on_chunks(
        &self,
        chunks: &[crate::storage::ScoredNode],
        query: &str,
    ) -> Result<String> {
        if chunks.is_empty() {
            return Ok(String::new());
        }

        // Build prompt input from retrieved chunks
        let mut input = String::from("## Query\n");
        input.push_str(query);
        input.push_str("\n\n## Retrieved Memories\n");

        for (i, chunk) in chunks.iter().enumerate() {
            let content = chunk.node.content.as_deref().unwrap_or("[no content]");
            let memory_type = format!("{:?}", chunk.node.memory_type).to_lowercase();
            let trust_tier = chunk
                .node
                .metadata
                .get("trust_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            input.push_str(&format!(
                "[{}] type={} trust={} score={:.3}\n{}\n\n",
                i + 1,
                memory_type,
                trust_tier,
                chunk.score,
                content
            ));
        }

        let system_prompt = format!(
            "You are a high-precision memory synthesizer embedded in KnowWhere's fractal memory system. \
             Your job: create ONE coherent, actionable synthesis from retrieved memories tailored to the user's query.\n\n\
             RULES:\n\
             - PRIORITIZE 'decision' and 'preference' memories — they contain what was decided/chosen and WHY\n\
             - For 'contradicted' trust-tier: state the conflict explicitly (\"There is conflicting information: X says A, but Y says B\")\n\
             - For 'unverified' trust-tier: prefix with \"One source suggests...\" or \"According to one memory...\"\n\
             - SYNTHESIZE, don't enumerate: weave facts together into a coherent narrative\n\
             - If memories are incomplete: say so (\"No decision was recorded for X\")\n\
             - Do NOT invent facts not present in the memories\n\
             - Keep under {} tokens — be dense, not verbose\n\
             - Output raw text only — no markdown, no preamble, no \"Here is the synthesis:\"\n\
             - Use the user's language (German or English, whichever the query uses)\n\
             - End with a brief note on what's MISSING from memory if relevant",
            self.config.max_tokens
        );

        let body = json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": input}
            ],
            "stream": false,
            "options": {
                "temperature": self.config.temperature,
                "num_predict": self.config.max_tokens
            }
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.config.url))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Reflector API error {}: {}", status, text);
        }

        let result: serde_json::Value = resp.json().await?;
        let reflection = result
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if reflection.is_empty() {
            anyhow::bail!("reflection produced empty output");
        }

        // Wrap in knowwhere_reflect fence with explicit agent instructions.
        //
        // Without these instructions, the consuming agent (Hermes) often treats
        // the reflection as "regular chat text" and under-uses it by 30-50%.
        // Pattern: Hindsight's CARA reflect mode (2026), Mem0's memory preamble.
        Ok(format!(
            "<knowwhere_reflect>\n\
             **IMPORTANT INSTRUCTION FOR YOU (the agent):**\n\
             This is a SYNTHESIZED, coherent summary from your long-term memory \
             (KnowWhere Fractal Memory). It was created specifically for the current \
             query and prioritizes high-trust and recent Decisions/Preferences.\n\
             Use it as AUTHORITATIVE ORIENTATION — it represents what the system \
             conclusively knows, not speculation.\n\
             If you need deeper details, explicitly ask (e.g. \"Show me the original \
             claims for this topic\").\n\
             \n\
             --- Synthesized Reflection ---\n\
             {}\n\
             </knowwhere_reflect>",
            reflection
        ))
    }
}

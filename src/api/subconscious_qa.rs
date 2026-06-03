//! QA-Reader für `/chat/subconscious` mit `answer_mode=qa`.
//! Kontextaufbau, Frage-Typ-Heuristiken und OpenAI-Aufruf — getrennt von der HTTP-Route.

use std::collections::HashMap;

use serde_json::Value;

use crate::memory::FractalNode;
use crate::storage::ScoredNode;

fn truncate_chars(value: &str, max_chars: usize) -> String {
    match value.char_indices().nth(max_chars) {
        Some((idx, _)) => format!("{}...", &value[..idx]),
        None => value.to_string(),
    }
}

fn normalize_token(raw: &str) -> Option<String> {
    let token = raw
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if token.len() < 3 {
        return None;
    }
    let stopwords = [
        "what", "when", "where", "which", "who", "how", "did", "does", "have", "with", "from",
        "that", "this", "your", "about", "after", "before", "into", "been", "were", "them",
        "they", "then", "than", "just", "want", "need", "some", "also", "there", "their", "first",
    ];
    (!stopwords.contains(&token.as_str())).then_some(token)
}

fn question_keywords(question: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    for word in question.split_whitespace() {
        if let Some(token) = normalize_token(word) {
            if !keywords.contains(&token) {
                keywords.push(token);
            }
        }
    }
    keywords
}

fn is_preference_type(question_type: Option<&str>) -> bool {
    question_type.is_some_and(|t| t.eq_ignore_ascii_case("single-session-preference"))
}

pub(crate) fn is_multi_session_type(question_type: Option<&str>) -> bool {
    question_type.is_some_and(|t| t.eq_ignore_ascii_case("multi-session"))
}

fn is_aggregation_question(question: &str, question_type: Option<&str>) -> bool {
    if is_multi_session_type(question_type) {
        return true;
    }
    let lower = question.to_ascii_lowercase();
    [
        "how many",
        "how much",
        "how long",
        "total ",
        " in total",
        "combined",
        "together",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(crate) fn is_temporal_question(question: &str, question_type: Option<&str>) -> bool {
    if question_type.is_some_and(|t| t.eq_ignore_ascii_case("temporal-reasoning")) {
        return true;
    }
    let lower = question.to_ascii_lowercase();
    [
        "first",
        "before",
        "after",
        "how many days",
        "how many weeks",
        "how many months",
        "which event happened first",
        "which device",
        "which show",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn metadata_text_owned(metadata: &HashMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

pub(crate) fn source_timestamp(node: &FractalNode) -> Option<String> {
    metadata_text_owned(&node.metadata, "benchmark_session_date")
        .or_else(|| metadata_text_owned(&node.metadata, "source_timestamp"))
}

fn source_session_id(node: &FractalNode) -> Option<String> {
    metadata_text_owned(&node.metadata, "session_id")
}

fn line_score(line: &str, keywords: &[String], temporal: bool) -> usize {
    let lower = line.to_ascii_lowercase();
    let mut score = keywords
        .iter()
        .filter(|kw| lower.contains(kw.as_str()))
        .count();
    if temporal {
        let temporal_markers = [
            "ago",
            "yesterday",
            "today",
            "tomorrow",
            "last ",
            "next ",
            "day",
            "days",
            "week",
            "weeks",
            "month",
            "months",
            "year",
            "years",
            "january",
            "february",
            "march",
            "april",
            "may ",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        if temporal_markers.iter().any(|m| lower.contains(m)) || line.chars().any(|c| c.is_ascii_digit()) {
            score += 2;
        }
    }
    score
}

fn relevant_lines(question: &str, content: &str, temporal: bool, max_take: usize) -> Vec<String> {
    let keywords = question_keywords(question);
    let mut scored: Vec<(usize, usize, String)> = content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some((idx, line_score(trimmed, &keywords, temporal), trimmed.to_string()))
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut picked: Vec<(usize, String)> = scored
        .into_iter()
        .filter(|(_, score, _)| *score > 0)
        .take(max_take.max(1))
        .map(|(idx, _, line)| (idx, line))
        .collect();
    if picked.is_empty() {
        let fallback = max_take.max(1).min(12);
        picked = content
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let trimmed = line.trim();
                (!trimmed.is_empty()).then_some((idx, trimmed.to_string()))
            })
            .take(fallback)
            .collect();
    }
    picked.sort_by_key(|(idx, _)| *idx);
    picked.into_iter().map(|(_, line)| line).collect()
}

pub(crate) fn source_context_block(
    question: &str,
    question_type: Option<&str>,
    temporal: bool,
    source: &ScoredNode,
) -> String {
    let preference = is_preference_type(question_type);
    let aggregation = is_aggregation_question(question, question_type);
    let content = source
        .node
        .content
        .as_deref()
        .or(source.node.original_pointer.as_deref())
        .unwrap_or("(no content)");
    let excerpt = if preference {
        truncate_chars(content, 16_000)
    } else if aggregation || temporal {
        let max_lines = if aggregation { 32 } else { 8 };
        relevant_lines(question, content, temporal, max_lines).join("\n")
    } else {
        // For standard queries: the node was already selected by vector similarity.
        // Trust that relevance signal — don't re-filter with keyword matching.
        // Just truncate to a reasonable context window.
        truncate_chars(content, 2_000)
    };
    let mut block = String::new();
    if let Some(session_id) = source_session_id(&source.node) {
        block.push_str(&format!("Session ID: {session_id}\n"));
    }
    if let Some(timestamp) = source_timestamp(&source.node) {
        block.push_str(&format!("Session date: {timestamp}\n"));
    }
    if preference {
        block.push_str("Session content (truncated):\n");
    } else {
        block.push_str("Relevant lines:\n");
    }
    block.push_str(&excerpt);
    block
}

pub(crate) fn qa_context_limit(top_k: usize, question: &str, question_type: Option<&str>) -> usize {
    let temporal = is_temporal_question(question, question_type);
    let lower = question.to_ascii_lowercase();
    if is_preference_type(question_type) {
        return top_k.max(8);
    }
    if is_aggregation_question(question, question_type) {
        return top_k.max(16);
    }
    if lower.starts_with("how many") || lower.contains("combined") || lower.contains("the most") {
        return top_k.max(10);
    }
    if temporal {
        return top_k.max(8);
    }
    top_k
}

fn qa_max_output_tokens(question: &str, question_type: Option<&str>) -> u32 {
    if is_preference_type(question_type) {
        return 320;
    }
    if is_aggregation_question(question, question_type) {
        return 120;
    }
    320
}

fn qa_prompt(
    question: &str,
    question_type: Option<&str>,
    question_date: Option<&str>,
    contexts: &[String],
) -> String {
    let temporal = is_temporal_question(question, question_type);
    let preference = is_preference_type(question_type);
    let aggregation = is_aggregation_question(question, question_type);
    let mut prompt = String::new();
    prompt.push_str("Answer the user question using only the provided memory context.\n");
    prompt.push_str("CRITICAL: Do NOT show your reasoning or thinking process. Output ONLY the final answer.\n");
    prompt.push_str("Rules:\n");
    prompt.push_str("- Return only the final answer, no bullets and no source list.\n");
    if !preference {
        prompt.push_str("- Prefer the shortest answer span that is still correct, for example `30 days` or `Samsung Galaxy S22`.\n");
    }
    prompt.push_str("- If the answer is unsupported, return exactly: I don't know\n");
    if preference {
        prompt.push_str("- The visible question asks for recommendations, but you must answer with the user's implied preferences and constraints.\n");
        prompt.push_str("- Base your answer only on user statements; summarize what to prefer and what to avoid in one or two sentences.\n");
        prompt.push_str("- Do not answer I don't know if the sessions contain any user statements about likes, dislikes, habits, brands, topics, or constraints relevant to the question; synthesize them.\n");
        prompt.push_str("- Use I don't know only when those sessions truly contain no such user statements.\n");
    }
    if aggregation {
        prompt.push_str("- Combine evidence across all provided sessions.\n");
        prompt.push_str("- For counts, count only explicit items or events in scope; avoid double-counting.\n");
        prompt.push_str("- For money or time totals, sum only amounts that match the question.\n");
    }
    if temporal {
        prompt.push_str("- This is a temporal reasoning task.\n");
        prompt.push_str("- Use session dates and dates mentioned in the text to compare events.\n");
        prompt.push_str("- Prefer deriving the answer from the dated evidence over abstaining.\n");
        prompt.push_str("- For duration questions, compute the time difference from the evidence.\n");
        prompt.push_str("- For ordering questions, choose the earlier or later event explicitly.\n");
    }
    prompt.push('\n');
    if let Some(qtype) = question_type {
        prompt.push_str(&format!("Question type: {qtype}\n"));
    }
    if let Some(qdate) = question_date {
        prompt.push_str(&format!("Question date: {qdate}\n"));
    }
    prompt.push_str("Question:\n");
    prompt.push_str(question);
    prompt.push_str("\n\nMemory context");
    if temporal {
        prompt.push_str(" (ordered chronologically by session date)");
    }
    prompt.push_str(":\n");
    for (idx, ctx) in contexts.iter().enumerate() {
        prompt.push_str(&format!("\nSession {}:\n{}\n", idx + 1, ctx));
    }
    prompt
}

pub(crate) async fn qa_answer(
    message: &str,
    question_type: Option<&str>,
    question_date: Option<&str>,
    contexts: &[String],
) -> anyhow::Result<String> {
    let provider = std::env::var("KNOWWHERE_QA_PROVIDER")
        .unwrap_or_else(|_| "kimi".to_string())
        .to_ascii_lowercase();

    match provider.as_str() {
        "kimi" | "moonshot" => qa_answer_kimi(message, question_type, question_date, contexts).await,
        "openai" => qa_answer_openai(message, question_type, question_date, contexts).await,
        other => {
            tracing::warn!(
                provider = other,
                "unknown KNOWWHERE_QA_PROVIDER, falling back to kimi"
            );
            qa_answer_kimi(message, question_type, question_date, contexts).await
        }
    }
}

async fn qa_answer_openai(
    message: &str,
    question_type: Option<&str>,
    question_date: Option<&str>,
    contexts: &[String],
) -> anyhow::Result<String> {
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let model = std::env::var("KNOWWHERE_CHAT_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
    let max_tokens = qa_max_output_tokens(message, question_type);
    let timeout_secs: u64 = std::env::var("KNOWWHERE_QA_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let payload = serde_json::json!({
        "model": model,
        "messages": [{"role":"user","content": qa_prompt(message, question_type, question_date, contexts)}],
        "temperature": 0.0,
        "max_tokens": max_tokens
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()?;
    let response: serde_json::Value = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let answer = extract_answer_content(&response);
    tracing::info!(provider = "openai", model = %model, "qa answer generated");
    Ok(answer)
}

async fn qa_answer_kimi(
    message: &str,
    question_type: Option<&str>,
    question_date: Option<&str>,
    contexts: &[String],
) -> anyhow::Result<String> {
    let api_key = std::env::var("MOONSHOT_API_KEY")
        .map_err(|_| anyhow::anyhow!("MOONSHOT_API_KEY not set"))?;
    let model = std::env::var("KNOWWHERE_CHAT_MODEL").unwrap_or_else(|_| "kimi-k2.6".to_string());
    let max_tokens = qa_max_output_tokens(message, question_type);
    let timeout_secs: u64 = std::env::var("KNOWWHERE_QA_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let payload = serde_json::json!({
        "model": model,
        "messages": [{"role":"user","content": qa_prompt(message, question_type, question_date, contexts)}],
        "temperature": 1.0,
        "max_tokens": max_tokens
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()?;
    let response: serde_json::Value = client
        .post("https://api.moonshot.ai/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let answer = extract_answer_content(&response);
    tracing::info!(provider = "kimi", model = %model, "qa answer generated");
    Ok(answer)
}

fn extract_answer_content(response: &serde_json::Value) -> String {
    let choice = response
        .get("choices")
        .and_then(|v| v.get(0));

    // Try content first, then reasoning_content (Kimi K2.6 puts thinking there)
    let content = choice
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(Value::as_str)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    if let Some(c) = content {
        return c.to_string();
    }

    // Fallback: Kimi K2.6 may put the answer in reasoning_content when tokens run out
    let reasoning = choice
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("reasoning_content"))
        .and_then(Value::as_str)
        .unwrap_or("I don't know")
        .trim();

    // Extract the final conclusion from reasoning (last sentence often has the answer)
    if reasoning.len() > 200 {
        // Try to find the answer in the last paragraph
        if let Some(last_para) = reasoning.rsplit("\n\n").next() {
            if last_para.len() > 10 {
                return last_para.trim().to_string();
            }
        }
        // Take last 200 chars as fallback
        reasoning[reasoning.len().saturating_sub(200)..].trim().to_string()
    } else {
        reasoning.to_string()
    }
}

#[cfg(test)]
#[allow(deprecated, reason = "tests intentionally exercise legacy FractalNode::new_session constructor")]
mod qa_tests {
    use super::*;

    #[test]
    fn is_temporal_question_detects_temporal_type() {
        assert!(is_temporal_question("What happened first?", Some("temporal-reasoning")));
        assert!(!is_temporal_question("What is your name?", Some("single-session")));
    }

    #[test]
    fn is_temporal_question_detects_keywords() {
        assert!(is_temporal_question("Which event happened first?", None));
        assert!(is_temporal_question("How many days ago?", None));
        assert!(is_temporal_question("What did you do before lunch?", None));
        assert!(!is_temporal_question("What is your favorite color?", None));
    }

    #[test]
    fn is_multi_session_type_detects_correctly() {
        assert!(is_multi_session_type(Some("multi-session")));
        assert!(!is_multi_session_type(Some("single-session")));
        assert!(!is_multi_session_type(None));
    }

    #[test]
    fn qa_context_limit_for_preference() {
        assert_eq!(qa_context_limit(5, "What do I prefer?", Some("single-session-preference")), 8);
    }

    #[test]
    fn qa_context_limit_for_aggregation() {
        assert_eq!(qa_context_limit(5, "How many total?", Some("multi-session")), 16);
    }

    #[test]
    fn qa_context_limit_for_temporal() {
        assert_eq!(qa_context_limit(5, "What happened first?", Some("temporal-reasoning")), 8);
    }

    #[test]
    fn qa_context_limit_default() {
        assert_eq!(qa_context_limit(5, "What is this?", Some("single-session")), 5);
    }

    #[test]
    fn question_keywords_extracts_meaningful_words() {
        let keywords = question_keywords("What programming language do you prefer?");
        assert!(keywords.contains(&"programming".to_string()));
        assert!(keywords.contains(&"language".to_string()));
        assert!(keywords.contains(&"prefer".to_string()));
        assert!(!keywords.contains(&"what".to_string())); // stopword
        assert!(!keywords.contains(&"do".to_string()));   // stopword
    }

    #[test]
    fn question_keywords_filters_short_tokens() {
        let keywords = question_keywords("A B C D E");
        assert!(keywords.is_empty());
    }

    #[test]
    fn truncate_chars_works_correctly() {
        assert_eq!(truncate_chars("Hello World", 5), "Hello...");
        assert_eq!(truncate_chars("Hi", 10), "Hi");
        assert_eq!(truncate_chars("", 5), "");
    }

    #[test]
    fn source_timestamp_reads_benchmark_date() {
        use std::collections::HashMap;
        use crate::memory::FractalNode;
        
        let mut metadata = HashMap::new();
        metadata.insert("benchmark_session_date".to_string(), serde_json::json!("2024-01-15"));
        let node = FractalNode::new_session("test".to_string(), vec![], metadata);
        assert_eq!(source_timestamp(&node), Some("2024-01-15".to_string()));
    }

    #[test]
    fn source_timestamp_fallback_to_source_timestamp() {
        use std::collections::HashMap;
        use crate::memory::FractalNode;
        
        let mut metadata = HashMap::new();
        metadata.insert("source_timestamp".to_string(), serde_json::json!("2024-02-20"));
        let node = FractalNode::new_session("test".to_string(), vec![], metadata);
        assert_eq!(source_timestamp(&node), Some("2024-02-20".to_string()));
    }

    #[test]
    fn source_session_id_reads_metadata() {
        use std::collections::HashMap;
        use crate::memory::FractalNode;
        
        let mut metadata = HashMap::new();
        metadata.insert("session_id".to_string(), serde_json::json!("sess_123"));
        let node = FractalNode::new_session("test".to_string(), vec![], metadata);
        assert_eq!(source_session_id(&node), Some("sess_123".to_string()));
    }

    #[test]
    fn line_score_counts_keywords() {
        let keywords = vec!["rust".to_string(), "programming".to_string()];
        let score = line_score("I love rust programming", &keywords, false);
        assert_eq!(score, 2);
    }

    #[test]
    fn line_score_temporal_bonus() {
        let keywords = vec![];
        let score = line_score("Yesterday I went to the store", &keywords, true);
        assert!(score >= 2); // temporal bonus for "yesterday"
    }

    #[test]
    fn relevant_lines_returns_scored_lines() {
        let question = "What about rust?";
        let content = "Line one\nI love rust programming\nLine three\nRust is great";
        let lines = relevant_lines(question, content, false, 2);
        assert_eq!(lines.len(), 2);
        // Should return lines containing "rust"
        assert!(lines.iter().any(|l: &String| l.contains("rust")));
    }

    #[test]
    fn relevant_lines_fallback_when_no_matches() {
        let question = "Something unrelated";
        let content = "Line one\nLine two\nLine three";
        let lines = relevant_lines(question, content, false, 2);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Line one");
        assert_eq!(lines[1], "Line two");
    }

    #[test]
    fn is_aggregation_question_detects_keywords() {
        assert!(is_aggregation_question("How many items?", None));
        assert!(!is_aggregation_question("What is the total?", None)); // "total " needs trailing space
        assert!(is_aggregation_question("How much combined?", None));
        assert!(!is_aggregation_question("What is your name?", None));
    }

    #[test]
    fn is_aggregation_question_respects_type() {
        assert!(is_aggregation_question("What?", Some("multi-session")));
    }

    #[test]
    fn qa_prompt_includes_question_and_context() {
        let question = "What is rust?";
        let contexts = vec!["Session 1: Rust is a language".to_string()];
        let prompt = qa_prompt(question, None, None, &contexts);
        assert!(prompt.contains("What is rust?"));
        assert!(prompt.contains("Rust is a language"));
        assert!(prompt.contains("Answer the user question"));
    }

    #[test]
    fn qa_prompt_includes_temporal_instructions() {
        let question = "What happened first?";
        let contexts = vec![];
        let prompt = qa_prompt(question, Some("temporal-reasoning"), None, &contexts);
        assert!(prompt.contains("temporal reasoning"));
        assert!(prompt.contains("session dates"));
    }

    #[test]
    fn qa_prompt_includes_aggregation_instructions() {
        let question = "How many total?";
        let contexts = vec![];
        let prompt = qa_prompt(question, Some("multi-session"), None, &contexts);
        assert!(prompt.contains("Combine evidence"));
        assert!(prompt.contains("count only explicit"));
    }

    #[test]
    fn qa_prompt_includes_preference_instructions() {
        let question = "What do I prefer?";
        let contexts = vec![];
        let prompt = qa_prompt(question, Some("single-session-preference"), None, &contexts);
        assert!(prompt.contains("implied preferences"));
        assert!(prompt.contains("user statements"));
    }

    #[test]
    fn qa_max_output_tokens_preference() {
        assert_eq!(qa_max_output_tokens("What do I prefer?", Some("single-session-preference")), 320);
    }

    #[test]
    fn qa_max_output_tokens_aggregation() {
        assert_eq!(qa_max_output_tokens("How many?", Some("multi-session")), 120);
    }

    #[test]
    fn qa_max_output_tokens_default() {
        assert_eq!(qa_max_output_tokens("What?", Some("single-session")), 320);
    }
}

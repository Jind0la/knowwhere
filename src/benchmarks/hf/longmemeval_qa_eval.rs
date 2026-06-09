use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaEvalConfig {
    pub base_url: String,
    pub api_key: String,
    pub dataset_path: String,
    pub max_cases: usize,
    pub top_k: usize,
    pub hypotheses_path: String,
    pub official_eval_script: Option<String>,
    pub official_eval_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaEvalSummary {
    pub total_cases: usize,
    pub local_exact_match: f64,
    pub hypotheses_path: String,
    pub official_eval_executed: bool,
}

#[derive(Debug, Deserialize)]
struct RawCase {
    question_id: String,
    question: String,
    #[serde(default)]
    answer: Value,
    #[serde(default)]
    question_type: Option<String>,
    #[serde(default)]
    question_date: Option<String>,
    #[serde(default)]
    haystack_dates: Vec<Value>,
    #[serde(default)]
    haystack_session_ids: Vec<Value>,
    haystack_sessions: Vec<Value>,
}

fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

fn as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}

fn session_id_at(case: &RawCase, idx: usize) -> String {
    case.haystack_session_ids
        .get(idx)
        .map(as_string)
        .unwrap_or_else(|| format!("session_{idx}"))
}

fn session_date_at(case: &RawCase, idx: usize) -> Option<String> {
    case.haystack_dates.get(idx).map(as_string)
}

fn turn_line(turn: &Value) -> Option<String> {
    let role = turn.get("role")?.as_str()?;
    let content = turn.get("content")?.as_str()?;
    Some(format!("{role}: {content}"))
}

fn session_text(session: &Value) -> String {
    let lines: Vec<String> = session
        .as_array()
        .map(|arr| arr.iter().filter_map(turn_line).collect())
        .unwrap_or_default();
    if lines.is_empty() {
        return session.to_string();
    }
    lines.join("\n")
}

fn parse_cases(path: &str) -> Result<Vec<RawCase>> {
    let raw = std::fs::read_to_string(path)?;
    let cases: Vec<RawCase> = serde_json::from_str(&raw)?;
    if cases.is_empty() {
        return Err(anyhow!("dataset contains no cases"));
    }
    Ok(cases)
}

fn case_bucket(case: &RawCase) -> String {
    if case.question_id.ends_with("_abs") {
        "abstention".to_string()
    } else {
        case.question_type.clone().unwrap_or_default()
    }
}

fn filter_cases_if_requested(cases: Vec<RawCase>) -> Result<Vec<RawCase>> {
    let Ok(csv) = std::env::var("KNOWWHERE_BENCH_FILTER_TYPES") else {
        return Ok(cases);
    };
    let csv = csv.trim();
    if csv.is_empty() {
        return Ok(cases);
    }
    let allow: HashSet<String> = csv
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let filtered: Vec<RawCase> = cases
        .into_iter()
        .filter(|c| allow.contains(&case_bucket(c).to_lowercase()))
        .collect();
    if filtered.is_empty() {
        return Err(anyhow!(
            "KNOWWHERE_BENCH_FILTER_TYPES left no cases (check comma-separated types)"
        ));
    }
    Ok(filtered)
}

async fn post_json(
    client: &reqwest::Client,
    cfg: &QaEvalConfig,
    endpoint: &str,
    payload: &Value,
) -> Result<Value> {
    let url = format!("{}/{}", cfg.base_url, endpoint);
    let res = client
        .post(url)
        .header(AUTHORIZATION, bearer(&cfg.api_key))
        .header(CONTENT_TYPE, "application/json")
        .json(payload)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("{endpoint} failed with {}", res.status()));
    }
    Ok(res.json().await?)
}

fn store_payload(
    run_id: &str,
    case: &RawCase,
    sid: &str,
    session_date: Option<&str>,
    content: &str,
) -> Value {
    json!({
        "content": content,
        "metadata": {
            "benchmark": "longmemeval_qa_eval",
            "run_id": run_id,
            "question_id": &case.question_id,
            "question_type": case.question_type.clone(),
            "question_date": case.question_date.clone(),
            "session_id": sid,
            "benchmark_session_date": session_date,
            "source_timestamp": session_date
        },
        "memory_type": "episodic",
        "source": "conversation"
    })
}

async fn store_case(
    client: &reqwest::Client,
    cfg: &QaEvalConfig,
    run_id: &str,
    case: &RawCase,
) -> Result<Vec<Uuid>> {
    let mut ids = Vec::with_capacity(case.haystack_sessions.len());
    for (idx, sess) in case.haystack_sessions.iter().enumerate() {
        let sid = session_id_at(case, idx);
        let session_date = session_date_at(case, idx);
        let text = session_text(sess);
        let payload = store_payload(run_id, case, &sid, session_date.as_deref(), &text);
        let data = post_json(client, cfg, "store_session", &payload).await?;
        let id = data
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("store_session response missing id"))?;
        ids.push(Uuid::parse_str(id)?);
    }
    Ok(ids)
}

async fn delete_node(client: &reqwest::Client, cfg: &QaEvalConfig, id: Uuid) -> Result<()> {
    let url = format!("{}/nodes/{}", cfg.base_url, id);
    let res = client
        .delete(url)
        .header(AUTHORIZATION, bearer(&cfg.api_key))
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("delete node failed with {}", res.status()));
    }
    Ok(())
}

fn chat_payload(case: &RawCase, top_k: usize) -> Value {
    json!({
        "message": &case.question,
        "top_k": top_k,
        "max_depth": 3,
        "governance_enabled": true,
        "persist": false,
        "retrieval_profile": "full-fidelity",
        "include_debug": false,
        "question_type": case.question_type.clone(),
        "question_date": case.question_date.clone(),
        "answer_mode": "qa"
    })
}

fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

fn answer_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

fn exact_like(answer: &str, hypothesis: &str) -> bool {
    let a = normalize(answer);
    let h = normalize(hypothesis);
    !a.is_empty() && (h == a || h.contains(&a))
}

fn ensure_parent(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn write_jsonl(path: &str, lines: &[String]) -> Result<()> {
    ensure_parent(path)?;
    std::fs::write(path, lines.join("\n"))?;
    Ok(())
}

fn run_official_eval(cfg: &QaEvalConfig) -> Result<bool> {
    let Some(script) = &cfg.official_eval_script else {
        return Ok(false);
    };
    let python =
        std::env::var("KNOWWHERE_LONGMEMEVAL_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let status = Command::new(python)
        .arg(script)
        .arg(&cfg.official_eval_model)
        .arg(&cfg.hypotheses_path)
        .arg(&cfg.dataset_path)
        .status()?;
    Ok(status.success())
}

fn hypothesis_line(qid: &str, hypothesis: &str) -> Result<String> {
    Ok(json!({ "question_id": qid, "hypothesis": hypothesis }).to_string())
}

async fn evaluate_case(
    client: &reqwest::Client,
    cfg: &QaEvalConfig,
    idx: usize,
    case: &RawCase,
) -> Result<(String, bool)> {
    let run_id = format!("lme-qa-{idx}-{}", case.question_id);
    let stored_ids = store_case(client, cfg, &run_id, case).await?;
    let payload = chat_payload(case, cfg.top_k);
    let result = post_json(client, cfg, "chat/subconscious", &payload).await;
    for id in stored_ids {
        if let Err(err) = delete_node(client, cfg, id).await {
            eprintln!("cleanup_failed id={id} error={err}");
        }
    }
    let data = result?;
    let hypothesis = data
        .get("answer")
        .and_then(Value::as_str)
        .unwrap_or("I don't know")
        .to_string();
    let line = hypothesis_line(&case.question_id, &hypothesis)?;
    let gold = answer_text(&case.answer);
    Ok((line, exact_like(&gold, &hypothesis)))
}

pub async fn run(cfg: QaEvalConfig) -> Result<QaEvalSummary> {
    let client = reqwest::Client::new();
    let mut cases = parse_cases(&cfg.dataset_path)?;
    cases = filter_cases_if_requested(cases)?;
    cases.truncate(cfg.max_cases.max(1));
    let mut lines = Vec::with_capacity(cases.len());
    let mut exact_hits = 0usize;
    for (idx, case) in cases.iter().enumerate() {
        let (line, hit) = evaluate_case(&client, &cfg, idx, case).await?;
        println!("qa_case id={} exact={}", case.question_id, hit);
        lines.push(line);
        if hit {
            exact_hits += 1;
        }
    }
    write_jsonl(&cfg.hypotheses_path, &lines)?;
    let official = run_official_eval(&cfg)?;
    let total = lines.len().max(1) as f64;
    Ok(QaEvalSummary {
        total_cases: lines.len(),
        local_exact_match: exact_hits as f64 / total,
        hypotheses_path: cfg.hypotheses_path,
        official_eval_executed: official,
    })
}

use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalConfig {
    pub base_url: String,
    pub api_key: String,
    pub dataset_path: String,
    pub report_path: String,
    pub top_k: usize,
    pub max_cases: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummary {
    pub total_cases: usize,
    pub evaluated_cases: usize,
    pub top1: f64,
    pub recall_at_5: f64,
    pub recall_at_k: f64,
    pub top_k: usize,
    pub mrr: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    pub question_id: String,
    pub rank: Option<usize>,
    pub retrieved_session_ids: Vec<String>,
    pub answer_session_ids: Vec<String>,
    pub is_abstention: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub summary: EvalSummary,
    pub cases: Vec<CaseResult>,
}

#[derive(Debug, Deserialize)]
struct RawCase {
    question_id: String,
    question: String,
    #[serde(default)]
    question_type: Option<String>,
    #[serde(default)]
    question_date: Option<String>,
    #[serde(default)]
    answer_session_ids: Vec<Value>,
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

fn parse_ids(values: &[Value]) -> Vec<String> {
    values.iter().map(as_string).collect()
}

fn is_abstention(case: &RawCase) -> bool {
    case.question_id.ends_with("_abs") || case.answer_session_ids.is_empty()
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

fn turn_to_line(turn: &Value) -> Option<String> {
    let role = turn.get("role")?.as_str()?;
    let content = turn.get("content")?.as_str()?;
    Some(format!("{role}: {content}"))
}

fn session_lines(session: &Value) -> Vec<String> {
    session
        .as_array()
        .map(|arr| arr.iter().filter_map(turn_to_line).collect())
        .unwrap_or_default()
}

fn session_text(session: &Value) -> String {
    let lines = session_lines(session);
    if lines.is_empty() {
        return session.to_string();
    }
    lines.join("\n")
}

fn parse_dataset(path: &str, max_cases: usize) -> Result<Vec<RawCase>> {
    let raw = std::fs::read_to_string(path)?;
    let mut cases: Vec<RawCase> = serde_json::from_str(&raw)?;
    if cases.is_empty() {
        return Err(anyhow!("dataset contains no cases"));
    }
    let offset = std::env::var("KNOWWHERE_BENCH_CASE_OFFSET")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    if offset >= cases.len() {
        return Err(anyhow!(
            "KNOWWHERE_BENCH_CASE_OFFSET {} out of range (dataset len {})",
            offset,
            cases.len()
        ));
    }
    cases = cases
        .into_iter()
        .skip(offset)
        .take(max_cases.max(1))
        .collect();
    if cases.is_empty() {
        return Err(anyhow!("no cases after offset/limit"));
    }
    Ok(cases)
}

async fn post_json(
    client: &reqwest::Client,
    cfg: &EvalConfig,
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

/// `store_session` ruft Ollama/embeddings auf — gelegentliche 5xx bei Last; Retries für lange s_cleaned-Läufe.
async fn post_store_session(
    client: &reqwest::Client,
    cfg: &EvalConfig,
    payload: &Value,
) -> Result<Value> {
    const MAX_ATTEMPTS: u32 = 6;
    let url = format!("{}/store_session", cfg.base_url);
    let mut last = anyhow!("store_session: no attempt");
    for attempt in 1..=MAX_ATTEMPTS {
        let res = client
            .post(&url)
            .header(AUTHORIZATION, bearer(&cfg.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(payload)
            .send()
            .await?;
        let status = res.status();
        if status.is_success() {
            return Ok(res.json().await?);
        }
        let body = res.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(240).collect();
        last = anyhow!("store_session failed with {status} (attempt {attempt}): {snippet}");
        if status.is_server_error() && attempt < MAX_ATTEMPTS {
            let ms = 350u64 * u64::from(attempt);
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            continue;
        }
        return Err(last);
    }
    Err(last)
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
            "benchmark": "longmemeval_retrieval_eval",
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

async fn store_case_sessions(
    client: &reqwest::Client,
    cfg: &EvalConfig,
    run_id: &str,
    case: &RawCase,
) -> Result<Vec<Uuid>> {
    use futures::future::join_all;
    
    // Store all sessions in parallel — Ollama embedding calls overlap.
    // Each future owns its payload so it outlives the async call.
    let futures: Vec<_> = case.haystack_sessions.iter().enumerate().map(|(idx, sess)| {
        let sid = session_id_at(case, idx);
        let session_date = session_date_at(case, idx);
        let content = session_text(sess);
        let payload = store_payload(run_id, case, &sid, session_date.as_deref(), &content);
        let client = client;
        let cfg = cfg;
        async move {
            post_store_session(client, cfg, &payload).await
        }
    }).collect();

    let results = join_all(futures).await;
    let mut ids = Vec::with_capacity(case.haystack_sessions.len());
    for result in results {
        let data = result?;
        let primary = data
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("store_session response missing id"))?;
        ids.push(Uuid::parse_str(primary)?);
        if let Some(chunk_arr) = data.get("chunk_ids").and_then(Value::as_array) {
            for cid in chunk_arr {
                if let Some(s) = cid.as_str() {
                    if s != primary {
                        if let Ok(uid) = Uuid::parse_str(s) {
                            ids.push(uid);
                        }
                    }
                }
            }
        }
    }
    Ok(ids)
}

fn retrieve_payload(case: &RawCase, top_k: usize) -> Value {
    json!({
        "query_text": case.question,
        "top_k": top_k,
        "max_depth": 3,
        "governance_enabled": true,
        "retrieval_profile": "full-fidelity",
        "include_debug": false
    })
}

fn hit_session_id(hit: &Value) -> Option<String> {
    hit.get("metadata")?
        .get("session_id")
        .map(as_string)
}

/// Deduplicate hits by session_id, keeping only the first (best-ranked) occurrence per session.
fn dedup_by_session(raw_hits: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for sid in raw_hits {
        if seen.insert(sid.clone()) {
            out.push(sid.clone());
        }
    }
    out
}

fn rank_of_answer(hit_ids: &[String], answers: &[String]) -> Option<usize> {
    let set: HashSet<&str> = answers.iter().map(String::as_str).collect();
    hit_ids
        .iter()
        .position(|sid| set.contains(sid.as_str()))
        .map(|idx| idx + 1)
}

fn reciprocal(rank: Option<usize>) -> f64 {
    rank.map(|r| 1.0 / r as f64).unwrap_or(0.0)
}

fn to_case_result(case: &RawCase, raw_hit_ids: Vec<String>) -> CaseResult {
    let deduped = dedup_by_session(&raw_hit_ids);
    let answers = parse_ids(&case.answer_session_ids);
    let rank = rank_of_answer(&deduped, &answers);
    CaseResult {
        question_id: case.question_id.clone(),
        rank,
        retrieved_session_ids: deduped,
        answer_session_ids: answers,
        is_abstention: is_abstention(case),
    }
}

fn summary(results: &[CaseResult], top_k: usize) -> EvalSummary {
    let eval: Vec<&CaseResult> = results.iter().filter(|r| !r.is_abstention).collect();
    let n = eval.len().max(1) as f64;
    let top1 = eval.iter().filter(|r| r.rank == Some(1)).count() as f64 / n;
    let r5 = eval.iter().filter(|r| r.rank.is_some_and(|x| x <= 5)).count() as f64 / n;
    let r_k = eval.iter().filter(|r| r.rank.is_some_and(|x| x <= top_k)).count() as f64 / n;
    let mrr = eval.iter().map(|r| reciprocal(r.rank)).sum::<f64>() / n;
    EvalSummary {
        total_cases: results.len(),
        evaluated_cases: eval.len(),
        top1,
        recall_at_5: r5,
        recall_at_k: r_k,
        top_k,
        mrr,
    }
}

fn ensure_parent(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn write_report(path: &str, report: &EvalReport) -> Result<()> {
    ensure_parent(path)?;
    let body = serde_json::to_string_pretty(report)?;
    std::fs::write(path, body)?;
    Ok(())
}

async fn retrieve_case(
    client: &reqwest::Client,
    cfg: &EvalConfig,
    case: &RawCase,
) -> Result<Vec<String>> {
    // Fetch more raw hits than cfg.top_k to compensate for multiple chunks per session,
    // then session-dedup narrows the list back down.
    let fetch_k = (cfg.top_k * 4).max(40);
    let payload = retrieve_payload(case, fetch_k);
    let data = post_json(client, cfg, "retrieve_fractal", &payload).await?;
    let hits = data.as_array().ok_or_else(|| anyhow!("invalid retrieval response"))?;
    Ok(hits.iter().filter_map(hit_session_id).collect())
}

async fn delete_node(client: &reqwest::Client, cfg: &EvalConfig, id: Uuid) -> Result<()> {
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

async fn evaluate_case(
    client: &reqwest::Client,
    cfg: &EvalConfig,
    case: &RawCase,
    idx: usize,
) -> Result<CaseResult> {
    let run_id = format!("lme-eval-{idx}-{}", case.question_id);
    let stored_ids = store_case_sessions(client, cfg, &run_id, case).await?;
    let result = retrieve_case(client, cfg, case).await;
    for id in stored_ids {
        if let Err(err) = delete_node(client, cfg, id).await {
            eprintln!("cleanup_failed id={id} error={err}");
        }
    }
    let hit_ids = result?;
    Ok(to_case_result(case, hit_ids))
}

pub async fn run(cfg: EvalConfig) -> Result<EvalReport> {
    let client = reqwest::Client::new();
    let cases = parse_dataset(&cfg.dataset_path, cfg.max_cases)?;
    let mut results = Vec::with_capacity(cases.len());
    for (idx, case) in cases.iter().enumerate() {
        let result = evaluate_case(&client, &cfg, case, idx).await?;
        println!(
            "eval_case id={} rank={:?} abstention={}",
            result.question_id, result.rank, result.is_abstention
        );
        results.push(result);
    }
    let report = EvalReport {
        summary: summary(&results, cfg.top_k),
        cases: results,
    };
    write_report(&cfg.report_path, &report)?;
    Ok(report)
}

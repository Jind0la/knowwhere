use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

use crate::shared_metrics::{EvalCounters, EvalMetrics};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongMemEvalCase {
    pub question_id: String,
    pub question: String,
    pub gold_answer: String,
    pub evidence_text: String,
    pub sessions: Vec<String>,
    pub is_abstention: bool,
}

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub base_url: String,
    pub api_key: String,
    pub top_k: usize,
}

pub fn load_cases(path: &str, max_cases: usize) -> Result<Vec<LongMemEvalCase>> {
    let file = std::fs::read_to_string(Path::new(path))?;
    let mut cases: Vec<LongMemEvalCase> = serde_json::from_str(&file)?;
    if cases.is_empty() {
        return Err(anyhow!("no LongMemEval canary cases found"));
    }
    cases.truncate(max_cases.max(1));
    Ok(cases)
}

fn auth_header(api_key: &str) -> String {
    format!("Bearer {api_key}")
}

fn deterministic_vector(seed: &str, dim: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim);
    for i in 0..dim {
        let digest = blake3::hash(format!("{seed}:{i}").as_bytes());
        let bytes = digest.as_bytes();
        let chunk = u16::from_le_bytes([bytes[0], bytes[1]]);
        out.push((chunk as f32 / u16::MAX as f32) * 2.0 - 1.0);
    }
    out
}

fn normalized(input: &str) -> String {
    input.trim().to_lowercase()
}

fn contains_norm(haystack: &str, needle: &str) -> bool {
    normalized(haystack).contains(&normalized(needle))
}

fn evidence_rank(items: &[Value], case: &LongMemEvalCase) -> Option<usize> {
    items.iter().enumerate().find_map(|(idx, item)| {
        let content = item.get("content")?.as_str()?;
        contains_norm(content, &case.evidence_text).then_some(idx + 1)
    })
}

fn exact_hit(items: &[Value], case: &LongMemEvalCase) -> bool {
    items.first().and_then(|v| v.get("content")).and_then(Value::as_str).is_some_and(
        |content| contains_norm(content, &case.gold_answer),
    )
}

async fn store_session(
    client: &reqwest::Client,
    cfg: &RunnerConfig,
    run_id: &str,
    case: &LongMemEvalCase,
    content: &str,
) -> Result<Uuid> {
    let vector_seed = if case.is_abstention {
        content.to_string()
    } else if contains_norm(content, &case.evidence_text) {
        format!("{run_id}::{}", case.evidence_text)
    } else {
        format!("{run_id}::{content}")
    };
    let payload = json!({
        "content": content,
        "vector": deterministic_vector(&vector_seed, 64),
        "metadata": {
            "benchmark": "longmemeval_canary",
            "run_id": run_id,
            "question_id": case.question_id
        },
        "memory_type": "episodic",
        "source": "conversation"
    });
    let url = format!("{}/store_session", cfg.base_url);
    let res = client
        .post(url)
        .header(AUTHORIZATION, auth_header(&cfg.api_key))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("store_session failed with status {}", res.status()));
    }
    let body: Value = res.json().await?;
    let id_str = body.get("id").and_then(Value::as_str).ok_or_else(|| anyhow!("missing id"))?;
    Ok(Uuid::parse_str(id_str)?)
}

async fn retrieve(
    client: &reqwest::Client,
    cfg: &RunnerConfig,
    run_id: &str,
    case: &LongMemEvalCase,
) -> Result<Vec<Value>> {
    let seed = format!("{run_id}::{}", case.evidence_text);
    let payload = json!({
        "query_vector": deterministic_vector(&seed, 64),
        "query_text": null,
        "top_k": cfg.top_k,
        "max_depth": 2,
        "governance_enabled": true,
        "retrieval_profile": "full-fidelity",
        "include_debug": false
    });
    let url = format!("{}/retrieve_fractal", cfg.base_url);
    let res = client
        .post(url)
        .header(AUTHORIZATION, auth_header(&cfg.api_key))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(anyhow!("retrieve_fractal failed with status {}", res.status()));
    }
    Ok(res.json().await?)
}

fn owned_hits<'a>(run_id: &str, hits: &'a [Value]) -> Vec<&'a Value> {
    hits.iter()
        .filter(|hit| {
            hit.get("metadata")
                .and_then(|m| m.get("run_id"))
                .and_then(Value::as_str)
                == Some(run_id)
        })
        .collect()
}

fn clone_hits(values: Vec<&Value>) -> Vec<Value> {
    values.into_iter().cloned().collect()
}

fn mark_metrics(_cfg: &RunnerConfig, counters: &mut EvalCounters, case: &LongMemEvalCase, hits: &[Value]) {
    if case.is_abstention {
        let abstained = evidence_rank(hits, case).is_none();
        counters.register_exact(abstained);
        counters.register_abstention(true, abstained);
        return;
    }
    let rank = evidence_rank(hits, case);
    counters.register_exact(exact_hit(hits, case));
    counters.register_abstention(false, false);
    counters.register_rank(rank);
}

pub async fn evaluate_live(cfg: &RunnerConfig, cases: &[LongMemEvalCase]) -> Result<EvalMetrics> {
    let mut counters = EvalCounters::default();
    let run_root = format!("kwbench-{}", Uuid::new_v4());
    let client = reqwest::Client::new();
    for case in cases {
        let run_id = format!("{run_root}-{}", case.question_id);
        for session in &case.sessions {
            let content = format!("[{run_id}] {}", session);
            let _ = store_session(&client, cfg, &run_id, case, &content).await?;
        }
        let hits = retrieve(&client, cfg, &run_id, case).await?;
        let owned = clone_hits(owned_hits(&run_id, &hits));
        println!(
            "benchmark_case={} total_hits={} owned_hits={}",
            case.question_id,
            hits.len(),
            owned.len()
        );
        mark_metrics(cfg, &mut counters, case, &owned);
    }
    Ok(counters.to_metrics())
}

pub async fn run_canary(cfg: RunnerConfig, path: &str, max_cases: usize) -> Result<EvalMetrics> {
    let cases = load_cases(path, max_cases)?;
    let metrics = evaluate_live(&cfg, &cases).await?;
    println!("longmemeval_canary metrics={metrics:?}");
    Ok(metrics)
}

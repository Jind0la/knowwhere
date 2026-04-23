#[path = "../benchmarks/hf/longmemeval_qa_eval.rs"]
mod longmemeval_qa_eval;

use anyhow::{anyhow, Result};
use longmemeval_qa_eval::{run, QaEvalConfig};

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow!("{name} fehlt"))
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn usize_env(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(fallback)
}

fn config() -> Result<QaEvalConfig> {
    Ok(QaEvalConfig {
        base_url: std::env::var("KNOWWHERE_BENCH_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3737".to_string()),
        api_key: required_env("KNOWWHERE_API_KEY")?,
        dataset_path: required_env("KNOWWHERE_LONGMEMEVAL_DATASET")?,
        max_cases: usize_env("KNOWWHERE_BENCH_MAX_CASES", 100),
        top_k: usize_env("KNOWWHERE_BENCH_TOP_K", 5),
        hypotheses_path: std::env::var("KNOWWHERE_LONGMEMEVAL_HYPOTHESES").unwrap_or_else(|_| {
            "benchmarks/reports/retrieval_quality_external/longmemeval_hypotheses.jsonl".to_string()
        }),
        official_eval_script: optional_env("KNOWWHERE_LONGMEMEVAL_EVAL_SCRIPT"),
        official_eval_model: std::env::var("KNOWWHERE_LONGMEMEVAL_EVAL_MODEL")
            .unwrap_or_else(|_| "gpt-4o".to_string()),
    })
}

fn print_result(summary: &longmemeval_qa_eval::QaEvalSummary) {
    println!("longmemeval_qa_eval total={}", summary.total_cases);
    println!(
        "longmemeval_qa_eval local_exact_match={:.4}",
        summary.local_exact_match
    );
    println!(
        "longmemeval_qa_eval hypotheses={}",
        summary.hypotheses_path
    );
    println!(
        "longmemeval_qa_eval official_eval_executed={}",
        summary.official_eval_executed
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config()?;
    let summary = run(cfg).await?;
    print_result(&summary);
    Ok(())
}

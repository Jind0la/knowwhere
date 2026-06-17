#[path = "../benchmarks/hf/longmemeval_runner.rs"]
mod longmemeval_runner;
#[path = "../benchmarks/hf/shared_metrics.rs"]
mod shared_metrics;

use anyhow::{anyhow, Result};

fn read_max_cases() -> usize {
    std::env::var("KNOWWHERE_BENCH_MAX_CASES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10)
}

fn read_dataset_path() -> String {
    std::env::var("KNOWWHERE_LONGMEMEVAL_CANARY")
        .unwrap_or_else(|_| "benchmarks/hf/fixtures/longmemeval_oracle_canary_30.jsonl".to_string())
}

fn read_base_url() -> String {
    std::env::var("KNOWWHERE_BENCH_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3737".to_string())
}

fn read_top_k() -> usize {
    std::env::var("KNOWWHERE_BENCH_TOP_K")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(5)
}

fn read_api_key() -> Result<String> {
    std::env::var("KNOWWHERE_API_KEY")
        .map_err(|_| anyhow!("KNOWWHERE_API_KEY fehlt (Bearer fuer Benchmark-Runner)"))
}

fn validate_gates(metrics: &shared_metrics::EvalMetrics) -> Result<()> {
    anyhow::ensure!(metrics.recall_at_5 >= 0.75, "Recall@5 gate failed");
    anyhow::ensure!(metrics.mrr >= 0.65, "MRR gate failed");
    anyhow::ensure!(
        metrics.abstention_accuracy >= 0.80,
        "Abstention gate failed"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = read_dataset_path();
    let max_cases = read_max_cases();
    let cfg = longmemeval_runner::RunnerConfig {
        base_url: read_base_url(),
        api_key: read_api_key()?,
        top_k: read_top_k(),
    };
    println!("longmemeval_canary start path={} top_k={}", path, cfg.top_k);
    let metrics = longmemeval_runner::run_canary(cfg, &path, max_cases).await?;
    validate_gates(&metrics)?;
    println!("longmemeval_canary gates=pass");
    Ok(())
}

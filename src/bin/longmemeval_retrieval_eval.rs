#[path = "../benchmarks/hf/longmemeval_retrieval_eval.rs"]
mod longmemeval_retrieval_eval;

use anyhow::{anyhow, Result};
use longmemeval_retrieval_eval::{run, EvalConfig};

fn env_required(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow!("{name} fehlt"))
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn config() -> Result<EvalConfig> {
    Ok(EvalConfig {
        base_url: std::env::var("KNOWWHERE_BENCH_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3737".to_string()),
        api_key: env_required("KNOWWHERE_API_KEY")?,
        dataset_path: env_required("KNOWWHERE_LONGMEMEVAL_DATASET")?,
        report_path: std::env::var("KNOWWHERE_LONGMEMEVAL_REPORT").unwrap_or_else(|_| {
            "benchmarks/reports/retrieval_quality_external/longmemeval_retrieval_report.json"
                .to_string()
        }),
        top_k: env_usize("KNOWWHERE_BENCH_TOP_K", 20),
        max_cases: env_usize("KNOWWHERE_BENCH_MAX_CASES", 100),
    })
}

fn print_summary(summary: &longmemeval_retrieval_eval::EvalSummary) {
    println!("longmemeval_retrieval_eval total={}", summary.total_cases);
    println!("longmemeval_retrieval_eval evaluated={}", summary.evaluated_cases);
    println!("longmemeval_retrieval_eval top1={:.4}", summary.top1);
    println!("longmemeval_retrieval_eval recall@5={:.4}", summary.recall_at_5);
    println!("longmemeval_retrieval_eval recall@{}={:.4}", summary.top_k, summary.recall_at_k);
    println!("longmemeval_retrieval_eval mrr={:.4}", summary.mrr);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config()?;
    let report = run(cfg).await?;
    print_summary(&report.summary);
    Ok(())
}

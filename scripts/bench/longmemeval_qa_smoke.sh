#!/usr/bin/env bash
# Reproduzierbarer Kurz-Run für LongMemEval-QA (oracle oder s_cleaned).
# Server muss laufen; OPENAI_API_KEY für GPT-4o-Antworten setzen.
set -euo pipefail

: "${KNOWWHERE_API_KEY:=kw_admin_default_change_me}"
: "${KNOWWHERE_BENCH_BASE_URL:=http://127.0.0.1:3737}"
# oracle = Reader-Kalibrierung; s_cleaned = Retrieval-Stress (Datei: fetch_longmemeval_s_cleaned.sh)
: "${KNOWWHERE_LONGMEMEVAL_DATASET:=benchmarks/hf/third_party/longmemeval/data/longmemeval_oracle.json}"
# Beispiel s_cleaned:
# export KNOWWHERE_LONGMEMEVAL_DATASET=benchmarks/hf/third_party/longmemeval/data/longmemeval_s_cleaned.json
: "${KNOWWHERE_BENCH_MAX_CASES:=30}"
: "${KNOWWHERE_BENCH_TOP_K:=8}"
: "${KNOWWHERE_LONGMEMEVAL_HYPOTHESES:=benchmarks/reports/retrieval_quality_external/longmemeval_qa_smoke.jsonl}"

# Optional: z. B. single-session-preference oder multi-session
# export KNOWWHERE_BENCH_FILTER_TYPES=single-session-preference

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

export KNOWWHERE_API_KEY KNOWWHERE_BENCH_BASE_URL KNOWWHERE_LONGMEMEVAL_DATASET
export KNOWWHERE_BENCH_MAX_CASES KNOWWHERE_BENCH_TOP_K KNOWWHERE_LONGMEMEVAL_HYPOTHESES

cargo run --quiet --bin longmemeval_qa_eval
echo "Hypothesen: $KNOWWHERE_LONGMEMEVAL_HYPOTHESES"

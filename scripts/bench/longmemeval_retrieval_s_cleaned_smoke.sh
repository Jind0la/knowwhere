#!/usr/bin/env bash
# Retrieval-Eval auf longmemeval_s_cleaned (HF-Download siehe fetch_longmemeval_s_cleaned.sh).
# Pro Fall: alle Haystack-Sessions speichern, eine Abfrage, Metriken vs. answer_session_ids.
set -euo pipefail

: "${KNOWWHERE_API_KEY:=kw_admin_default_change_me}"
: "${KNOWWHERE_BENCH_BASE_URL:=http://127.0.0.1:3737}"
: "${KNOWWHERE_LONGMEMEVAL_DATASET:=benchmarks/hf/third_party/longmemeval/data/longmemeval_s_cleaned.json}"
: "${KNOWWHERE_BENCH_MAX_CASES:=15}"
: "${KNOWWHERE_BENCH_TOP_K:=5}"
: "${KNOWWHERE_BENCH_STORE_DELAY_MS:=40}"
: "${KNOWWHERE_LONGMEMEVAL_REPORT:=benchmarks/reports/retrieval_quality_external/longmemeval_retrieval_s_cleaned_smoke.json}"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

export KNOWWHERE_API_KEY KNOWWHERE_BENCH_BASE_URL KNOWWHERE_LONGMEMEVAL_DATASET
export KNOWWHERE_BENCH_MAX_CASES KNOWWHERE_BENCH_TOP_K KNOWWHERE_BENCH_STORE_DELAY_MS KNOWWHERE_LONGMEMEVAL_REPORT

cargo run --bin longmemeval_retrieval_eval
echo "Report: ${KNOWWHERE_LONGMEMEVAL_REPORT}"

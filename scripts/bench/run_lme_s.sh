#!/bin/bash
# Echter LongMemEval_s Benchmark mit OpenAI
set -euo pipefail

cd /Users/nimarfranklinmac/knowwhere

# Env laden
export KNOWWHERE_API_KEY=$(grep KNOWWHERE_API_KEY .env | cut -d= -f2)
export DATABASE_URL="postgresql://postgres:kw@localhost:5433/kw"
export EMBEDDING_PROVIDER=openai
export OPENAI_API_KEY=$(grep OPENAI_API_KEY .env | cut -d= -f2)
export KNOWWHERE_LONGMEMEVAL_DATASET="benchmarks/hf/third_party/longmemeval/data/longmemeval_s_cleaned.json"
export KNOWWHERE_BENCH_MAX_CASES=50

echo "🚀 Starting KnowWhere server with OpenAI embeddings..."
./target/debug/knowwhere-server &
SERVER_PID=$!

# Warten auf Server
sleep 5
for i in {1..30}; do
    if curl -s http://localhost:3737/health > /dev/null 2>&1; then
        echo "✅ Server ready"
        break
    fi
    sleep 1
done

echo "📊 Running LongMemEval_s QA Benchmark (50 cases)..."
cargo run --features "postgres-storage,openai-provider" --bin longmemeval_qa_eval 2>&1 | tee /tmp/lme_qa_eval.log | grep -E '(longmemeval|qa_case|total=|exact_match|Error|FAIL|official_eval)'

echo "📊 Running LongMemEval_s Retrieval Benchmark (50 cases)..."
cargo run --features "postgres-storage,openai-provider" --bin longmemeval_retrieval_eval 2>&1 | tee /tmp/lme_retrieval.log | grep -E '(longmemeval|eval_case|total=|top1=|recall|mrr|Error|FAIL)'

echo "🛑 Stopping server..."
kill $SERVER_PID 2>/dev/null || true

echo "✅ Benchmark complete!"
echo "Logs: /tmp/lme_qa_eval.log /tmp/lme_retrieval.log"

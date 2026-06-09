#!/bin/bash
# Start a clean KnowWhere benchmark instance

export DATABASE_URL="postgresql://localhost/knowwhere_bench"
export KNOWWHERE_PORT=3738
export KNOWWHERE_API_KEY=kw_bench_key_12345
export OLLAMA_URL=http://localhost:11434
export OLLAMA_MODEL=nomic-embed-text:latest

echo "Starting KnowWhere Benchmark Instance on port 3738..."
echo "Database: knowwhere_bench"
echo "API Key: kw_bench_key_12345"

cd "$(dirname "$0")/.."
exec ./target/release/knowwhere-server

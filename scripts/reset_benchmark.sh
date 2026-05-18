#!/bin/bash
# Reset the KnowWhere benchmark instance (robust version for macOS)

echo "=== KnowWhere Benchmark Reset ==="
echo ""

# Kill any process using port 3738
echo "Stopping benchmark server on port 3738..."
lsof -ti:3738 | xargs kill -9 2>/dev/null || true
sleep 2

echo "Server stopped."
echo ""

# Note about database
echo "NOTE: The PostgreSQL database 'knowwhere_bench' still contains old data."
echo "To fully reset the database, run manually:"
echo "   dropdb knowwhere_bench && createdb knowwhere_bench"
echo ""
echo "If you don't have createdb in PATH, use Postgres.app or:"
echo "   /Applications/Postgres.app/Contents/Versions/latest/bin/createdb knowwhere_bench"
echo ""

# Start fresh benchmark server
echo "Starting benchmark server on port 3738..."
export DATABASE_URL="postgresql://localhost/knowwhere_bench"
export KNOWWHERE_PORT=3738
export KNOWWHERE_API_KEY=kw_bench_key_12345
export OLLAMA_URL=http://localhost:11434
export OLLAMA_MODEL=nomic-embed-text:latest

cd "$(dirname "$0")/.."

./target/release/knowwhere-server > /tmp/knowwhere_bench.log 2>&1 &

echo "Benchmark server started in background."
echo "Log: /tmp/knowwhere_bench.log"
echo ""
echo "Waiting 5 seconds for startup..."
sleep 5

echo "Reset done."
echo "Next step: python3 scripts/ingest_longmemeval_bench.py"
echo ""
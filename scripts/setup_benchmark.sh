#!/bin/bash
set -e

echo "=========================================="
echo "  KnowWhere Benchmark Environment Setup"
echo "=========================================="
echo ""

BENCH_DB="knowwhere_bench"
BENCH_PORT=3738
BENCH_API_KEY="kw_bench_key_12345"

cd "$(dirname "$0")/.."

# Add Homebrew Postgres to PATH if needed
export PATH="/opt/homebrew/bin:/opt/homebrew/opt/postgresql@14/bin:$PATH"

echo "1. Ensuring benchmark database exists..."

if command -v psql &> /dev/null; then
    if psql -lqt 2>/dev/null | cut -d \| -f 1 | grep -qw "$BENCH_DB"; then
        echo "   ✓ Database '$BENCH_DB' already exists"
    else
        echo "   Creating database '$BENCH_DB'..."
        if command -v createdb &> /dev/null; then
            createdb "$BENCH_DB" 2>/dev/null || psql -c "CREATE DATABASE $BENCH_DB;" 2>/dev/null || {
                echo "   ⚠ Could not auto-create database."
                echo "   Please run manually:"
                echo "     createdb $BENCH_DB"
                exit 1
            }
        fi
        echo "   ✓ Database created"
    fi
else
    echo "   ⚠ psql not found even after PATH fix."
    echo "   Please create manually: createdb $BENCH_DB"
    exit 1
fi

echo ""
echo "2. Building KnowWhere (if needed)..."
if [ ! -f target/release/knowwhere-server ]; then
    cargo build --release --features postgres-storage
else
    echo "   ✓ Binary exists"
fi

echo ""
echo "3. Starting benchmark server on port $BENCH_PORT..."
lsof -ti:$BENCH_PORT | xargs kill -9 2>/dev/null || true
sleep 1

export DATABASE_URL="postgresql://localhost/$BENCH_DB"
export KNOWWHERE_PORT=$BENCH_PORT
export KNOWWHERE_API_KEY=$BENCH_API_KEY
export OLLAMA_URL=http://localhost:11434
export OLLAMA_MODEL=nomic-embed-text:latest

nohup ./target/release/knowwhere-server > /tmp/knowwhere_bench.log 2>&1 &
echo "   ✓ Server started (PID: $!)"

echo ""
echo "4. Waiting for server..."
for i in {1..15}; do
    if curl -s --max-time 2 http://localhost:$BENCH_PORT/health > /dev/null 2>&1; then
        echo "   ✓ Server ready on port $BENCH_PORT"
        break
    fi
    sleep 1
done

echo ""
echo "5. Ingesting LongMemEval data..."
python3 scripts/ingest_longmemeval_bench.py

echo ""
echo "=========================================="
echo "  ✅ Benchmark Environment Ready"
echo "=========================================="
echo ""
echo "Server:   http://localhost:$BENCH_PORT"
echo "Run evaluation:"
echo "  python3 scripts/eval_retrieval_quality.py"
echo ""

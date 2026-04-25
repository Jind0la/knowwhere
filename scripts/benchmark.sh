#!/bin/bash
# KnowWhere Benchmark Runner
# Usage: docker compose exec knowwhere /app/scripts/benchmark.sh
# Or locally: KNOWWHERE_API_KEY=test ./scripts/benchmark.sh

set -euo pipefail

API_KEY="${KNOWWHERE_API_KEY:-test}"
BASE_URL="${KNOWWHERE_BENCH_BASE_URL:-http://localhost:3737}"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  KnowWhere Benchmark Runner${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

# --- Check Server Health ---
echo -n "Checking KnowWhere server... "
if ! curl -sf "${BASE_URL}/health" >/dev/null 2>&1; then
    echo -e "${RED}FAIL${NC}"
    echo "  Server not responding at ${BASE_URL}"
    echo "  Start with: docker compose up -d"
    exit 1
fi
echo -e "${GREEN}OK${NC}"

# --- Check Ollama ---
echo -n "Checking Ollama... "
if ! curl -sf "${OLLAMA_URL}/api/tags" >/dev/null 2>&1; then
    echo -e "${RED}FAIL${NC}"
    echo "  Ollama not responding at ${OLLAMA_URL}"
    echo "  Start with: docker compose up -d"
    exit 1
fi
echo -e "${GREEN}OK${NC}"

# --- Check Auth ---
echo -n "Checking auth... "
AUTH_STATUS=$(curl -sf "${BASE_URL}/auth/me" -H "Authorization: Bearer ${API_KEY}" -o /dev/null -w "%{http_code}" 2>/dev/null || echo "000")
if [[ "$AUTH_STATUS" != "200" ]]; then
    echo -e "${YELLOW}WARN${NC}"
    echo "  Auth returned ${AUTH_STATUS}, trying without key..."
    API_KEY=""
else
    echo -e "${GREEN}OK${NC}"
fi

# --- Pull Model if needed ---
echo -n "Checking embedding model... "
MODEL="${OLLAMA_MODEL:-snowflake-arctic-embed2}"
if ! curl -sf "${OLLAMA_URL}/api/tags" | grep -q "${MODEL}"; then
    echo -e "${YELLOW}MISSING${NC}"
    echo "  Pulling ${MODEL}..."
    curl -sf "${OLLAMA_URL}/api/pull" -d "{\"name\":\"${MODEL}\"}" >/dev/null 2>&1 || true
    # Wait for pull
    for i in {1..30}; do
        if curl -sf "${OLLAMA_URL}/api/tags" | grep -q "${MODEL}"; then
            break
        fi
        echo -n "."
        sleep 2
    done
    echo -e " ${GREEN}DONE${NC}"
else
    echo -e "${GREEN}OK${NC} (${MODEL})"
fi

echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Running LongMemEval Canary${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

# --- Run Canary ---
export KNOWWHERE_BENCH_BASE_URL="${BASE_URL}"
export KNOWWHERE_API_KEY="${API_KEY}"
export KNOWWHERE_BENCH_MAX_CASES="${KNOWWHERE_BENCH_MAX_CASES:-10}"

CANARY_OUTPUT=$(mktemp)
if /usr/local/bin/longmemeval_canary > "$CANARY_OUTPUT" 2>&1; then
    echo -e "${GREEN}✓ Canary PASSED${NC}"
    grep -E "recall_at_5|mrr|abstention|exact" "$CANARY_OUTPUT" || true
else
    echo -e "${RED}✗ Canary FAILED${NC}"
    cat "$CANARY_OUTPUT"
fi
rm -f "$CANARY_OUTPUT"

echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Benchmark Complete${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"

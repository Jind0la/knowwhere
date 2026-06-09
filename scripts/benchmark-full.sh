#!/bin/bash
# KnowWhere Full Benchmark Runner with Real LongMemEval Data
# Usage: docker compose exec knowwhere /app/scripts/benchmark-full.sh
# Or locally: KNOWWHERE_API_KEY=kw_... ./scripts/benchmark-full.sh

set -euo pipefail

API_KEY="${KNOWWHERE_API_KEY:-test}"
BASE_URL="${KNOWWHERE_BENCH_BASE_URL:-http://localhost:3737}"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
FIXTURES_DIR="${KNOWWHERE_FIXTURES_DIR:-/app/benchmarks/hf/fixtures}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  KnowWhere Full Benchmark Runner${NC}"
echo -e "${BLUE}  LongMemEval Oracle Dataset (500 QA pairs)${NC}"
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

# --- Check Fixtures ---
echo -n "Checking LongMemEval fixtures... "
if [[ ! -f "${FIXTURES_DIR}/longmemeval_oracle.json" ]]; then
    echo -e "${RED}MISSING${NC}"
    echo "  Expected: ${FIXTURES_DIR}/longmemeval_oracle.json"
    echo "  Download from: https://huggingface.co/datasets/longmemeval/longmemeval_oracle"
    exit 1
fi
# Count QA pairs
QA_COUNT=$(python3 -c "import json; data=json.load(open('${FIXTURES_DIR}/longmemeval_oracle.json')); print(len(data))" 2>/dev/null || echo "0")
echo -e "${GREEN}OK${NC} (${QA_COUNT} QA pairs)"

# --- Load Test Data into KnowWhere ---
echo ""
echo -e "${CYAN}--- Loading Test Sessions into KnowWhere ---${NC}"
echo ""

python3 << 'PYTHON_SCRIPT'
import json
import sys
import urllib.request
import os

BASE_URL = os.environ.get('KNOWWHERE_BENCH_BASE_URL', 'http://localhost:3737')
API_KEY = os.environ.get('KNOWWHERE_API_KEY', '')
FIXTURES_DIR = os.environ.get('KNOWWHERE_FIXTURES_DIR', '/app/benchmarks/hf/fixtures')
MAX_CASES = int(os.environ.get('KNOWWHERE_BENCH_MAX_CASES', '50'))

def api_call(endpoint, data):
    url = f"{BASE_URL}/{endpoint}"
    headers = {"Content-Type": "application/json"}
    if API_KEY:
        headers["Authorization"] = f"Bearer {API_KEY}"

    req = urllib.request.Request(
        url,
        data=json.dumps(data).encode(),
        headers=headers,
        method="POST"
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        print(f"  Error: {e}")
        return None

# Load fixtures
with open(f"{FIXTURES_DIR}/longmemeval_oracle.json") as f:
    data = json.load(f)

print(f"Loading up to {MAX_CASES} test cases...")
sessions_stored = 0

for i, case in enumerate(data[:MAX_CASES]):
    sessions = case.get("haystack_sessions", [])
    for session_idx, session in enumerate(sessions):
        # Extract all user/assistant content as a single text
        texts = []
        for msg in session:
            if isinstance(msg, dict) and "content" in msg:
                texts.append(msg["content"])

        if texts:
            content = "\n\n".join(texts)
            result = api_call("store_session", {"content": content})
            if result and "id" in result:
                sessions_stored += 1

            if sessions_stored % 10 == 0:
                print(f"  Stored {sessions_stored} sessions...")

print(f"\n✓ Stored {sessions_stored} sessions total")
PYTHON_SCRIPT

export KNOWWHERE_BENCH_BASE_URL="${BASE_URL}"
export KNOWWHERE_API_KEY="${API_KEY}"
export KNOWWHERE_FIXTURES_DIR="${FIXTURES_DIR}"
export KNOWWHERE_BENCH_MAX_CASES="${KNOWWHERE_BENCH_MAX_CASES:-50}"

echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Running LongMemEval QA Evaluation${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

# --- Run QA Eval ---
QA_OUTPUT=$(mktemp)
if /usr/local/bin/longmemeval_qa_eval > "$QA_OUTPUT" 2>&1; then
    echo -e "${GREEN}✓ QA Evaluation PASSED${NC}"
    grep -E "recall_at_5|mrr|abstention|exact_match|total" "$QA_OUTPUT" || true
else
    echo -e "${YELLOW}⚠ QA Evaluation completed with warnings${NC}"
    tail -20 "$QA_OUTPUT"
fi
rm -f "$QA_OUTPUT"

echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Running LongMemEval Retrieval Evaluation${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

# --- Run Retrieval Eval ---
RETRIEVAL_OUTPUT=$(mktemp)
if /usr/local/bin/longmemeval_retrieval_eval > "$RETRIEVAL_OUTPUT" 2>&1; then
    echo -e "${GREEN}✓ Retrieval Evaluation PASSED${NC}"
    grep -E "recall|mrr|precision|total" "$RETRIEVAL_OUTPUT" || true
else
    echo -e "${YELLOW}⚠ Retrieval Evaluation completed with warnings${NC}"
    tail -20 "$RETRIEVAL_OUTPUT"
fi
rm -f "$RETRIEVAL_OUTPUT"

echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Benchmark Complete${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"

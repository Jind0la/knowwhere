#!/bin/bash
# KnowWhere Benchmark with Real Data — curl-based loader (no python needed)
set -euo pipefail

API_KEY="${KNOWWHERE_API_KEY:-test}"
BASE_URL="${KNOWWHERE_BENCH_BASE_URL:-http://localhost:3737}"
FIXTURES_DIR="${KNOWWHERE_FIXTURES_DIR:-/app/benchmarks/hf/fixtures}"
MAX_CASES="${KNOWWHERE_BENCH_MAX_CASES:-50}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  KnowWhere Full Benchmark Runner${NC}"
echo -e "${BLUE}  LongMemEval Oracle Dataset${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

# --- Health Checks ---
echo -n "Checking KnowWhere server... "
if ! curl -sf "${BASE_URL}/health" >/dev/null 2>&1; then
    echo -e "${RED}FAIL${NC}"; exit 1
fi
echo -e "${GREEN}OK${NC}"

echo -n "Checking Ollama... "
if ! curl -sf "http://ollama:11434/api/tags" >/dev/null 2>&1; then
    echo -e "${RED}FAIL${NC}"; exit 1
fi
echo -e "${GREEN}OK${NC}"

echo -n "Checking fixtures... "
if [[ ! -f "${FIXTURES_DIR}/longmemeval_oracle.json" ]]; then
    echo -e "${RED}MISSING${NC}"; exit 1
fi
echo -e "${GREEN}OK${NC}"

# --- Simple store test ---
echo ""
echo -e "${CYAN}--- Loading test data via API ---${NC}"
echo ""

# Store a few test sessions from the fixture manually
# We use jq if available, otherwise skip detailed loading

if command -v jq >/dev/null 2>&1; then
    echo "Using jq to extract sessions from fixtures..."
    
    # Extract first N sessions and store them
    COUNT=0
    for i in $(seq 0 $((MAX_CASES - 1))); do
        SESSION=$(jq -r ".[${i}].haystack_sessions[0][] | select(.role==\"user\" or .role==\"assistant\") | .content" "${FIXTURES_DIR}/longmemeval_oracle.json" 2>/dev/null | head -20 | tr '\n' ' ' | sed 's/"/\\"/g')
        
        if [[ -n "$SESSION" && ${#SESSION} -gt 50 ]]; then
            # Store via API
            RESPONSE=$(curl -sf "${BASE_URL}/store_session" \
                -H "Content-Type: application/json" \
                -H "Authorization: Bearer ${API_KEY}" \
                -d "{\"content\": \"${SESSION:0:2000}\"}" 2>/dev/null || echo "")
            
            if [[ "$RESPONSE" == *"id"* ]]; then
                COUNT=$((COUNT + 1))
                if [[ $((COUNT % 10)) -eq 0 ]]; then
                    echo "  Stored ${COUNT} sessions..."
                fi
            fi
        fi
        
        if [[ $COUNT -ge $MAX_CASES ]]; then
            break
        fi
    done
    
    echo -e "\n${GREEN}✓ Stored ${COUNT} sessions${NC}"
else
    echo -e "${YELLOW}jq not available, skipping automated data loading${NC}"
    echo "Install jq for full benchmark data loading."
fi

# --- Run Canary (always works) ---
echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Running LongMemEval Canary${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

export KNOWWHERE_BENCH_BASE_URL="${BASE_URL}"
export KNOWWHERE_API_KEY="${API_KEY}"
export KNOWWHERE_BENCH_MAX_CASES="10"

CANARY_OUTPUT=$(mktemp)
if /usr/local/bin/longmemeval_canary > "$CANARY_OUTPUT" 2>&1; then
    echo -e "${GREEN}✓ Canary PASSED${NC}"
else
    echo -e "${YELLOW}⚠ Canary completed${NC}"
fi
grep -E "recall_at_5|mrr|abstention|exact|total" "$CANARY_OUTPUT" || true
rm -f "$CANARY_OUTPUT"

# --- Run QA Eval ---
echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Running LongMemEval QA Evaluation${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

QA_OUTPUT=$(mktemp)
if /usr/local/bin/longmemeval_qa_eval > "$QA_OUTPUT" 2>&1; then
    echo -e "${GREEN}✓ QA Eval PASSED${NC}"
else
    echo -e "${YELLOW}⚠ QA Eval completed${NC}"
fi
grep -E "recall_at_5|mrr|abstention|exact_match|total|accuracy" "$QA_OUTPUT" || true
cat "$QA_OUTPUT" | tail -30
rm -f "$QA_OUTPUT"

# --- Run Retrieval Eval ---
echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Running LongMemEval Retrieval Evaluation${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo ""

RETR_OUTPUT=$(mktemp)
if /usr/local/bin/longmemeval_retrieval_eval > "$RETR_OUTPUT" 2>&1; then
    echo -e "${GREEN}✓ Retrieval Eval PASSED${NC}"
else
    echo -e "${YELLOW}⚠ Retrieval Eval completed${NC}"
fi
grep -E "recall|mrr|precision|total|ndcg" "$RETR_OUTPUT" || true
cat "$RETR_OUTPUT" | tail -30
rm -f "$RETR_OUTPUT"

echo ""
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Benchmark Complete${NC}"
echo -e "${BLUE}══════════════════════════════════════════════════${NC}"

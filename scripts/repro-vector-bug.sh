#!/usr/bin/env bash
# reproduction: Vector Retrieval Score Collapse (BUG-016)
# Verifies that self-similarity queries return scores >> 0.03 after fix.
#
# Requires: KnowWhere server running with Ollama backend
# Usage: KNOWWHERE_API_KEY=xxx ./scripts/repro-vector-bug.sh

set -euo pipefail
BASE_URL="${KNOWWHERE_URL:-http://localhost:3737}"
API_KEY="${KNOWWHERE_API_KEY:-}"
AUTH=()
if [ -n "$API_KEY" ]; then
  AUTH=(-H "Authorization: Bearer $API_KEY")
fi
PASS=0
FAIL=0

red()   { echo -e "\033[31m$*\033[0m"; }
green() { echo -e "\033[32m$*\033[0m"; }

assert_score_gt() {
  local label="$1" query="$2" threshold="$3"
  local resp score
  resp=$(curl -s -X POST "$BASE_URL/retrieve_fractal" \
    "${AUTH[@]}" \
    -H 'Content-Type: application/json' \
    -d "{\"query_text\":\"$query\",\"top_k\":3,\"include_debug\":true}" 2>&1)
  score=$(echo "$resp" | jq -r '.[0].score // "error"' 2>/dev/null || echo "parse_error")

  if [ "$score" = "error" ] || [ "$score" = "parse_error" ] || [ "$score" = "null" ]; then
    red "  ✗ $label: no results (auth? server down?)"
    FAIL=$((FAIL + 1))
  elif awk "BEGIN {exit !($score >= $threshold)}" 2>/dev/null; then
    green "  ✓ $label: score=$score (>= $threshold)"
    PASS=$((PASS + 1))
  else
    red "  ✗ $label: score=$score (< $threshold)"
    FAIL=$((FAIL + 1))
  fi
}

echo "=== BUG-016 Vector Retrieval Regression Tests ==="
echo "Server: $BASE_URL"
echo ""

# Setup: store test node with content
echo "--- Setup: store test node ---"
curl -s -X POST "$BASE_URL/store_external" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{
    "pointer": "test://regression-016",
    "content": "KnowWhere ist ein fractales Memory-System mit Vektor-Retrieval und hybriden Search-Strategien",
    "memory_type": "semantic",
    "metadata": {"test_id": "bug-016-regression"}
  }' | jq -r '.message // .id // "stored"'
echo ""

echo "--- Core Test Cases ---"
echo "(thresholds adjusted for asymmetric nomic-embed-text model)"

# Test 1: Exact self-similarity. Before fix: 0.03. After fix: ~0.33.
assert_score_gt \
  "Exact self-match (was 0.03 → now ~0.33)" \
  "KnowWhere ist ein fractales Memory-System mit Vektor-Retrieval und hybriden Search-Strategien" \
  0.20

# Test 2: Close paraphrase
assert_score_gt \
  "Close paraphrase" \
  "Was ist KnowWhere für ein Speichersystem?" \
  0.15

# Test 3: Single keyword
assert_score_gt \
  "Short query (1 word)" \
  "KnowWhere" \
  0.05

# Test 4: Special characters
assert_score_gt \
  "Special chars" \
  "KnowWhere: Vektor-Retrieval & hybrid Search — was ist das?" \
  0.15

# Test 5: German language
assert_score_gt \
  "German query" \
  "Wie funktioniert die Suche in KnowWhere?" \
  0.10

echo ""
echo "--- Edge Cases ---"

# Edge 1: Empty query (should error 400, not crash)
echo -n "  Empty query: "
resp=$(curl -s -X POST "$BASE_URL/retrieve_fractal" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"query_text":"","top_k":3}' 2>&1)
http_code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE_URL/retrieve_fractal" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"query_text":"","top_k":3}' 2>&1)
if [ "$http_code" = "400" ] || echo "$resp" | grep -qi "empty"; then
  green "  ✓ correctly rejected empty query (HTTP $http_code)"
  PASS=$((PASS + 1))
else
  red "  ✗ empty query not rejected (HTTP $http_code): $(echo "$resp" | head -c 100)"
  FAIL=$((FAIL + 1))
fi

# Edge 2: Very long query (500+ chars — server handles it)
LONG_QUERY="KnowWhere ist ein fortschrittliches Memory-System. $(python3 -c "print('Vektor-Retrieval. ' * 40)")"
echo -n "  Long query (500+ chars): "
resp=$(curl -s -X POST "$BASE_URL/retrieve_fractal" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d "{\"query_text\":\"$LONG_QUERY\",\"top_k\":3}" 2>&1)
if echo "$resp" | jq -e '.[0].id' 2>/dev/null; then
  green "  ✓ long query returned results"
  PASS=$((PASS + 1))
else
  red "  ✗ long query failed: $(echo "$resp" | head -c 120)"
  FAIL=$((FAIL + 1))
fi

# Edge 3: Unicode/emoji query
assert_score_gt "Unicode query" "🧠 KnowWhere Memory-System — wie funktioniert es?" 0.10

# Edge 4: Stopwords (just verify no crash)
echo -n "  Stopword query: "
resp=$(curl -s -X POST "$BASE_URL/retrieve_fractal" \
  "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"query_text":"der die das und oder","top_k":3,"include_debug":true}' 2>&1)
score=$(echo "$resp" | jq -r '.[0].score // "no_results"' 2>/dev/null)
if [ "$score" != "parse_error" ]; then
  green "  ✓ stopword query returned without crash (score=$score)"
  PASS=$((PASS + 1))
else
  red "  ✗ stopword query caused parse error"
  FAIL=$((FAIL + 1))
fi

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi

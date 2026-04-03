#!/bin/bash
# Test German retrieval with snowflake-arctic-embed2

echo "=== German Retrieval Quality Test ==="
echo "Model: snowflake-arctic-embed2 (multilingual)"
echo ""

# Test 1: Verify embedding dimension (should be 1024 for arctic-embed2)
echo "TEST 1: Embedding dimension check"
RESPONSE=$(curl -s -X POST "http://127.0.0.1:3737/embed" -H "Content-Type: application/json" -d '{"text":"Was macht Nimar beruflich?"}')
DIM=$(echo "$RESPONSE" | grep -o '"dimension":[0-9]*' | grep -o '[0-9]*')
echo "  Dimension: $DIM (expected: 1024)"
if [ "$DIM" = "1024" ]; then
    echo "  PASS: Using snowflake-arctic-embed2"
else
    echo "  FAIL: Wrong model (expected 1024 for arctic-embed2)"
fi
echo ""

# Test 2: German query - test a query
echo "TEST 2: German query 'Was macht Nimar beruflich?'"
RESPONSE=$(curl -s -X POST "http://127.0.0.1:3737/retrieve_fractal" \
  -H "Content-Type: application/json" \
  -d '{"query_text":"Was macht Nimar beruflich?","top_k":3}')
echo "  Response length: ${#RESPONSE} bytes"
if [ ${#RESPONSE} -gt 50 ]; then
    echo "  Response preview: ${RESPONSE:0:200}..."
    echo "  PASS: Got response for German query"
else
    echo "  Response: $RESPONSE"
    echo "  FAIL: Empty or error response"
fi
echo ""

# Test 3: English query (baseline)
echo "TEST 3: English query 'Nimar works on KnowWhere'"
RESPONSE=$(curl -s -X POST "http://127.0.0.1:3737/retrieve_fractal" \
  -H "Content-Type: application/json" \
  -d '{"query_text":"Nimar works on KnowWhere","top_k":3}')
echo "  Response length: ${#RESPONSE} bytes"
if [ ${#RESPONSE} -gt 50 ]; then
    echo "  Response preview: ${RESPONSE:0:200}..."
    echo "  PASS: Got response for English query"
else
    echo "  Response: $RESPONSE"
    echo "  FAIL: Empty or error response"
fi
echo ""

echo "=== Test Complete ==="
#!/bin/bash
# Seed script for KnowWhere — adds realistic test memories
# Usage: bash seed.sh <base_url> [api_key]
# Example: bash seed.sh http://localhost:3737

BASE_URL="${1:-http://localhost:3737}"
API_KEY="${2:-}"

AUTH_HEADER=""
if [ -n "$API_KEY" ]; then
  AUTH_HEADER="Authorization: Bearer $API_KEY"
fi

echo "🌱 Seeding KnowWhere at $BASE_URL"

# Helper function
post_session() {
  local content="$1"
  local memory_type="$2"
  local importance="$3"
  local sensitivity="${4:-normal}"

  curl -s -X POST "$BASE_URL/store_session" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "{
      \"content\": \"$content\",
      \"memory_type\": \"$memory_type\",
      \"importance\": $importance,
      \"sensitivity\": \"$sensitivity\"
    }" | jq -r '.id // .message'
}

# Helper for external pointers
post_external() {
  local pointer="$1"
  local memory_type="$2"
  local importance="$3"

  curl -s -X POST "$BASE_URL/store_external" \
    -H "Content-Type: application/json" \
    ${AUTH_HEADER:+-H "$AUTH_HEADER"} \
    -d "{
      \"pointer\": \"$pointer\",
      \"memory_type\": \"$memory_type\",
      \"importance\": $importance
    }" | jq -r '.id // .message'
}

echo ""
echo "📝 Storing episodic memories..."
post_session "Nimar mentioned today that he prefers async communication over synchronous meetings for non-urgent decisions." "episodic" 7
post_session "In the project kickoff meeting, Nimar decided to prioritize the PostgreSQL storage layer over the dashboard for v0.3." "episodic" 8
post_session "Nimar said his biggest challenge with AI consulting is convincing SMB clients that data privacy isn't a blocker for AI adoption." "episodic" 6
post_session "We discussed that the KnowWhere dashboard should feel like a premium tool, not a developer toy." "episodic" 7
post_session "Nimar mentioned he wants to target Handwerk (trades) businesses in NRW specifically." "episodic" 8

echo ""
echo "📚 Storing semantic memories..."
post_session "KnowWhere follows a Pointer-First architecture — external data sources are referenced, not copied." "semantic" 9 "normal"
post_session "The 5 memory types in KnowWhere are: episodic, semantic, preference, procedural, and meta." "semantic" 8 "normal"
post_session "KnowWhere uses Hybrid Retrieval: combining vector similarity (USearch), keyword search (BM25), and Reciprocal Rank Fusion." "semantic" 9 "normal"
post_session "Governance-before-Recall means every memory candidate is validated against policy before entering the prompt." "semantic" 9 "high"
post_session "The Dream Mode consists of two separate processes: Consolidation (building summaries) and Audit (checking for issues)." "semantic" 8 "normal"

echo ""
echo "❤️ Storing preference memories..."
post_session "Nimar prefers detailed technical explanations over simplified summaries when learning new concepts." "preference" 7
post_session "Nimar likes working in short, focused bursts rather than long uninterrupted sessions." "preference" 6
post_session "German is preferred for internal communication; English for external-facing content." "preference" 8 "normal"
post_session "Nimar prefers clear bullet points over long paragraphs in documentation." "preference" 7
post_session "Visual diagrams are appreciated for architecture discussions." "preference" 6

echo ""
echo "🔧 Storing procedural memories..."
post_session "To build KnowWhere: cargo build. To run: cargo run. Server starts on port 3737." "procedural" 9 "normal"
post_session "Set KNOWWHERE_API_KEY environment variable to enable authentication." "procedural" 8 "normal"
post_session "Set KNOWWHERE_DATA_DIR to change where memories are persisted (default: ./data)." "procedural" 7 "normal"
post_session "To enable Frigate connector: set FRIGATE_URL environment variable." "procedural" 6 "low"
post_session "Run cargo test to execute the test suite before pushing changes." "procedural" 8 "normal"

echo ""
echo "🏷️ Storing meta memories..."
post_session "This is a test memory — can be safely deleted after verifying the system works." "meta" 3 "low"
post_session "Confidence scores below 0.5 will be flagged by governance policy." "meta" 5 "normal"
post_session "The memory system is case-sensitive for memory_type (use lowercase: episodic, semantic, etc)." "meta" 5 "normal"

echo ""
echo "🔗 Storing external pointers..."
post_external "https://github.com/Jind0la/knowwhere" "semantic" 8
post_external "https://knowwhere.source_of_truth.pdf" "meta" 6
post_external "file:///home/nimar/projects/knowwhere/docs/PRD.md" "semantic" 7

echo ""
echo "✅ Done! Check the dashboard or call /nodes/recent to see stored memories."
echo "📊 Node count:"
curl -s "$BASE_URL/health" | jq .

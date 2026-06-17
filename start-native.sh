#!/bin/bash
# KnowWhere Native Start — sourced from launchd plist
# Reads secrets from ~/.knowwhere/.env and ~/.zshrc for API keys

set -e

ENV_FILE="$HOME/.knowwhere/.env"
if [ -f "$ENV_FILE" ]; then
  set -a
  source "$ENV_FILE"
  set +a
fi

# Source zshrc for API keys (VOYAGE_API_KEY, DEEPSEEK_API_KEY)
if [ -f "$HOME/.zshrc" ]; then
  set -a
  source "$HOME/.zshrc" 2>/dev/null
  set +a
fi

# Auto-detection: knows VOYAGE_API_KEY → Voyage, DEEPSEEK_API_KEY → DeepSeek
# Falls back to Ollama if keys not set. No hardcoded provider override.
export KNOWWHERE_DATA_DIR="./native_data"
export KNOWWHERE_RERANKER_MODEL_PATH="/Users/nimarfranklinmac/.cache/knowwhere/reranker/model.onnx"
export KNOWWHERE_RERANKER_TOKENIZER_PATH="/Users/nimarfranklinmac/.cache/knowwhere/reranker/tokenizer.json"

cd "$(dirname "$0")"
exec ./target/debug/knowwhere-server

#!/bin/bash
# KnowWhere Native Start — sourced from launchd plist
# Reads secrets from ~/.knowwhere/.env

set -e

ENV_FILE="$HOME/.knowwhere/.env"
if [ -f "$ENV_FILE" ]; then
  set -a
  source "$ENV_FILE"
  set +a
fi

export OLLAMA_URL="http://127.0.0.1:11434"
export KNOWWHERE_EMBEDDING_PROVIDER="ollama"
export KNOWWHERE_DATA_DIR="./native_data"
export KNOWWHERE_API_KEY="kw_testkey_12345"

cd "$(dirname "$0")"
exec ./target/release/knowwhere-server

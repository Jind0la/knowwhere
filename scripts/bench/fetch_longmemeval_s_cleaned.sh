#!/usr/bin/env bash
# Laedt longmemeval_s_cleaned.json von Hugging Face (~265 MB).
# Quelle: https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${ROOT}/benchmarks/hf/third_party/longmemeval/data/longmemeval_s_cleaned.json"
URL="https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json"

mkdir -p "$(dirname "${DEST}")"
echo "Downloading to ${DEST} ..."
curl -fsSL -o "${DEST}" "${URL}"
wc -c "${DEST}"
head -c 120 "${DEST}"; echo

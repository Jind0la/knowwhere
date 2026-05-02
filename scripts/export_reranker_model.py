#!/usr/bin/env python3
"""Export bge-reranker-v2-m3 to ONNX for KnowWhere Cross-Encoder Reranking.

One-time setup: pip install "optimum[exporters]" onnxruntime
Run: python3 scripts/export_reranker_model.py [output_dir]

Output:
  output_dir/model.onnx        (~2.2 GB)
  output_dir/tokenizer.json    (~1 MB)
  output_dir/config.json       (<1 KB)

Set env vars for KnowWhere:
  export KNOWWHERE_RERANKER_MODEL_PATH=/path/to/model.onnx
  export KNOWWHERE_RERANKER_TOKENIZER_PATH=/path/to/tokenizer.json
"""

import os
import subprocess
import sys
from pathlib import Path

MODEL_NAME = "BAAI/bge-reranker-v2-m3"
OUTPUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / ".cache" / "knowwhere" / "reranker"


def main():
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    model_onnx = OUTPUT_DIR / "model.onnx"
    tokenizer_json = OUTPUT_DIR / "tokenizer.json"

    if model_onnx.exists() and tokenizer_json.exists():
        print(f"✓ Model already exported at {OUTPUT_DIR}")
        print(f"  {model_onnx} ({model_onnx.stat().st_size / 1e9:.1f} GB)")
        return

    print(f"Exporting {MODEL_NAME} to ONNX...")
    print(f"Output directory: {OUTPUT_DIR}")
    print("This may take 5-10 minutes and requires ~5 GB disk space.")

    cmd = [
        sys.executable, "-m", "optimum.exporters.onnx",
        "--model", MODEL_NAME,
        "--task", "text-classification",
        "--optimize", "O3",
        str(OUTPUT_DIR),
    ]

    try:
        subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError:
        print("\nERROR: Export failed. Make sure you have:")
        print("  pip install 'optimum[exporters]' onnxruntime")
        print("\nIf O3 optimization fails, try without --optimize:")
        print(f"  optimum-cli export onnx --model {MODEL_NAME} --task text-classification {OUTPUT_DIR}")
        sys.exit(1)

    # Verify output
    onnx_size = model_onnx.stat().st_size if model_onnx.exists() else 0
    print(f"\n✓ Export complete!")
    print(f"  {model_onnx} ({onnx_size / 1e9:.1f} GB)")
    print(f"  {tokenizer_json}")
    print(f"\nSet these env vars for KnowWhere:")
    print(f"  export KNOWWHERE_RERANKER_MODEL_PATH={model_onnx}")
    print(f"  export KNOWWHERE_RERANKER_TOKENIZER_PATH={tokenizer_json}")


if __name__ == "__main__":
    main()

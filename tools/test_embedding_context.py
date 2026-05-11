#!/usr/bin/env python3
"""Empirically test context limits of Ollama embedding models.
Finds the prefix length where cos_sim crosses 0.999 (truncation point).
"""

import requests
import json
import math
import time
import sys

OLLAMA_URL = "http://127.0.0.1:11434"

def cosine_sim(emb1, emb2):
    dot = sum(a * b for a, b in zip(emb1, emb2))
    norm1 = math.sqrt(sum(a * a for a in emb1))
    norm2 = math.sqrt(sum(b * b for b in emb2))
    if norm1 == 0 or norm2 == 0:
        return 0.0
    return dot / (norm1 * norm2)

def get_embedding(model, text):
    """Get embedding from Ollama, with retries."""
    for attempt in range(3):
        try:
            r = requests.post(
                f"{OLLAMA_URL}/api/embed",
                json={"model": model, "input": text},
                timeout=120,
            )
            r.raise_for_status()
            data = r.json()
            emb = data["embeddings"][0]
            return emb
        except Exception as e:
            print(f"  Attempt {attempt+1}/3 failed: {e}")
            if attempt < 2:
                time.sleep(2)
    raise RuntimeError(f"Failed to embed with model {model}")

def get_model_info(model):
    """Get model details from Ollama show."""
    try:
        r = requests.post(f"{OLLAMA_URL}/api/show", json={"name": model}, timeout=10)
        if r.status_code == 200:
            return r.json()
    except Exception:
        pass
    return {}

def test_model(model, prefix_lengths=None):
    """Test a model's effective context window."""
    if prefix_lengths is None:
        prefix_lengths = [2000, 4000, 8000, 12000, 16000, 20000, 24000, 32000]

    suffix1 = " The user loves jazz music and plays saxophone every weekend."
    suffix2 = " The user hates all music and prefers complete silence always."

    # Pad suffixes to equal length
    pad = abs(len(suffix1) - len(suffix2))
    if len(suffix1) < len(suffix2):
        suffix1 += " " * pad
    else:
        suffix2 += " " * pad

    assert len(suffix1) == len(suffix2), f"Suffix lengths differ: {len(suffix1)} vs {len(suffix2)}"

    info = get_model_info(model)
    model_info_str = info.get("model_info", {})
    dim = None

    print(f"\n{'='*70}")
    print(f"Testing model: {model}")
    print(f"{'='*70}")

    results = []
    for prefix_len in prefix_lengths:
        doc1 = ("x" * prefix_len) + suffix1
        doc2 = ("x" * prefix_len) + suffix2

        print(f"  prefix={prefix_len:5d} chars...", end=" ", flush=True)

        t0 = time.time()
        emb1 = get_embedding(model, doc1)
        emb2 = get_embedding(model, doc2)
        elapsed = time.time() - t0

        if dim is None:
            dim = len(emb1)
            print(f"(dim={dim}) ", end="")

        sim = cosine_sim(emb1, emb2)

        if sim > 0.99999:
            status = "⚠️  IDENTICAL (truncated)"
        elif sim > 0.999:
            status = "⚠️  NEAR-IDENTICAL"
        elif sim > 0.99:
            status = "⚠️  VERY SIMILAR"
        else:
            status = f"✅ diff={1-sim:.4f}"

        print(f"cos_sim={sim:.6f}  {elapsed:.1f}s  {status}")
        results.append((prefix_len, sim, elapsed, dim))

        # If we've hit truncation twice in a row at high prefix lengths, stop
        if sim > 0.99999 and prefix_len >= 16000 and len(results) >= 2:
            if results[-2][1] > 0.99999:
                print(f"  -> Truncation confirmed at {prefix_len} chars. Stopping.")
                break

    return results, dim

def main():
    models = sys.argv[1:] if len(sys.argv) > 1 else None

    if models is None:
        # Default: test all available
        r = requests.get(f"{OLLAMA_URL}/api/tags", timeout=10)
        tags = r.json().get("models", [])
        # Filter to embedding models (not LLMs)
        embedding_keywords = ["embed", "bge", "nomic", "mxbai"]
        models = []
        for m in tags:
            name = m["name"]
            for kw in embedding_keywords:
                if kw in name.lower():
                    models.append(name)
                    break
        if not models:
            print("No embedding models found. Specify models as arguments.")
            return

    print(f"Models to test: {models}")

    all_results = {}
    for model in models:
        try:
            results, dim = test_model(model)
            all_results[model] = {"results": results, "dim": dim}
        except Exception as e:
            print(f"ERROR testing {model}: {e}")
            all_results[model] = {"error": str(e)}

    # Summary
    print(f"\n\n{'='*70}")
    print("SUMMARY")
    print(f"{'='*70}")
    for model, data in all_results.items():
        if "error" in data:
            print(f"{model}: ERROR - {data['error']}")
            continue
        results = data["results"]
        dim = data["dim"]
        truncation_point = "NONE"
        for prefix_len, sim, elapsed, _ in results:
            if sim > 0.99999:
                truncation_point = f"{prefix_len} chars"
                break
        if truncation_point == "NONE" and results:
            truncation_point = f">{results[-1][0]} chars (no truncation detected)"
        print(f"{model}: dim={dim}, truncation={truncation_point}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Test nomic-embed-text with num_ctx=8192 to override Ollama's default 2048."""
import requests, json, math, time

OLLAMA_URL = "http://127.0.0.1:11434"

def cosine_sim(emb1, emb2):
    dot = sum(a*b for a,b in zip(emb1,emb2))
    n1 = math.sqrt(sum(a*a for a in emb1))
    n2 = math.sqrt(sum(b*b for b in emb2))
    return dot/(n1*n2) if n1>0 and n2>0 else 0

model = "nomic-embed-text:latest"
suffix1 = " The user loves jazz music and plays saxophone every weekend."
suffix2 = " The user hates all music and prefers complete silence always."
pad = abs(len(suffix1)-len(suffix2))
if len(suffix1)<len(suffix2): suffix1 += ' '*pad
else: suffix2 += ' '*pad

prefix_lengths = [2000, 4000, 8000, 12000, 16000, 20000, 24000, 32000]

print(f"Testing {model} with num_ctx=8192")
for prefix_len in prefix_lengths:
    doc1 = ('x'*prefix_len) + suffix1
    doc2 = ('x'*prefix_len) + suffix2
    print(f"  prefix={prefix_len:5d} chars...", end=" ", flush=True)
    t0 = time.time()
    r1 = requests.post(f"{OLLAMA_URL}/api/embed", json={
        "model": model, "input": doc1,
        "options": {"num_ctx": 8192}
    }, timeout=120)
    r2 = requests.post(f"{OLLAMA_URL}/api/embed", json={
        "model": model, "input": doc2,
        "options": {"num_ctx": 8192}
    }, timeout=120)
    elapsed = time.time()-t0
    emb1 = r1.json()['embeddings'][0]
    emb2 = r2.json()['embeddings'][0]
    sim = cosine_sim(emb1, emb2)
    if sim > 0.99999: status = "⚠️  IDENTICAL"
    elif sim > 0.999: status = "⚠️  NEAR-ID"
    elif sim > 0.99: status = "⚠️  VERY SIMILAR"
    else: status = f"✅ diff={1-sim:.4f}"
    print(f"cos_sim={sim:.6f} dim={len(emb1)} {elapsed:.1f}s {status}")
    if sim > 0.99999 and prefix_len >= 16000:
        print("  -> Truncation confirmed. Stopping.")
        break

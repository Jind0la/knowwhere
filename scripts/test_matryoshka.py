#!/usr/bin/env python3
"""Test Matryoshka embedding: compare full 768d vs truncated cosine similarities."""
import requests, numpy as np, sys

texts = [
    "Redis zum Cachen von User-Sessions",
    "Redis als Message-Queue für Jobs",
    "PostgreSQL als Message-Queue nutzen",
    "Ein komplett anderes Thema über Steuererklärung",
    "Noch ein Steuerthema: Freibetrag 2026",
]

resp = requests.post("http://localhost:11434/api/embed",
    json={"model": "nomic-embed-text:latest", "input": texts}, timeout=30)
embeddings = [np.array(e) for e in resp.json()["embeddings"]]

def cos(a, b): 
    return float(np.dot(a,b)/(np.linalg.norm(a)*np.linalg.norm(b)))

trunc_dims = [64, 128, 256, 512, 768]
print(f"{'Pair':>6}  Redis/Cache↔Redis/Queue  Redis/Cache↔Steuer  Redis/Queue↔Postgres/Queue")
print(f"{'full 768d'}  " + "  ".join(f"         {cos(embeddings[i][:768], embeddings[j][:768]):.4f}" for i,j in [(0,1),(0,3),(1,2)]))

for dim in [512, 256, 128, 64]:
    trunc_sims = []
    for i, j in [(0,1),(0,3),(1,2)]:
        sim = cos(embeddings[i][:dim], embeddings[j][:dim])
        trunc_sims.append(sim)
    ratios = [t/cos(embeddings[i][:768], embeddings[j][:768]) for t,(i,j) in zip(trunc_sims, [(0,1),(0,3),(1,2)])]
    print(f"trunc {dim:>3}d  " + "  ".join(f"   {s:.4f}" for s in trunc_sims) + "  ratios: " + " ".join(f"{r:.2f}" for r in ratios))

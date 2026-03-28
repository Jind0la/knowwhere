#!/usr/bin/env python3
"""
KnowWhere Integration Test — MilaOS Design Journey
Tests: store → embed → retrieve → fractal zoom
"""

import json
import time
import requests

BASE = "http://localhost:3737"
TOKEN = "kw-test-key-2026"
HEADERS = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}

def api(path, method="GET", data=None):
    url = BASE + path
    kwargs = {"headers": HEADERS}
    if data:
        kwargs["json"] = data
    r = requests.request(method, url, **kwargs)
    try:
        return r.status_code, r.json()
    except:
        return r.status_code, r.text


# ───────────────────────────────────────────────────────────────
# PHASE 0: Clear existing nodes
# ───────────────────────────────────────────────────────────────
print("=" * 60)
print("PHASE 0: Clear existing nodes")
print("=" * 60)
status, _ = api("/nodes/purge_dummy", "POST")
print(f"Purge dummy: {status}")
_, health = api("/health")
print(f"Health: {health}")

# ───────────────────────────────────────────────────────────────
# PHASE 1: Store 30 test nodes — MilaOS Design Journey
# ───────────────────────────────────────────────────────────────
print("\n" + "=" * 60)
print("PHASE 1: Storing 30 nodes — MilaOS Design Journey")
print("=" * 60)

messages = [
    # Bucket A — Projektstart, Vision
    {"content": "Ich will einen anonymen Smart-Home-Assistenten bauen. Kein Login, keine Cloud, keine Nutzerkonten. Alles local auf dem eigenen Server.", "bucket": "A"},
    {"content": "Das Projekt heißt MilaOS. Datenschutz first. Keine Google, keine Amazon Dienste.", "bucket": "A"},
    {"content": "MilaOS Zielgruppe: Menschen die Privatsphäre im Smart Home wollen. Keine Angst vor Technik aber misstrauisch gegen Big Tech.", "bucket": "A"},
    {"content": "Erste Skizze: MilaOS als Python-App mit lokaler SQLite-DB. Interface über Webbrowser.", "bucket": "A"},
    {"content": "Budget für Server: maximal 50 Euro pro Monat. Muss auf einem Raspberry Pi laufen können.", "bucket": "A"},
    # Bucket B — Design-Entscheidungen
    {"content": "Farbschema für MilaOS: Dunkelgrau (#1a1a2e) als Hauptfarbe, Amber (#f59e0b) als Akzent. Nicht zu hell, augenschonend.", "bucket": "B"},
    {"content": "Design-Prinzip: Minimalistisch. Keine Charts, keine Grafiken, nur Text und einfache Icons. Fokus auf Funktion.", "bucket": "B"},
    {"content": "Wireframes für iOS Interface: Home-Screen mit Geräte-Liste, ein Detail-Screen pro Gerät, Settings als simples Menu.", "bucket": "B"},
    {"content": "Typografie: Inter Font. Lesbar, modern, Open Source. 16px Basisgröße.", "bucket": "B"},
    {"content": "Logo-Idee: Minimalistisches M in Dunkelgrau mit Amber Punkt. Nicht zu verspielt.", "bucket": "B"},
    {"content": "Design Revision: doch nicht Inter, sondern IBM Plex Sans. Besser für technische Interfaces.", "bucket": "B"},
    {"content": "Neue Farbpalette: Slate-900 (#0f172a) mit Cyan-Akzent (#22d3ee). Wirkt professioneller.", "bucket": "B"},
    {"content": "Dashboard: Eine Seite, alle Geräte auf einen Blick. Keine Navigation nötig.", "bucket": "B"},
    # Bucket C — Technologie-Stack
    {"content": "Backend: Rust mit Axum Framework. Schnell, sicher, keine GC-Pausen. Passt zum minimalistischen Ansatz.", "bucket": "C"},
    {"content": "Embedding: nomic-embed-text-v2-moe über Ollama. 768 Dimensionen, lokale Ausführung.", "bucket": "C"},
    {"content": "Vector Search: USearch für semantische Ähnlichkeitssuche. HNSW Index, cosine similarity.", "bucket": "C"},
    {"content": "Keyword Search: BM25 für exakte Term-Matches. German-optimiert mit Stop-Word-Filter.", "bucket": "C"},
    {"content": "Fusion: Reciprocal Rank Fusion (k=60) kombiniert Vector und BM25 Ergebnisse.", "bucket": "C"},
    {"content": "Persistenz: PostgreSQL für relationale Daten, state.json für Vektoren. Nicht alles in eine DB.", "bucket": "C"},
    # Bucket D — Rückschläge
    {"content": "Frigate Integration hat drei Wochen gedauert. NVR API ist schlecht dokumentiert. Am Ende nur Pointer gespeichert, keine Rohbilder.", "bucket": "D"},
    {"content": "Ollama Embedding Qualität war zunächst mäßig. Wechsel auf nomic-embed-text-v2-moe hat viel verbessert.", "bucket": "D"},
    {"content": "Erstes UI war zu kompliziert. Zu viele Features auf einmal. Minimalismus hat länger gedauert als erwartet.", "bucket": "D"},
    {"content": "Raspberry Pi 4 war zu langsam für Rust + PostgreSQL. Umgestiegen auf Mini-PC mit Intel N100.", "bucket": "D"},
    {"content": "Einbruchserkennung über Frigate hat zu viele False Positives. Schwelle höher gesetzt, jetzt besser.", "bucket": "D"},
    # Bucket E — Team-Entscheidungen
    {"content": "Nimar macht Design und PM allein. Brauche keinen großen Team. Lieber langsam aber mit Kontrolle.", "bucket": "E"},
    {"content": "Agent Max als Projektleiter definiert. Max koordiniert Tasks und erinnert an Deadlines. Max lebt in einem eigenen Agent.", "bucket": "E"},
    {"content": "Wöchentliche Reviews jeden Sonntag Abend. Max bereitet Review-Dokument vor, Nimar entscheidet.", "bucket": "E"},
    {"content": "Kein Agent soll allein entscheiden. Immer ein Mensch in the Loop. Das ist nicht verhandelbar.", "bucket": "E"},
    # Bucket F — Aktuelle Gedanken
    {"content": "Launch-Ziel: Ende April 2026. Erst private Beta, dann öffentlich wenn stabilized.", "bucket": "F"},
    {"content": "Suche erste Beta-Tester. 5-10 Personen die MilaOS auf echter Hardware testen. Feedback-Kanal über Telegram.", "bucket": "F"},
]

store_times = []
node_ids = []

for i, msg in enumerate(messages):
    start = time.time()
    status, resp = api("/store_session", "POST", {
        "content": msg["content"],
        "metadata": {"source": "user:Nimar", "session_id": "milaos-design-journey", "bucket": msg["bucket"]}
    })
    elapsed = (time.time() - start) * 1000
    store_times.append(elapsed)
    ok = status in (200, 201)
    if ok:
        node_id = resp.get("id", "?")
        node_ids.append(node_id)
        bucket = msg["bucket"]
        content_preview = msg["content"][:55] + "..."
        print(f"  [{i+1:02d}] ✓ Bucket {bucket} | {elapsed:.0f}ms | {content_preview}")
    else:
        print(f"  [{i+1:02d}] ✗ FAILED {status}: {str(resp)[:100]}")

avg_store = sum(store_times)/len(store_times) if store_times else 0
min_store = min(store_times) if store_times else 0
max_store = max(store_times) if store_times else 0
print(f"\n  Store Stats: avg={avg_store:.0f}ms, min={min_store:.0f}ms, max={max_store:.0f}ms")

# ───────────────────────────────────────────────────────────────
# PHASE 2: Retrieval Queries
# ───────────────────────────────────────────────────────────────
print("\n" + "=" * 60)
print("PHASE 2: Retrieval Queries — 10 Tests")
print("=" * 60)

queries = [
    ("Was war unsere erste Idee für das Projekt?", "A"),
    ("Welche Farben haben wir für das Design gewählt?", "B"),
    ("Warum haben wir uns gegen Cloud-Login entschieden?", "A+D"),
    ("Was war das größte technische Problem?", "D"),
    ("Welches Embedding-Modell nutzen wir?", "C"),
    ("Wie ist das Budget verteilt?", "D+E"),
    ("Wer ist Max im Team?", "E"),
    ("Wann ist der Launch geplant?", "F"),
    ("Erzähl mir von den Wireframes", "B"),
    ("Was waren die wichtigsten Entscheidungen?", "A+B+C+E"),
]

retrieval_results = []

for i, (query_text, expected_buckets) in enumerate(queries):
    start = time.time()
    status, resp = api("/retrieve_fractal", "POST", {
        "query_text": query_text,
        "top_k": 5,
        "max_depth": 2
    })
    elapsed = (time.time() - start) * 1000

    if not isinstance(resp, list):
        print(f"\n  [{i+1:02d}] ✗ Query FAILED {status}: {str(resp)[:200]}")
        retrieval_results.append({"query": query_text, "status": status, "error": True})
        continue

    results = resp
    scores = [r.get("score", 0) for r in results]
    result_buckets = [r.get("metadata", {}).get("bucket", "?") for r in results]

    top3_buckets = result_buckets[:3]
    matched = any(b in expected_buckets for b in top3_buckets)
    top1_bucket = result_buckets[0] if result_buckets else "?"
    top1_score = scores[0] if scores else 0

    retrieval_results.append({
        "query": query_text,
        "expected": expected_buckets,
        "top3_buckets": top3_buckets,
        "scores": scores,
        "matched": matched,
        "top1_score": top1_score,
        "elapsed_ms": elapsed
    })

    match_icon = "✓" if matched else "✗"
    print(f"\n  [{i+1:02d}] {match_icon} Query: {query_text}")
    print(f"       Expected buckets: {expected_buckets}")
    print(f"       Top-3 buckets:   {top3_buckets}")
    print(f"       Scores: {[f'{s:.4f}' for s in scores]}")
    print(f"       Top-1: {top1_score:.4f} | Time: {elapsed:.0f}ms")

    if results:
        top = results[0]
        content = top.get("content", "") or ""
        print(f"       Top-1 content: {content[:80]}...")

# ───────────────────────────────────────────────────────────────
# PHASE 3: Fractal Zoom
# ───────────────────────────────────────────────────────────────
print("\n" + "=" * 60)
print("PHASE 3: Fractal Zoom Tests")
print("=" * 60)

_, parent = api("/store_session", "POST", {
    "content": "MilaOS Design Guide Version 1.0 — Das vollständige Design-Handbuch",
    "metadata": {"source": "user:Nimar", "type": "design_guide_parent"}
})
_, c1 = api("/store_session", "POST", {
    "content": "Farbschema: Slate-900 (#0f172a) Hintergrund, Cyan (#22d3ee) Akzent, Slate-400 Text",
    "metadata": {"source": "user:Nimar", "type": "design_child"}
})
_, c2 = api("/store_session", "POST", {
    "content": "Typografie: IBM Plex Sans, 16px Basis, Überschriften 24px bold",
    "metadata": {"source": "user:Nimar", "type": "design_child"}
})
_, c3 = api("/store_session", "POST", {
    "content": "Layout: Single-Page Dashboard, keine Navigation, Geräte als Karten",
    "metadata": {"source": "user:Nimar", "type": "design_child"}
})

print(f"  Parent: {str(parent)[:60]}...")
print(f"  Child 1: {str(c1)[:60]}...")
print(f"  Child 2: {str(c2)[:60]}...")
print(f"  Child 3: {str(c3)[:60]}...")

_, zoom_resp = api("/retrieve_fractal", "POST", {
    "query_text": "Was steht im Design Guide über Farben und Layout?",
    "top_k": 5,
    "max_depth": 3
})

if isinstance(zoom_resp, list) and zoom_resp:
    print(f"\n  Zoom Results ({len(zoom_resp)} nodes):")
    for r in zoom_resp[:5]:
        c = r.get("content", "") or ""
        s = r.get("score", 0)
        print(f"    score={s:.4f} | {c[:70]}...")
elif isinstance(zoom_resp, list):
    print("\n  Zoom: empty results")
else:
    print(f"\n  Zoom: unexpected response {type(zoom_resp)}: {str(zoom_resp)[:100]}")

# ───────────────────────────────────────────────────────────────
# PHASE 4: Edge Cases
# ───────────────────────────────────────────────────────────────
print("\n" + "=" * 60)
print("PHASE 4: Edge Cases")
print("=" * 60)

# Test: empty query string
status, resp = api("/retrieve_fractal", "POST", {"query_text": "", "top_k": 3})
print(f"  Empty query_text: status={status}, type={type(resp).__name__}")

# Test: sinnlose query
status, resp = api("/retrieve_fractal", "POST", {"query_text": "Atomkernfusion im Wohnzimmer mit Delphinen", "top_k": 3})
results = resp if isinstance(resp, list) else []
top_score = results[0].get("score", 0) if results else 0
print(f"  Sinnlose query: {status}, {len(results)} results, top_score={top_score:.4f}")

# Test: sehr lange nachricht
status, resp = api("/store_session", "POST", {"content": "A" * 2000})
print(f"  Lange Nachricht (2000 chars): {status}")

# Test: Sonderzeichen / Emoji
status, resp = api("/store_session", "POST", {"content": "App mit Speicher und Netzwerk und Robotern sowie Emojis Gebaude und Pfeile"})
print(f"  Sonderzeichen: {status}")

# Test: Cross-Topic Query
status, resp = api("/retrieve_fractal", "POST", {"query_text": "Budget und Design zusammen", "top_k": 5})
cross_results = resp if isinstance(resp, list) else []
cross_buckets = [r.get("metadata", {}).get("bucket", "?") for r in cross_results]
print(f"  Cross-Topic (Budget+Design): {status}, buckets in top-5: {cross_buckets}")

# ───────────────────────────────────────────────────────────────
# PHASE 5: System Integrity
# ───────────────────────────────────────────────────────────────
print("\n" + "=" * 60)
print("PHASE 5: System Integrity")
print("=" * 60)

_, health = api("/health")
_, recent = api("/nodes/recent?limit=10")

print(f"  Health: {health}")
print(f"  Node count: {health.get('node_count', '?')}")
print(f"  Recent nodes returned: {len(recent) if isinstance(recent, list) else 'N/A'}")

# Embedding test
status, emb = api("/embed", "POST", {"text": "Test embedding"})
emb_dim = len(emb.get("embedding", [])) if isinstance(emb, dict) else 0
print(f"  Embedding dimension: {emb_dim} (expected 768)")

# ───────────────────────────────────────────────────────────────
# FINAL REPORT
# ───────────────────────────────────────────────────────────────
print("\n" + "=" * 60)
print("FINAL REPORT — KnowWhere Integration Test")
print("=" * 60)

successful_stores = len(node_ids)
avg_store_ms = sum(store_times) / len(store_times) if store_times else 0

valid_results = [r for r in retrieval_results if not r.get("error", False)]
retrieval_matched = sum(1 for r in valid_results if r.get("matched", False))
retrieval_total = len(valid_results)
avg_retrieval_ms = sum(r["elapsed_ms"] for r in valid_results) / len(valid_results) if valid_results else 0
avg_top1_score = sum(r["top1_score"] for r in valid_results) / len(valid_results) if valid_results else 0

recall_pct = 100 * retrieval_matched / retrieval_total if retrieval_total else 0

print(f"""
Nodes stored:         {successful_stores}/30
Store latency:        avg={avg_store_ms:.0f}ms

Retrieval:
  Queries tested:      {retrieval_total}/10
  Matched (Precision@3): {retrieval_matched}/{retrieval_total} ({recall_pct:.0f}%)
  Avg retrieval time:    {avg_retrieval_ms:.0f}ms
  Avg top-1 score:       {avg_top1_score:.4f}

Embedding dimension:   {emb_dim} (expected 768) {'✓' if emb_dim == 768 else '✗'}

Go/No-Go:
""")

if recall_pct >= 80:
    verdict = "✓ GO — OpenClaw Integration kann starten"
elif recall_pct >= 60:
    verdict = "~ CONDITIONAL — Retrieval-Parameter tune nötig"
else:
    verdict = "✗ NO-GO — Retrieval muss gefixt werden"

print(f"  {verdict}")
print()

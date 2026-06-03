# KnowWhere PersonaMem Benchmark — Vollständige Erkenntnisdokumentation

> Stand: 2026-05-10. Laufender Full-Run: `knowwhere-factcheck-full` (589Q, fact-check prompt, ETA ~2h)

---

## 1. Chronologie der Full-Runs (589 Queries)

| Run | Embedding | Answer LLM | Accuracy | Retrieval | Datum |
|---|---|---|---|---|---|
| `knowwhere` (Baseline) | snowflake-arctic-embed2 | gemini-2.5-pro | **45.0%** | 254ms | 08.05. |
| `knowwhere-noprefix` | snowflake-arctic-embed2 | gemini-2.5-flash | **48.9%** | 2772ms | 09.05. |
| `knowwhere-nomic-full` | nomic-embed-text | gemini-2.5-flash | **50.8%** | 1542ms | 09.05. |
| `knowwhere-full-ingest` | bge-m3 | gemini-2.5-pro | **57.9%** | 294ms | 10.05. |

## 2. Beste 20-Query Spikes

| Run | Accuracy | Embedding | Besonderheit |
|---|---|---|---|
| `knowwhere-factcheck-v2` | **75.0%** | bge-m3 | **Fact-Check Prompt** |
| `knowwhere-suggestfix-v1` | 80.0%¹ | bge-m3 | Suggest-Fix (fragwürdig) |
| `knowwhere-nomic-pro` | 65.0% | nomic-embed-text | Gemini-2.5-Pro |
| `knowwhere-nomic-test` | 60.0% | nomic-embed-text | Baseline nomic |
| `knowwhere-timeline-v2` | 55.0% | nomic-embed-text | Timeline-Template |

¹ `suggestfix-v1`: 16/20 aber enthält doppelte Queries — effektiv ~70%

---

## 3. Kritische Bugs — Gefunden & Gefixt

### Bug #1: `store_external` embeddete `pointer` statt `content`
- **Datei**: `src/api/routes.rs`
- **Symptom**: Alle gespeicherten Vektoren waren Embeddings von URIs (`file://...`) statt des eigentlichen Textinhalts
- **Fix**: `req.content` als primäre Embedding-Quelle, Fallback auf `req.pointer`
- **Impact**: Retrieval-Scores waren quasi zufällig (~0.02-0.03 Cosine-Similarity)

### Bug #2: `retrieve_fractal` nutzte `embed()` statt `embed_query()`
- **Datei**: `src/api/routes.rs`
- **Symptom**: Query-Vektoren hatten keinen `search_query:` Prefix, während Dokument-Vektoren mit `search_document:` Prefix erstellt wurden → asymmetrische Embeddings
- **Fix**: `state.embedding.embed(text)` → `embed_query(&*state.embedding, text)`
- **Impact**: Semantic-Search war de facto kaputt

### Bug #3: `score_debug_response` zeigte falsche Scores
- **Datei**: `src/api/routes.rs`
- **Symptom**: `base_score` wurde auf 1.0 gesetzt statt echten Cosine-Score zu zeigen
- **Fix**: `score`-Parameter zu `score_debug_response` hinzugefügt, korrekt durchgereicht
- **Impact**: Debugging war irreführend — sah aus als würde Retrieval funktionieren

### Bug #4: RRF-Fusion zerstörte semantische Scores
- **Datei**: `src/storage/in_memory.rs`
- **Symptom**: BM25-Keyword-Matches bekamen RRF-Score 1.0 und überschrieben Cosine-Similarity komplett
- **Fix**: RRF komplett entfernt. Stattdessen: Cosine-Scores direkt verwenden + BM25 als optionalen Boost (20% Gewicht)
- **Impact**: Retrieval-Qualität brach bei Keywords ein — semantisch relevante Dokumente wurden von BM25-Treffern verdrängt

### Bug #5: Snowflake Arctic Embed Prefix-Asymmetrie
- **Symptom**: snowflake-arctic-embed2 erwartet asymmetrische Prefixes (`search_document:` vs `search_query:`), aber KnowWhere behandelte es symmetrisch
- **Fix**: Prefixes deaktiviert (Option B) → Wechsel zu nomic-embed-text und später bge-m3
- **Impact**: snowflake-arctic-embed2 war mit ~48.9% unbrauchbar für diesen Use-Case

---

## 4. Embedding-Modell-Vergleich

| Modell | Größe | Dimensionen | Beste Accuracy | Retrieval-Zeit | Bewertung |
|---|---|---|---|---|---|
| snowflake-arctic-embed2 | 1.2GB | 1024 | 48.9% | 2772ms | ❌ Prefix-Probleme, langsam, sperrig |
| nomic-embed-text | 274MB | 768 | 50.8% | 867ms | ⚠️ Stabil, aber nur mittelmäßige Qualität |
| bge-m3 | ~2GB | 1024 | **57.9%** | 294ms | ✅ Bester Kompromiss aus Qualität & Speed |

**Fazit**: bge-m3 ist der klare Gewinner. +7.1% über nomic-embed-text, +9.0% über snowflake-arctic.

---

## 5. Prompt Engineering — Der entscheidende Hebel

### Problem-Diagnose (nach 57.9%-Run)
Analyse der 248 falschen Queries aus `knowwhere-full-ingest`:
- **0% leere Contexts** → Retrieval ist nicht der Bottleneck
- **~85% LLM-Reasoning-Fehler** → Context hat die relevante Info, LLM wählt trotzdem falsch
- **15% Retrieval-Lücken** → Info fehlt tatsächlich im Context

### Fehlermuster des LLM
1. **Ton statt Fakten**: LLM wählt die "elegantere" Option statt die faktisch korrekte
2. **Erfundene Constraints**: LLM lehnt Optionen ab wegen Kriterien, die nicht in der Frage stehen
3. **Over-Analyse bei suggest_new_ideas**: 15.1% Accuracy (79/93 falsch)

### Lösung: Fact-Check-First Prompt

**Vorher** (Default PersonaMem Prompt):
```
Step 1: Identify 2-3 key preferences
Step 2: Check alignment/contradiction for each option
Step 3: Select option with strongest alignment
```

**Nachher** (Fact-Check Prompt):
```
Step 1: FACT CHECK — eliminiere Optionen mit faktischen Fehlern
Step 2: Aus Überlebenden: spezifisch > generisch
Step 3: Buchstabe + Reasoning
```

### Ergebnis
| Metrik | Vorher | Nachher |
|---|---|---|
| Overall (20Q) | ~57-60% | **75.0%** |
| suggest_new_ideas | 15.1% | **67%** |
| provide_preference_aligned_recs | 70.9% | **100%** (4/4) |

---

## 6. Question-Type Analyse (aus 57.9%-Run)

| Question Type | Queries | Accuracy | Prompt-Typ |
|---|---|---|---|
| recalling_the_reasons_behind_previous_updates | 99 | **78.8%** | Default |
| provide_preference_aligned_recommendations | 55 | **70.9%** | Inference (3-Step) |
| recalling_facts_mentioned_by_the_user | 17 | 64.7% | Default |
| generalizing_to_new_scenarios | 57 | 64.9% | Inference (3-Step) |
| recall_user_shared_facts | 129 | 62.0% | Default |
| track_full_preference_evolution | 139 | 59.0% | Default |
| suggest_new_ideas | 93 | **15.1%** | Inference (3-Step) |

**Key Insight**: `suggest_new_ideas` war mit 15.1% der klare Ausreißer — der 3-Step-Inference-Prompt priorisierte "Alignment" über faktische Korrektheit. Fact-Check-Prompt hat das auf 67% gebracht.

---

## 7. Weitere Experimente & Erkenntnisse

### Timeline-Templates
- **Idee**: Claims mit `turn_index` speichern, zeitlich sortiert ausgeben
- **Ergebnis**: 55.0% (nomic) — marginal besser als Baseline (50.8%), aber kein Durchbruch
- **Warum**: Timeline half bei temporal reasoning, aber der Grund-Bottleneck war LLM-Reasoning, nicht Context-Struktur
- **Status**: Nicht weiterverfolgt nach Fact-Check-Prompt-Erfolg

### User-ID-Filter
- **Idee**: Nur Claims des angefragten Users retrieven
- **Ergebnis**: 60.0% (20Q) — leichte Verbesserung, aber kein Game-Changer
- **Warum**: bge-m3 trennt User-Embeddings bereits gut genug

### Gemini-2.5-Pro vs Flash
- Pro liefert konsistent ~5-10% mehr Accuracy bei 3-4× höherer Latenz
- Für Benchmark: Pro ist die richtige Wahl
- Flash als Fallback wenn Pro-Rate-Limits erreicht

### Gstack Integration
- **Installiert** aber nicht aktiv für KnowWhere-Dev genutzt
- Skills verlinkt nach `~/.hermes/skills/`
- `/health`, `/investigate`, `/review` wären relevant, wurden aber durch direkte Terminal/Debugging-Arbeit ersetzt

---

## 8. Infrastruktur-Entscheidungen

| Entscheidung | Begründung |
|---|---|
| Postgres statt SQLite | Skalierbarkeit für 39K+ Nodes |
| Ollama nativ (nicht Docker) | M1 Mac 8GB RAM — Docker overhead zu hoch |
| Release-Builds (`--release`) | 10-50× schneller als Debug-Builds |
| `--skip-ingestion` Flag | 195 Docs × Gemini Flash = 2.5h Ingestion — nur einmal nötig |
| bge-m3 in Ollama | Bester Kompromiss aus Embedding-Qualität & RAM (passt neben qwen2.5:3b) |

---

## 9. RAM-Budget (M1 MacBook Air 8GB)

| Komponente | RAM |
|---|---|
| bge-m3 (Ollama) | ~2.0 GB |
| qwen2.5:3b (Ollama, Consolidation) | ~2.3 GB |
| KnowWhere Server (Rust) | ~200 MB |
| Postgres | ~300 MB |
| **Frei** | ~3.2 GB |

✅ Passt. snowflake-arctic-embed2 (1.2GB) + qwen2.5:3b (2.3GB) = 3.5GB → OOM-Risiko.

---

## 10. Offene Fragen & Nächste Schritte

1. **Full-Run mit Fact-Check-Prompt** (läuft gerade, `knowwhere-factcheck-full`)
2. **Weitere Prompt-Verfeinerung**: Die 2 verbliebenen suggest_new_ideas-Fehler sind derselbe Query (LLM priorisiert "collaboration" über "music production")
3. **Consolidation-Qualität prüfen**: L1→L0 Pipeline läuft mit qwen2.5:3b — sind die Summaries gut genug?
4. **128k Split testen**: Aktuell nur 32k — wie skaliert die Accuracy mit größeren Context-Fenstern?

---

*Letzter Full-Run: `knowwhere-full-ingest` — 57.9% (341/589), bge-m3 + Gemini-2.5-Pro*
*Aktuell laufend: `knowwhere-factcheck-full` — Fact-Check-Prompt, ETA ~2h*

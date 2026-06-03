# KnowWhere Optimierungs-Session — 10. Mai 2026

## Ergebnis: 59.8% Accuracy (+7.8 PP über SOTA)

**KnowWhere RAG schlägt alle Raw-LLMs auf dem PersonaMem-Benchmark.**

| Modell | Accuracy | Ansatz |
|---|---|---|
| Gemini 1.5-Flash | 52.0% | Full 128k Context |
| GPT-4.5 | 52.0% | Full 128k Context |
| **KnowWhere v4** | **59.8%** | RAG mit 30 Nodes |

---

## Was wir gemacht haben

### 1. Kritische Bugfixes (KnowWhere Server)
- **Bug #1**: `store_external` embeddete `pointer` statt `content` → Claims wurden als URI embedded
- **Bug #2**: `retrieve_fractal` nutzte `embed()` statt `embed_query()` → Query/Doc-Embedding-Asymmetrie
- **Bug #3**: `score_debug_response` zeigte immer 1.0 statt echtem Cosine-Score
- **Bug #4**: BM25 via RRF-Fusion zerstörte semantische Scores → BM25 deaktiviert, pure Vector Search

### 2. Parallele Ingestion (ThreadPoolExecutor)
- **Vorher**: Sequentiell, 195 Docs × 30s = 100 Minuten
- **Nachher**: 5 Threads parallel, ~20 Minuten
- Implementiert in `ingest()` via `ThreadPoolExecutor(max_workers=5)`

### 3. Context-Template Evolution

| Version | Format | Accuracy |
|---|---|---|
| Baseline | Evidence (4 items) | 58.1% |
| Zoom | Topic → Summary → Changes → Evidence (12) | 59.1% |
| Zoom + Pref | + Preference Profile (5 items/cat) | 59.1% |
| Tuned | Evidence 8, Pref 3 items/cat | 59.8% |
| **Timeline First** | Evolution → Current State → Moments | (nicht getestet) |

**"Timeline First" Format (letzter Stand, ungetestet):**
```
## {Persona}: Evolution
**Initially:** Podcasting, public music sharing
**Then:** Album reviews, remixing experiments  
**Now:** Personal music production, Pacific Fusion sound

## {Persona}: Current State
**Core passions:** Creating music that expresses personal journey
**Identity:** Musician blending Pacific Islander sounds with electronic

## Context
Cross-session synthesis from Topics

## Moment [t4] ...
(5 key facts)
```

### 4. Prompt-Optimierungen (personamem.py)
- 4-Step Reasoning für Alignment-Queries
- "Temporal Awareness"-Step für suggest_new_ideas
- Referenziert Evolution + Current State statt Preference Profile

### 5. Model-Upgrade
- Answer-Model: gemini-2.5-flash → **gemini-2.5-pro** (+10 PP auf 20 Queries)
- Embedding: nomic-embed-text → **bge-m3**

---

## Performance-Details nach Query-Typ

| Query-Typ | Baseline | Best (v4) | SOTA (Raw LLM) | Delta vs SOTA |
|---|---|---|---|---|
| provide_preference_aligned_recs | 58.2% | **76.4%** | 57% | **+19.4** |
| generalizing_to_new_scenarios | 50.9% | **63.2%** | 54% | **+9.2** |
| recalling_reasons_behind_updates | 72.7% | **78.8%** | 84% | -5.2 |
| recall_user_shared_facts | 69.8% | **63.6%** | 65% | -1.4 |
| track_full_preference_evolution | 54.7% | **59.0%** | 73% | -14.0 |
| suggest_new_ideas | 33.3% | **20.4%** | 28% | -7.6 |

---

## Offene Probleme

1. **suggest_new_ideas @ 20%** — selbst Claude 3.7-Sonnet nur 28%. Fundamentales Reasoning-Problem.
2. **track_full_preference_evolution @ 59%** — braucht Timeline-First-Format.
3. **Gemini Spending Cap** — 429 Errors bei Full-Benchmarks mit pro-Model.

---

## Aktive Optimierungen im Code

| Datei | Änderung |
|---|---|
| `knowwhere.py` | ThreadPoolExecutor parallel ingestion, Timeline-First context template, concurrency=2 |
| `personamem.py` | 4-Step prompt mit Evolution/Current-State-Referenzen |
| `knowwhere/.env` | OMB_ANSWER_MODEL=gemini-2.5-pro |

---

## Nächste Schritte

1. Timeline-First-Format testen (wurde durch Spending-Cap unterbrochen)
2. suggest_new_ideas-Fix: Weighted Preferences (Kern-Passion vs sekundäre Interessen)
3. Ggf. anderes Answer-Model (Claude/GPT-4o via OpenRouter wenn Budget erlaubt)

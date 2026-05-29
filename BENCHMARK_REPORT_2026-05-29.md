# KnowWhere v0.6 — Benchmark-Report

**Datum:** 2026-05-29  
**Branch:** `rm-summarizer-matryoshka-zoom` (gemerged nach `main`)  
**Server:** PostgreSQL, localhost:3738  
**Embedding:** nomic-embed-text-v2-moe (Matryoshka 768d → 256d/64d Truncation)  
**Architektur-Status:** Summarizer ENTFERNT (~4K LOC), Consolidation DEAKTIVIERT, Fractal Zoom auf Matryoshka umgestellt  

---

## 1. LongMemEval S — Retrieval Quality

### Setup
- **Dataset:** LongMemEval S (single-session-user), 30 Cases
- **Modus:** Multi-Session (alle Sessions indexieren → alle Queries ausführen)
- **Endpoint:** `retrieve_fractal` mit `governance_enabled=true`, `retrieval_profile=full-fidelity`
- **Pro Case:** ~53 Haystack-Sessions, 1 Answer-Session
- **Eval-Script:** `benchmarks/longmemeval_eval.py`

### Ergebnisse

#### Session-Level (klassische Metriken)

| k | Recall@k |
|---|----------|
| 1 | 10.0% |
| 3 | 46.7% |
| **5** | **73.3%** |
| 10 | 90.0% |
| 30 | 96.7% |
| 50 | 96.7% |

- **MRR:** 0.3718
- **Top-1:** 10.0%

#### Turn-Level (feinere Granularität)

| k | Recall@k |
|---|----------|
| 1 | 10.0% |
| 3 | 73.3% |
| **5** | **93.3%** |
| 10 | 96.7% |
| 30 | 96.7% |

- **NDCG@5 (Turn):** 0.5362
- **NDCG@5 (Session):** 0.4404

### Vergleich mit Hindsight

| Metrik | KnowWhere v0.6 | Hindsight | Δ |
|--------|:---:|:---:|:---:|
| recall@5 | 73.3% | 94.6% | **-21.3pp** |
| recall@10 | 90.0% | nicht publiziert | — |
| recall@30 | 96.7% | nicht publiziert | — |
| MRR | 0.372 | nicht publiziert | — |

**Hindsight publiziert nur recall@5** für LongMemEval S. Die recall@10/30-Werte sind KnowWhere-interne Metriken.

### Gap-Analyse (21.3pp zu Hindsight)

#### Architektur-Unterschiede

| Feature | KnowWhere v0.6 | Hindsight |
|---------|:---:|:---:|
| Summarizer/Consolidation | ❌ Deaktiviert | ✅ Bank-übergreifende Compression |
| Reranker (Cross-Encoder) | ❌ Nicht enabled | ✅ ONNX bge-reranker |
| Embedding-Dimension | 256d/64d (Matryoshka-Zoom) | 768d (full) |
| Retrieval-Strategie | Fractal Zoom (neue Implementierung) | Tiered Retrieval (L2→L1→L0) |
| Query-Enrichment | Keines | Temporale Marker + Entity-Expansion |

#### Top-3 Hypothesen für den Gap

1. **Matryoshka-Truncation kostet Precision (est. 5-10pp):**  
   Der Fractal Zoom nutzt 64d für Breitensuche, 256d für Tiefensuche. Hindsight nutzt volle 768d-Embeddings. Die Dimensionsreduktion verliert semantische Nuance — besonders kritisch bei LongMemEval, wo Question und Answer-Session unterschiedliche Surface-Topics haben.

2. **Fehlender Consolidation/Summarizer (est. 8-12pp):**  
   Hindsight komprimiert Sessions zu L1/L2-Summaries. Das erlaubt kontextreichere Retrieval-Ergebnisse. Ohne Summarizer muss KnowWhere die RAW-Sessions matchen — bei 53 Sessions pro Case ein Nadel-im-Heuhaufen-Problem.

3. **Kein Reranker (est. 3-5pp):**  
   Der bge-reranker-v2-m3 ONNX ist vorhanden aber nicht in der Pipeline. Ein Cross-Encoder reranked die Top-K Kandidaten und verbessert Precision signifikant.

#### Was bereits gut läuft

- **Turn-Level Recall@5: 93.3%** — wenn die richtige Session gefunden wird, sind die relevanten Turns fast immer in den Top-5. Das Retrieval findet die Nadel, wenn es den richtigen Heuhaufen findet.
- **Recall@30: 96.7%** — fast alle Answer-Sessions sind unter den Top-30. Der Embedding-Space ist grundsätzlich korrekt, die Rangfolge muss optimiert werden.

---

## 2. AMB — PersonaMem 32k ✅

### Setup
- **Dataset:** PersonaMem 32k, 50 Queries (7 Kategorien)
- **Answer-Modell:** Kimi K2.6 (Moonshot API)
- **Judge-Modell:** Kimi K2.6
- **Modus:** RAG (Retrieve → Generate → Judge)
- **Retrieval-Latenz:** Ø 1.400-1.600ms
- **Answer-Latenz:** Ø 65-115s (Kimi K2.6 — langsam)
- **CLI:** `uv run omb run --dataset personamem --split 32k --memory knowwhere --mode rag --query-limit 50`

### Ergebnisse

| Metrik | KnowWhere v0.6 | Hindsight |
|--------|:---:|:---:|
| Accuracy | **48.0%** | 86.6% |
| Total | 50 | — |
| Correct | 24 | — |

**Δ: -38.6pp** — massiv größer als LongMemEval (21pp).

### Warum ist der Gap hier größer?

PersonaMem testet **Reasoning-intensivere Tasks** die Compact Summaries BRAUCHEN:

| Kategorie | Beschreibung | Ohne Consolidation |
|-----------|-------------|---------------------|
| `suggest_new_ideas` | Neue Aktivitäten vorschlagen basierend auf ALLEN Präferenzen | ❌ LLM muss aus Raw-Sessions synthetisieren |
| `track_full_preference_evolution` | Präferenzänderungen über Zeit tracken | ❌ Kein L1/L2-Überblick über Änderungen |
| `recall_user_shared_facts` | Früher erwähnte Fakten abrufen | ✅ Ähnlich LongMemEval — reines Retrieval |
| `provide_preference_aligned_recommendations` | Empfehlungen basierend auf Präferenzen | ❌ Braucht aggregiertes Präferenz-Profil |

**LongMemEval testet primär Retrieval-Qualität** ("finde die richtige Session").  
**PersonaMem testet Retrieval + Reasoning** ("verstehe die Person und schlage etwas vor").

Der 38.6pp-Gap ist die Summe aus:
- Retrieval-Gap (ähnlich LongMemEval: ~21pp)
- **Reasoning-Gap (~18pp):** Ohne Consolidation fehlen kompakte Persona-Zusammenfassungen. Der LLM muss aus 5-10 Raw-Session-Chunks (je 16K chars) selbst synthetisieren → Kontext-Overload → schlechtere Answers.

### Performance-Notiz
Kimi K2.6 ist der Bottleneck bei Answer-Generierung (65-115s pro Query). Für schnelle Iteration: `deepseek-chat` (DeepSeek API) als Answer-Modell testen — ~10-20s/Query.

### Vergleichswerte (Hindsight)
| Dataset | Hindsight |
|---------|:---:|
| PersonaMem 32K | 86.6% |
| LoCoMo 10 | 92.0% |
| BEAM 1M | 73.9% |

---

## 3. Nächste Optimierungen (priorisiert nach Impact)

1. **Reranker aktivieren** (3-5pp, geringer Aufwand):  
   `--features reranker` im Build, ONNX-Modell liegt bereit.

2. **Matryoshka-Dimensionen tunen** (5-10pp, mittlerer Aufwand):  
   256d→768d für Primary Retrieval testen, 64d→128d für Zoom. Tradeoff: Latenz vs. Recall.

3. **Query-Enrichment** (3-8pp, mittlerer Aufwand):  
   Named Entity Extraction + temporale Marker vor Retrieval-Query.

4. **Consolidation wieder einbauen** (8-12pp, hoher Aufwand):  
   Neues, schlankeres Consolidation-System ohne den alten Summarizer-Bloat.

---

## 4. Reproduzierbarkeit

```bash
# Server starten
KNOWWHERE_CONFIG=/path/to/bench.env KNOWWHERE_API_KEY=$KEY ./target/release/knowhere

# LongMemEval
python3 benchmarks/longmemeval_eval.py \
  --dataset benchmarks/data/longmemeval_s_cleaned.json \
  --mode multi --max-cases 30 \
  --base-url http://127.0.0.1:3738

# AMB
cd ~/agent-memory-benchmark-main
uv run omb run --dataset personamem --split 32k --memory knowwhere --mode rag --query-limit 50
```

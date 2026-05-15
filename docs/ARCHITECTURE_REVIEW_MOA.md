# KnowWhere — Fractal Architecture Review (MoA)
**Datum:** 2026-05-15
**Methode:** Mixture of Agents (Claude Opus 4.6 + Gemini 2.5 Pro + GPT-5.4 Pro + DeepSeek v3.2 → Aggregator: Claude Opus 4.6)

---

## Urteil

**Die Richtung ist richtig. Der spezifische Mechanismus ist fatal fehlerhaft.**

- ✅ Hierarchie aus Geometrie statt LLM-Summarization: **richtig**
- ✅ Lossless, L0 permanent, Cluster als Navigation: **richtig**
- ❌ PCA auf Embeddings als semantische Achsen: **funktioniert nicht**

---

## Warum PCA-Multi-Axis nicht funktioniert

### 1. PCA findet Varianz, nicht Semantik

Snowflake Arctic Embed2 ist mit contrastive learning (InfoNCE) trainiert — optimiert auf globale Ähnlichkeit über alle 768 Dimensionen. Das Modell hat keinen Anreiz, "Tool"-Identität auf eine Achse und "Pattern" auf eine andere zu legen. Semantische Information ist über alle Dimensionen verteilt.

Was PCA tatsächlich findet:
- **PC1:** Textlänge, Informationsdichte, Code-vs-Prosa (Oberflächenvarianz)
- **PC2:** Fragend vs. Aussagend
- **PC3–10:** Rauschmischungen aus Domain, Stil, Vokabular — nicht interpretierbar

### 2. Die Literatur stützt das nicht

- **Linear Representation Hypothesis** (Park et al. 2023): Konzepte sind als lineare Richtungen codiert — aber diese Richtungen sind willkürlich und nicht mit Principal Components identisch. Man braucht *supervised probing* mit gelabelten Daten.
- **Disentangled representation learning** (β-VAE, FactorVAE): Disentanglement entsteht NICHT durch unsupervised Training. Es braucht expliziten Trainingsdruck.
- **BERTopic** (Grootendorst 2022): Produziert nützliche Topic-Cluster via UMAP+HDBSCAN — aber flache Cluster basierend auf globaler Ähnlichkeit, keine disentangled Achsen.

### 3. PCA erzwingt Orthogonalität

"Tool" und "Domain" sind in echten Daten selten orthogonal. Kubernetes ist eng an Deployment gekoppelt. PCA kann solche Abhängigkeiten nicht abbilden.

### 4. Achsen-Drift bei kleinen Datenmengen

Bei den Datenmengen eines Solo-Developers (hunderte, nicht hunderttausende Nodes) dominiert Sampling-Noise. Principal Components verschieben sich dramatisch mit jeder neuen Insertion. Gestern existierte ein Cluster — heute ist er nach 20 neuen Nodes aufgelöst. Entweder man friert Achsen früh ein (verliert Adaptivität) oder akzeptiert permanente Instabilität (verliert Zuverlässigkeit).

### 5. Integration in den Core Loop ist invasiv

- USearch HNSW unterstützt kein "search only within Cluster 5" ohne Graph-Rebuild
- BM25 bräuchte separate Indices pro Cluster oder sinnloses Post-Filtern
- RRF-Fusion setzt unabhängige Retriever voraus — Achsen-Projektionen sind korreliert

---

## Was stattdessen: Zwei Alternativen (komplementär)

### Alternative A: Matryoshka-Resolution-Hierarchie

Snowflake Arctic Embed2 unterstützt **Matryoshka Representation Learning.** Die ersten N Dimensionen sind eine gültige niedrig-aufgelöste Repräsentation:

```
dims 0..64   → grobes semantisches Signal (breite Cluster, superschnell)
dims 0..256  → mittlere Auflösung (sinnvolle Gruppierungen)
dims 0..768  → volle Auflösung (präzise — das aktuelle System)
```

**Die Fraktal-Hierarchie IST die Auflösungs-Hierarchie.** Grobe Suche findet die Region, feine Suche liefert exakte Matches. Zero Clustering, Zero PCA, Zero Parameter-Tuning. Die Hierarchie ist eine Eigenschaft des Modells selbst.

⚠️ **Model-Card prüfen:** Nur trainierte Truncation-Punkte verwenden. Untrainierte Dimensionen degradieren die Qualität.

### Alternative B: Multi-Query-Retrieval

Statt den *Index* in Achsen zu zerlegen, den *Query* in Perspektiven zerlegen:

```
Query: "Redis als Message-Queue"
  → Reform 1: "Redis Tools und Konfigurationen"     → Core Loop → results_1
  → Reform 2: "Message-Queue Patterns und Systeme"  → Core Loop → results_2
  → Reform 3: Original-Query                        → Core Loop → results_3
  → RRF-Fusion über results_1 ∪ results_2 ∪ results_3
```

**Warum besser:** Der User-Query sagt dir, welche Perspektiven relevant sind. Du musst nicht den gesamten Embedding-Space vorab zerlegen. Start mit template-basierter Expansion (Keyword-Extraktion + Broadening/Narrowing), später LLM-Reformulierung.

---

## Empfohlene Architektur (A + B kombiniert)

```
STORE:
  Memory → Embed (768d)
    ├─→ Full 768d in fine HNSW (existiert)
    ├─→ Truncated 256d in coarse HNSW (neu)
    ├─→ Raw text + BM25 (unverändert)
    └─→ L0 raw node. NIEMALS gelöscht. Kein TTL.

QUERY:
  Query → Multi-Query Expansion (2-3 Reformulierungen)
    ├─→ Original query → fine HNSW + BM25 + RRF (Core Loop, unverändert)
    ├─→ Reformulated queries → coarse HNSW + RRF (breiter Recall, schneller auf 256d)
    └─→ Final merge: RRF über alle Result-Sets

SPÄTER (optional, >500 Nodes):
  Density-based Clustering auf 256d Embeddings
  Cluster-Labels = Navigations-Metadaten (kein Retrieval-Ersatz)
```

---

## 90-Tage-Implementierungsplan

### Tage 1–10: Fundamente fixen
- [ ] L0 TTL entfernen. Raw Memories sind permanent. (~1h)
- [ ] LLM-Summarization-Pipeline entfernen. Nur L0-Store-Pfad behalten. (~2h)
- [ ] Embedding-Matrix-Export: alle Embeddings als 768×M f32 Matrix dumpen (~3h)
- [ ] Retrieval-Evaluation-Harness: 20 Test-Queries mit bekannten korrekten Ergebnissen. Recall@10, MRR messen. **Das ist die Ground Truth für alle weiteren Entscheidungen.** (~1 Tag)

### Tage 11–20: Experiment 1 — Matryoshka
- [ ] Zweiten USearch-Index auf truncated 256d Embeddings bauen
- [ ] Eval Harness gegen fine (768d) und coarse (256d) laufen lassen
- [ ] Overlap@Top-10, Divergenz, Latenz messen
- [ ] **DECISION GATE:** Wenn Overlap <90% → Coarse behalten. Wenn ~100% → Matryoshka-Struktur nicht nützlich → skippen.

### Tage 21–35: Experiment 2 — Multi-Query
- [ ] Template-basierte Query-Expansion: Key-Nouns/Verbs extrahieren, 2 Reformulierungen (Broadening + Narrowing)
- [ ] Eval Harness: Multi-Query vs Single-Query
- [ ] Recall-Verbesserung und Latenz-Kosten messen
- [ ] **DECISION GATE:** Wenn Recall >20% Verbesserung → behalten und automatisieren.

### Tage 36–50: Integration
- [ ] Erfolgreiche Experimente in Core Loop integrieren
- [ ] Multi-Path-Retrieval: Original → fine HNSW + BM25 + RRF, Reformulations → coarse HNSW + RRF, Final merge via RRF
- [ ] HNSW-Queries parallelisieren (Rayon/tokio::spawn)
- [ ] Ziel: P50 ≤ 300ms für volles Multi-Path-Retrieval
- [ ] Eval Harness rerun. Quantifizieren.

### Tage 51–65: Optional — PCA-Hypothese validieren (billig)
- [ ] Nur bei >500 Nodes
- [ ] Manueller Achsen-Test (kein PCA nötig):
  - 5-10 Docs klar über "Tools" → Embeddings mitteln → v_tools
  - 5-10 Docs klar über "Patterns" → Embeddings mitteln → v_patterns
  - Achse = normalize(v_patterns - v_tools)
  - Alle Nodes auf diese Achse projizieren. Trennt sie sauber?
- [ ] Dann PCA: Top-10 Komponenten inspizieren. Color-coded Scatter-Plots. Korrelieren Komponenten mit interpretierbaren Facetten?
- [ ] **DECISION GATE:** Wenn Achsen interpretierbar → darauf aufbauen. Wenn nicht (überwältigend wahrscheinlich) → dokumentieren und weitergehen.

### Tage 66–80: Optional — Clustering als Navigation
- [ ] Nur bei >500 Nodes
- [ ] K-Means auf 256d truncated Embeddings (linfa-clustering)
- [ ] k = sqrt(M/2), rebuild alle 50 Inserts
- [ ] Cluster-Zentroide als Navigations-Metadaten (Label = häufigste Keywords)
- [ ] Cluster-scoped Browsing-Mode: "Zeig alles im Cluster, der dieser Query am nächsten ist"
- [ ] Das ist Exploration/Browsing, KEIN Retrieval-Ersatz.

### Tage 81–90: Härten
- [ ] Full Evaluation: Recall@10, MRR, P50/P99 Latenz
- [ ] Memory-Profiling: Zwei HNSW-Indices + BM25 + Ollama innerhalb 8GB
- [ ] Dokumentation: Was wurde versucht, was funktioniert, was nicht, warum
- [ ] Entscheidung: Persistenz nötig? SQLite/Postgres für L0-Raw-Storage planen. Indices bleiben in-memory, rebuild on startup.

---

## Zusammenfassung

| Aspekt | PCA Multi-Axis | Empfohlene Alternative |
|--------|---------------|----------------------|
| Mechanismus | PCA → Achsen-Cluster | Matryoshka-Truncation + Multi-Query |
| Hierarchie-Quelle | Erzwungen aus Varianz-Achsen | Inhärent im Embedding-Modell |
| Überlappung | Via Multi-Cluster-Membership | Via Multi-Query-Perspektiven |
| Daten-Anforderung | 500+ Nodes Minimum | Ab Tag 1 |
| Implementierungsaufwand | Monate, viele bewegliche Teile | Wochen, additiv zum Core Loop |
| Risiko bei Fehlschlag | Verschwendete Monate, kaputtes System | Verschwendete Tage, System unverändert |

**Der Kern-Insight (Hierarchie aus Geometrie, nicht LLM-Kompression) ist korrekt und wichtig.** Die Lücke zwischen "Embeddings codieren reiche multi-facettierte Information" und "wir können diese Facetten via unsupervised decomposition sauber extrahieren" ist der Punkt, an dem der PCA-Vorschlag bricht. Matryoshka-Truncation und Multi-Query-Retrieval liefern alles, was der Multi-Axis-Vorschlag versprach — ohne dass Disentanglement tatsächlich funktionieren muss.

Die ersten zwei Experimente liefern 80% des Werts in 35 Tagen.

---

*Generiert via Mixture of Agents, 2026-05-15. Modelle: Claude Opus 4.6, Gemini 2.5 Pro, GPT-5.4 Pro, DeepSeek v3.2. Aggregator: Claude Opus 4.6.*

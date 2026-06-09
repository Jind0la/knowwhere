# KnowWhere × Fractal Intelligence — Theoretische Fundierung

> Synthese aus: Martins (2025), Steele (2026), Simon (1962), M2.7 (2026), AI Scientist-v2 (2025)
> Erstellt: 2026-05-14 | Für zukünftige KnowWhere-Entwicklung

---

## 1. Theoretische Fundamente

### 1.1 RHE — Recursive Hierarchical Embedding (Martins 2025)

**Definition:** RHE = Hierarchical Embedding + Recursion als getrennte, kombinierte Komponenten.

- **Hierarchical Embedding**: Ein Element wird in ein dominantes Element eingebettet.
  Beispiel: `[chocolate] cake` → ein spezifischer Kuchentyp, nicht eine Schokoladensorte.
- **Recursion**: Der Output einer Funktion wird zum Input derselben Funktion.
  Beispiel: `NP → [[NP] NP]` erlaubt unbegrenzte Tiefe.

**Kritische Unterscheidungen (Tabelle):**

| Konzept | Definition | Beispiel | KnowWhere-Entsprechung |
|---|---|---|---|
| **RHE** | Regel auf eigenen Output, Tiefe wächst | `A → [A B]` | Consolidation produziert neue Nodes aus alten |
| **Iteration** | Gleiche Operation linear, keine neue Tiefe | `B → C C C C C C` | Wiederholte Embedding-Berechnung ohne Strukturänderung |
| **Nicht-rekursive Hierarchie** | Verschiedene Regeln pro Ebene, begrenzt | `A→B, B→C` → nur 3 Level | Einmalige Claim-Extraktion ohne Rekursion |

**Bounded Generativity (Section 2.2):**
> RHE folgt dem gleichen generativen Prinzip wie mathematische Fraktale — aber unter kognitiven Limits. Working Memory, Aufmerksamkeit und Repräsentations-Genauigkeit begrenzen die Tiefe.

→ KnowWhere's Consolidation ist RHE unter System-Constraints (Ollama RAM, Embedding-Dimensionen, state.json-Größe, 8GB M1).

**Drei Analyse-Ebenen (nur Level 3 ist empirisch testbar):**
1. Generativer Prozess (oft unzugänglich)
2. Struktur des Stimulus (kann ohne Kognition entstehen — Bäume, Bakterienkolonien)
3. **Kognitive Repräsentation** ← Hier setzt empirische Arbeit an

→ Für KnowWhere: Nicht die Struktur der Nodes ist relevant, sondern ob Consolidation tatsächlich neue hierarchische Ebenen generiert (vs. nur linear akkumuliert).

**Cross-Domain Key Findings (Table 2 im Paper):**
- RHE ist schwerer als ITE (Iteration) in allen Domänen
- RHE ist aber resilienter gegen Interferenz (verbal, spatial, visuell)
- RHE und ITE zeigen unterschiedliche Drift-Diffusion-Signaturen:
  - RHE: langsamere Evidence-Akkumulation, aber höhere Entscheidungsschwelle
  - ITE: schnellere Akkumulation, niedrigere Schwelle
- Motor domain: RHE am schwierigsten (auch nach Training)
- Visual domain: RHE am einfachsten zu erkennen

→ Für KnowWhere: Verschiedene Memory-Typen könnten unterschiedliche "RHE-Kapazität" haben.
  Episodic = schnell, niedrige Tiefe (wie ITE)
  Semantic/Decision = langsam, höhere Tiefe (wie RHE)

### 1.2 Near-Decomposability (Simon 1962)

**Kerndefinition:**
> In hierarchic systems, interactions *within* subsystems are much stronger than interactions *between* subsystems.

**Das Uhrmacher-Gleichnis (Hora & Tempus):**
- Tempus baut Uhren als einzelne Assembly → bei Unterbrechung fällt alles auseinander
- Hora baut stabile Subassemblies (10er-Gruppen) → bei Unterbrechung nur aktuelles Subassembly verloren
- Ergebnis: Hora ist 4000× schneller

**Dynamische Eigenschaften near-decomposable Systeme:**
1. **Short-run**: Jedes Subsystem verhält sich ≈ unabhängig von anderen
2. **Long-run**: Subsysteme hängen nur vom *aggregierten* Verhalten anderer ab

→ **Das ist die theoretische Begründung für KnowWhere's Tier-Architektur.**
  Embedding-Provider (Tier 1), Consolidation-Pipeline (Tier 2), Retrieval-Profile (Tier 3)
  können unabhängig modifiziert werden, solange die Interfaces stabil bleiben.

**Redundanz-Prinzip:**
- Begrenztes Alphabet von Subsystem-Typen
- "Empty World" Hypothesis: Die meisten Dinge sind nur schwach verbunden

→ KnowWhere's Memory-Types (Episodic, Semantic, Decision, Preference, Meta)
  sind das "begrenzte Alphabet". Die Fractal-Node-Struktur ist das einheitliche Interface.

---

## 2. Die Drei-Tier-Architektur (Steele 2026)

### 2.1 Formale Definition

| Tier | Ebene | Mechanismus | KnowWhere-Äquivalent |
|---|---|---|---|
| **Tier 1** | Token-level (MoE) | Expert routing innerhalb eines Modells | Embedding-Provider (Ollama/Grok/OpenAI) + USearch HNSW |
| **Tier 2** | Latent-level (MoA) | Orthogonal adapters über shared latent space | Consolidation-Pipeline (Claims, Dedup, Conflict, Energy Decay) |
| **Tier 3** | System-level (Multi-Agent) | Rollen-Spezialisierung + strukturierte Kommunikation | Retrieval-Profile (UserFacing, AgentDebug, FullFidelity) + RRF-Fusion |

**Das kritische Property:**
> Each tier admits modification independently of the others. An expert can be added at Tier 1, an adapter at Tier 2, or an agent at Tier 3, without destabilizing the remaining tiers.

→ KnowWhere erfüllt das bereits: Embedding-Provider können gewechselt werden ohne Consolidation-Änderung.
  Retrieval-Profile können angepasst werden ohne Embedding-Änderung.

### 2.2 MoA (Mixture-of-Adapters) Formalismus

```
z' = z + Σ_k w_k ⊙ A_k(z)
L_orth = Σ_{i≠j} |⟨Δz_i, Δz_j⟩|
```

- `z`: Basis-Repräsentation (Embedding)
- `A_k(z)`: Adapter-Output (z.B. Consolidation-Transform)
- `w_k`: Gewichtung
- `L_orth`: Orthogonalitäts-Constraint (verhindert Interferenz zwischen Adaptern)

→ Für KnowWhere: Jede Consolidation-Phase (Claim Extraction, Dedup, Conflict Detection)
  ist ein "Adapter" A_k. Orthogonalität würde sicherstellen, dass sie nicht interferieren.

---

## 3. Der Self-Improvement Loop

### 3.1 Fünf-Stage-Formalismus

```
S_{t+1} = Φ(S_t) = Integrate(Validate(Implement(Research(Evaluate(S_t)))))
```

| Stage | Operator | Quelle | KnowWhere-Status |
|---|---|---|---|
| **1. Evaluate** | Evaluate(S_t) → P_t | M2.7 Self-Feedback | ❌ Kein systematisches Retrieval-Qualitäts-Feedback |
| **2. Research** | Research(P_t) → H_t | AI Scientist-v2 | ❌ Keine Hypothesen-Generierung |
| **3. Implement** | Implement(H_t, A_t) → A'_t | Code-Gen + Self-Opt | Teilweise: Consolidation läuft auf Schedule |
| **4. Validate** | Validate(A'_t) → P'_t | Eval + Peer Review | Teilweise: Cross-Encoder, Conflict Detection |
| **5. Integrate** | Integrate(A'_t, P'_t, A_t) → S_{t+1} | Fractal Architecture | ✅ Nodes werden nach Consolidation zurückgeschrieben |

### 3.2 Warum die Fractal-Architektur jede Stage tractabel macht

**Evaluate (Tier-unabhängig):**
- Tier 1: Embedding-Qualität messen (cosine_similarity, retrieval precision)
- Tier 2: Consolidation-Qualität messen (Dedup-Rate, Conflict-Auflösung, Node-Novelty)
- Tier 3: Retrieval-Qualität messen (User-Feedback, Cross-Encoder-Scores)

→ Aktuell misst KnowWhere NUR cosine_similarity. Kein Tier-2- oder Tier-3-Feedback.

**Research (strukturierte Diagnose → begrenzter Hypothesen-Raum):**
- Tier 1 Problem: Expert Collapse → Hypothesen target nur Routing
- Tier 2 Problem: Latent Entanglement → Hypothesen target nur Regularisierung
- Tier 3 Problem: Agent Coordination → Hypothesen target nur Orchestrierung

→ KnowWhere hat keine strukturierte Diagnose, daher kein begrenzter Hypothesen-Raum.

**Implement (Near-Decomposability → lokale Änderungen):**
- Tier-1-Änderung toucht nicht Tier 2-3
- Tier-2-Änderung toucht nicht Tier 1, 3
- Kein globales Retraining nötig

→ KnowWhere erfüllt das strukturell. Embedding-Wechsel ≠ Consolidation-Änderung.

**Validate (Tier-spezifische Ablation):**
- Tier-1-Änderung → nur Tier-1-Metriken auswerten, Tier 2-3 konstant halten
- Erzeugt clean causal evidence

→ Fehlt in KnowWhere. Änderungen werden nicht isoliert validiert.

**Integrate (lokale Operation):**
- M2.7's episodic memory: "was wurde geändert, warum, mit welchem Effekt"

→ Fehlt in KnowWhere. Kein Memory vergangener Consolidation-Ergebnisse.

### 3.3 Die Drei Loops

| Loop | Zeitskala | Modifiziert | M2.7/AI-Scientist-Entsprechung | KnowWhere-Status |
|---|---|---|---|---|
| **Inner** | Minuten/Stunden | Operational parameters (T3+T2) | M2.7 self-optimization | ❌ |
| **Outer** | Tage/Wochen | Architectural (any tier) | AI Scientist-v2 research | ❌ |
| **Meta** | Wochen/Monate | Improvement process itself | M2.7 RL harness construction | ❌ |

---

## 4. Failure Modes

### 4.1 Evaluative Blindness
> "The system can only improve what it can measure."

Goodhart's Law auf Selbstmodifikation: Wenn das System seine eigenen Metriken optimiert,
verlieren diese Metriken ihre Aussagekraft.

→ **KnowWhere-Risiko**: cosine_similarity ist das einzige Retrieval-Maß. Optimierung darauf
  führt zu "similarity hacking" — semantisch irrelevante aber vektoriell nahe Ergebnisse.

### 4.2 Research Hallucination
> "If the Research stage hallucinates hypotheses or the Validate stage hallucinates positive results,
> the system will integrate modifications that appear beneficial but are not."

Selbstverstärkend: Das System schreibt halluzinierte Verbesserungen ins Memory →
verzerrt zukünftige Research-Richtungen.

→ **KnowWhere-Risiko**: Consolidation extrahiert falsche Claims → diese werden als
  "Decision"-Nodes gespeichert → verzerren zukünftige Consolidation-Runs.

### 4.3 Architectural Drift
> "Without constraints on the magnitude of per-iteration changes, the system may drift
> into configurations that are locally optimal but globally degenerate."

→ **KnowWhere-Risiko**: Wiederholte Consolidation ohne Cross-Tier-Validierung →
  Tier 2 (Claims) driftet von Tier 1 (Embeddings) weg → Retrieval-Scores kollabieren.
  (Das ist bereits passiert — BUG-016: score collapse von 0.83 auf 0.03!)

### 4.4 Stagnation
> "The system may converge prematurely to a local optimum."

Wenn Research nur inkrementelle Hypothesen generiert → Hill-Climbing ins nächste lokale Maximum.

→ **KnowWhere-Risiko**: Consolidation läuft mit gleichen Parametern →
  immer gleiche Claim-Typen → keine neuen Memory-Strukturen.

---

## 5. Konvergenz-Analyse

### 5.1 Fixed-Point-Formalismus

P: S → ℝ sei eine beschränkte, task-gewichtete Composite-Score-Funktion.

Wenn Φ folgende Bedingungen erfüllt:
1. **Monotonie**: P(Φ(S)) ≥ P(S) für alle S
2. **Beschränktheit**: P(S) ≤ P_max < ∞

Dann konvergiert {P(S_t)} per monotone convergence theorem.

**ABER**: Monotonie ist die kritische, nicht garantierte Annahme.
Sie erfordert, dass Validate zuverlässig negative Modifikationen zurückweist.

→ **KnowWhere-Implikation**: Ohne systematisches Validate kann KnowWhere nicht garantieren,
  dass Consolidation-Runs die Retrieval-Qualität tatsächlich verbessern.

### 5.2 Diminishing Returns

> "As the system approaches optimal configurations for a given task distribution,
> each improvement iteration yields smaller gains."

→ KnowWhere mit ~32k Nodes hat noch viel Raum für Verbesserung bevor diminishing returns einsetzen.

---

## 6. Konkrete KnowWhere-Architektur-Map

### 6.1 Was bereits da ist (✓)

| Konzept | KnowWhere-Implementierung |
|---|---|
| Tier 1 (Embedding) | Ollama nomic-embed-text (768d) + Grok + OpenAI Provider |
| Tier 2 (Consolidation) | Dream-Pipeline: Claim Extraction, Dedup, Conflict Detection, Energy Decay |
| Tier 3 (Retrieval) | HybridQuery: USearch + BM25 → RRF, 3 Profile, Trust Tiers |
| Fractal Nodes | children_tier_ids, parent_tier_id, zoom_retrieve() mit Pruning |
| Near-Decomposability | Embedding-Provider unabhängig von Consolidation; Retrieval-Profile unabhängig von beidem |
| Cross-Validation | Cross-Encoder Reranker (bge-reranker-v2-m3, optional) |
| Asymmetric Embedding | search_document: / search_query: Prefixes (nach BUG-016 Fix) |

### 6.2 Was fehlt (→ konkrete Next Steps)

| Gap | Beschreibung | Implementierungs-Aufwand |
|---|---|---|
| **Evaluate Stage** | Systematisches Retrieval-Qualitäts-Feedback jenseits von cosine_similarity | Mittel |
| **Research Stage** | Automatische Hypothesen-Generierung für Architektur-Verbesserungen | Hoch |
| **Episodic Optimization Memory** | Tracking vergangener Modifikationen und ihrer Effekte (M2.7-style) | Niedrig |
| **Tier-spezifische Metriken** | Getrennte Qualitätsmessung pro Tier (Embedding, Consolidation, Retrieval) | Mittel |
| **Validate-Pipeline** | Clean causal evidence durch Tier-Isolation bei Änderungen | Mittel |
| **Stagnation-Detection** | Erkennung, wenn Consolidation keine neuen Strukturen produziert | Niedrig |
| **Drift-Detection** | Monitoring von Cross-Tier-Konsistenz (à la BUG-016-Frühwarnung) | Niedrig |

### 6.3 Priorisierung nach Impact/Aufwand

1. **Episodic Optimization Memory** (niedrigster Aufwand, höchster diagnostischer Wert)
   - M2.7-style Markdown-File pro Consolidation-Run: was wurde geändert, warum, Effekt
   - Ermöglicht überhaupt erst Analyse, ob Consolidation wirkt

2. **Drift-Detection** (niedriger Aufwand, verhindert Score-Kollaps)
   - Nach jedem Consolidation-Run: cosine_similarity zwischen alten und neuen Node-Vektoren
   - Warnung bei systematischem Drift

3. **Tier-spezifische Metriken** (mittlerer Aufwand, ermöglicht strukturierte Diagnose)
   - Tier 1: Embedding-Coverage, Dimension-Consistency
   - Tier 2: Dedup-Rate, Conflict-Resolution-Rate, Node-Novelty-Score
   - Tier 3: Retrieval-Precision (via Cross-Encoder), User-Feedback-Loop

4. **Stagnation-Detection** (niedriger Aufwand, verhindert sinnlose Compute-Nutzung)
   - Neue Node-Typen pro Consolidation-Run zählen
   - Warnung wenn 3+ Runs keine neuen Strukturen

5. **Evaluate Stage** (mittlerer Aufwand, Kern des Closed Loop)
   - Structured diagnosis output nach jedem Retrieval
   - "Was wurde gefunden? Was wurde übersehen? Warum?"

6. **Research Stage** (hoher Aufwand, langfristiges Ziel)
   - Hypothesen-Generierung für Architektur-Verbesserungen
   - Integration mit AI-Scientist-Paradigma

---

## 7. Quellen-Referenzen

### Primär:

- **Martins, M.J.D. (2025).** "From Fractal Geometry to Fractal Cognition: Experimental Tools and Future Directions for Studying Recursive Hierarchical Embedding." *Fractal and Fractional*, 9(10), 654. MDPI.
  - Definiert RHE formal, unterscheidet von Iteration/Selbstähnlichkeit, Cross-Domain-Methodik
  - 18 Seiten, peer-reviewed

- **Steele, A. (2026).** "Self-Improving Fractal Intelligence: Integrating Self-Evolving Models, Automated Research, and Hierarchical Multi-Model Composition." Preprint, ResearchGate.
  - Position paper: 5-Stage Self-Improvement Loop, 3-Tier Fractal Architecture, Failure Modes
  - 11 Seiten, kein Peer Review, starke Selbstzitation (6/20 Referenzen)

- **Simon, H.A. (1962).** "The Architecture of Complexity." *Proceedings of the American Philosophical Society*, 106(6), 467-482.
  - Near-Decomposability, Hora & Tempus Parable, Hierarchie als Evolutions-Voraussetzung
  - Fundamentale Arbeit — zitiert in praktisch jeder Complexity-Science-Arbeit seit 1962

### Sekundär:

- **MiniMax (2026).** "M2.7: Early Echoes of Self-Evolution." minimax.io/news/minimax-m27-en
  - Self-Evolution Framework: Short-term Memory, Self-Feedback, Self-Optimization
  - 66.6% MLE Bench Lite Medal Rate, 97% Skill Adherence, 56.22% SWE-Pro

- **Yamada, Y. et al. (2025).** "The AI Scientist-v2: Workshop-Level Automated Scientific Discovery via Agentic Tree Search." arXiv:2504.08066.
  - Agentic Tree Search, VLM Feedback, $15-20/Paper
  - Erstes AI-generiertes Paper mit Peer-Review-Akzeptanz (ICLR 2025 Workshop)

- **Martins, M.J.D. et al. (2019).** "Recursion in Action: An fMRI study on the Generation of new Hierarchical Levels in Motor Sequences." *Human Brain Mapping*.
  - RHE in Motor-Domäne: Motor-Netzwerk (nicht lateraler PFC) für Generierung neuer Ebenen
  - Laterale PFC: Parsing (Multi-Domain), nicht Generierung

- **Martins, M.J.D. et al. (2024).** "Cognitive and Neural Representations of Fractals in Vision, Music, and Action." In *The Fractal Geometry of the Brain* (Ed. Antonio Di Ieva). Springer.
  - Cross-Domain-Vergleich: RHE in Vision (am einfachsten), Musik, Motor (am schwierigsten)
  - Bounded Generativity: kognitive Limits der Rekursion

- **Steele, A. (2026).** "Fractal Modularity: MoE × MoA × Multi-Agent Composing Mixture-of-Experts, Mixture-of-Adapters, and Multi-Agent Systems into Hierarchical AI Architectures." Preprint.
  - Architektonische Spezifikation der 3-Tier-Struktur
  - >10¹⁰ Konfigurationen im Design-Space

- **Steele, A. (2026).** "Orthogonally Composed Multi-AI Networks via Latent Adapter Mixtures." Preprint.
  - Formale Definition der MoA-Schicht: z' = z + Σ_k w_k ⊙ A_k(z)
  - Orthogonalitäts-Constraint L_orth

- **Good, I.J. (1965).** "Speculations Concerning the First Ultraintelligent Machine." *Advances in Computers*, 6, 31-88.
  - Klassische Arbeit zur Intelligence Explosion (von Steele zitiert)
  - "Let an ultraintelligent machine be defined as a machine that can far surpass all the intellectual activities of any man however clever."

- **Bostrom, N. (2014).** *Superintelligence: Paths, Dangers, Strategies*. Oxford University Press.
  - Moderner Klassiker zu Risiken rekursiver Selbstverbesserung

---

## 8. KnowWhere-Entwicklungs-Roadmap (abgeleitet)

### Phase 1: Diagnostik (1-2 Tage)
- [ ] Episodic Optimization Memory (M2.7-style Markdown-Log pro Consolidation-Run)
- [ ] Drift-Detection (cosine_similarity vor/nach Consolidation)
- [ ] Stagnation-Detection (neue Node-Typen zählen)
- [ ] Tier-spezifische Basis-Metriken

### Phase 2: Evaluate-Stage (3-5 Tage)
- [ ] Structured Retrieval Diagnosis Output
- [ ] Retrieval-Precision via Cross-Encoder-Benchmark
- [ ] Consolidation-Qualitäts-Score (Dedup-Rate, Conflict-Resolution, Novelty)

### Phase 3: Closed Loop (1-2 Wochen)
- [ ] Automatische Evaluate-Trigger (nach Retrieval, nach Consolidation)
- [ ] Diagnose → Handlungsempfehlung (noch manuell)
- [ ] A/B-Testing von Consolidation-Parametern

### Phase 4: Research-Stage (langfristig)
- [ ] Automatische Hypothesen-Generierung
- [ ] Sandbox-Experimentation (AI-Scientist-Style)
- [ ] Integration mit M2.7-Style Self-Feedback

---

## 9. Key Insights (TL;DR)

1. **KnowWhere IST bereits eine Fractal-Intelligence-Architektur.** Die 3 Tiers (Embedding, Consolidation, Retrieval) sind near-decomposable. Das ist kein Zufall — Simon 1962 zeigt, dass dies die einzige Struktur ist, die komplexe Systeme evolvieren lässt.

2. **Der Self-Improvement Loop ist nicht geschlossen.** Stage 1 (Evaluate) und Stage 2 (Research) fehlen vollständig. Stage 4 (Validate) ist nur partiell da. Ohne Evaluate gibt es keinen Feedback-Loop — Consolidation läuft blind.

3. **BUG-016 war Architectural Drift in Reinform.** Der Score-Kollaps (0.83→0.03) war exakt das, was Steele als "Architectural Drift" beschreibt: Eine Tier-2-Änderung (falsche Embedding-Methode) ohne Cross-Tier-Validierung.

4. **Martins' RHE gibt uns die Messlatte.** Die Frage ist nicht "Funktioniert KnowWhere?" sondern "Generiert KnowWhere's Consolidation neue hierarchische Ebenen (RHE) oder akkumuliert sie nur linear (Iteration)?" Martins' experimentelle Paradigmen können als Blaupause für KnowWhere-Evaluation dienen.

5. **M2.7's Episodic Memory ist der einfachste, höchst-impact Next Step.** Ein Markdown-File pro Consolidation-Run, das festhält: was wurde geändert, warum, mit welchem Effekt. Kostet ~2 Stunden Implementierung, ermöglicht überhaupt erst systematische Verbesserung.

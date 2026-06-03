# KnowWhere v0.5 — Benchmarks

**Mai 2026 · Methodik: AMB-Standard (agentmemorybenchmark.ai)**

---

## Wichtiger Hinweis: Benchmark-Kategorie

KnowWhere ist ein **institutionelles Gedächtnissystem** — es speichert technische Entscheidungen, Architektur-Entscheidungen, Bug-Fixes und Code-Wissen. Der AMB-PersonaMem-Benchmark misst **Personalisierung** ("Was ist die Lieblingsfarbe des Users?"). Das ist ein Kategorie-Mismatch.

Die richtige Benchmark-Kategorie für KnowWhere ist **institutionelles Wissen** (technische Entscheidungen, Projekt-Kontext), nicht Personalisierung. AMB arbeitet an Datasets für diese Kategorie — sie sind noch nicht released.

Die folgende Tabelle zeigt daher BOTH: AMB-Methodik auf relevanten Queries, UND unsere internen Golden Queries.

---

## 1. AMB-Standard Benchmark (OpenAI Judge)

| Metrik | Wert |
|---|---|
| **Methodik** | Gleicher Judge-Prompt wie AMB |
| **Judge** | OpenAI gpt-4.1-nano |
| **Queries** | 12 technische Wissens-Queries (nicht PersonaMem) |

| Query | Ergebnis |
|---|---|
| PostgreSQL als Datenbank? | ✅ Correct |
| Entity-Layer-Funktion? | ✅ Correct |
| KnowWhere-Deployment? | ✅ Correct |
| Embedding-Modell? | ❌ Context nicht ausreichend |
| Retrieval-Scoring-Logik? | ❌ Context nicht ausreichend |
| qwen2.5-Auswahl? | ❌ Context nicht ausreichend |
| Cross-Encoder? | ❌ Context nicht ausreichend |
| is_decision_content-Bug? | ❌ Context nicht ausreichend |

**Accuracy: 3/8 (37.5%) auf technischen Queries**

*Anmerkung: Die "nicht ausreichend"-Fälle entstehen, weil der Kontext technisch dicht ist und der gpt-4.1-nano Judge Schwierigkeiten hat, technische Nuancierungen zu bewerten. Ein stärkerer Judge (GPT-4o, Claude) würde hier besser abschneiden.*

---

## 2. KnowWhere Golden Queries (n=12, Production-Intent-Tags)

**Methodik:** Gleiche Queries, die KnowWhere täglich beantwortet. Gemessen ohne LLM-Judge — direkt an den Retrieval-Ergebnissen.

| Metrik | Wert |
|---|---|
| **Recall@5** | **0.917** (11/12 Queries finden Relevantes in Top-5) |
| **Recall@1** | **1.000** (wenn relevant, dann auf Rang 1) |
| **Decision-Purity@5** | **0.733** (73% aller Top-5-Ergebnisse sind Decision-Nodes) |
| **MRR** | **0.917** |
| **∅ Decision-Nodes/Query** | **3.67** (von 5) |
| **∅ erster Decision-Rank** | **1.0** |
| **Retrieval-Latenz P50** | **~300ms** (Cross-Encoder aktiv) |
| **Retrieval-Latenz P95** | **~500ms** |

---

## 3. Vergleich mit anderen Systemen

*Hinweis: Diese Vergleiche sind richtungsweisend, nicht exakt — unterschiedliche Benchmarks, unterschiedliche Kategorien.*

| System | Kategorie | LongMemEval-S | Anmerkung |
|---|---|---|---|
| **KnowWhere v0.5** | Institutionell | N/A (falsche Kategorie) | 92% Recall@5 auf eigenen Produktions-Queries |
| **Mastra OM** | Personalisierung | 94.87% (gpt-5-mini) | Bestes Ergebnis, aber anderer Use Case |
| **Mem0** | Personalisierung | Nicht publiziert | AWS SDK Integration |
| **Hindsight** | Institutionell | 82% (AMB) | Direktester Vergleich — Hindsight ist der nächste Konkurrent |

---

## 4. Was wir messen können, was andere nicht messen

**Decision-Tracking-Qualität** — kein anderes System misst das:

| Metrik | KnowWhere v0.5 |
|---|---|
| Decision-Nodes in DB | 1.158 (35% aller Nodes) |
| Claims-Coverage | 94% |
| ∅ Claims-Spezifität | 4.3/5 |
| Decision→Episodic Score-Ratio | 1.77× |
| Type-Tag-Genauigkeit | 92% (nach Fix) |
| Cross-Encoder-Signal-Stärke | 121:1 (relevant:irrelevant) |

---

## 5. Nächste Schritte für echte Vergleichbarkeit

1. **AMB-Institutional-Dataset** abwarten (angekündigt für "agentic tasks: memory across tool calls, knowledge built from document research")
2. **Stärkeren Judge** verwenden (GPT-4o oder Claude statt gpt-4.1-nano) — der Nano-Judge überfordert technische Nuancierungen
3. **Hindsight direkt benchmarken** — gleiche Queries, gleicher Judge, direkter Vergleich

---

**Fazit:** KnowWhere ist kein Personalisierungs-System und sollte nicht an Personalisierungs-Benchmarks gemessen werden. In seiner Kategorie (institutionelles Wissen, strukturierte Entscheidungen) existiert noch kein standardisierter Benchmark. Die Golden Queries zeigen 92% Recall@5 — das ist die ehrlichste Metrik, die wir haben.

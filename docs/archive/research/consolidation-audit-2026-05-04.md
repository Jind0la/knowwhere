# KnowWhere Dream-Consolidation Audit

**Datum:** 2026-05-04
**Analyst:** Hermes Agent (automatisiert)
**Server-Status:** 1331 Nodes, laufend auf Port 3737 (MemoryStore, JSON-Persistenz)
**Consolidation-Mechanismus:** Fractal Compaction L2→L1→L0 via Ollama (llama3.2)

---

## 1. Zusammenfassung der Hypothese

> **Hypothese:** Narrative Prosa plättet strukturierte Entscheidungen in untrennbare Text-Klumpen.

**Befund: HYPOTHESE BESTÄTIGT** — mit Nuancen.

Die Dream-Consolidation produziert zwar *Fakten über Entscheidungen*, aber keine *strukturierten Entscheidungen*. Die Prompt-Strategie fragt explizit nach „Key decisions and WHY“, aber das Ergebnis ist stets ein narrativer Fließtext ohne trembare What/Why/Alternatives/Consequences-Felder. Die L0-Kompression reduziert dies weiter auf einen Ein-Satz-Brei.

---

## 2. Systemarchitektur der Consolidation

### 2.1 Fractal Compaction Chain

```
L2 (Raw)          →  L1 (Overview)     →  L0 (Summary)
Originaler Text      Paragraph           Ein Satz
context_tier=raw     context_tier=      context_tier=
                     overview            summary
```

- **L1 Prompt:** „Summarize in 2-3 sentences. Sentence 1: key decisions made and WHY. Sentence 2: important facts. Sentence 3: entities and timestamps.“ (max 100 Wörter)
- **L0 Prompt:** „Summarize in ONE sentence (≤20 words). If any decisions were made, state the decision AND the reason. Include the word 'decision' or 'decided'.“
- **Entscheidungserkennung:** Keyword-basiert (`is_decision_content()`) — sucht nach „decision:“, „entscheidung“, „decided“, „entschieden“ im L1-Content
- **Modell:** Ollama llama3.2, Temperature=0, Seed=42, num_predict=50 (L0) / 200 (L1)

### 2.2 Node-Statistik

| Metrik | Wert |
|--------|------|
| Total Nodes | 1330 |
| Semantic | 666 (50.1%) |
| Episodic | 646 (48.6%) |
| Decision | 18 (1.4%) |
| Nodes mit Children (consolidiert) | 435 (32.7%) |
| Source = consolidation | 684 (51.4%) |
| Source = conversation | 595 (44.7%) |
| Source = import | 51 (3.8%) |

---

## 3. Analyse von 15 Consolidation-Ergebnissen

### 3.1 Entscheidungs-Klassifikation

| # | Node ID | Tier | Typ | Enthält Entscheidung? | What/Why getrennt? | Alternatives? | Consequences? | Mit Fakten vermischt? |
|---|---------|------|-----|----------------------|---------------------|---------------|---------------|----------------------|
| 1 | `2fbe0a30` | overview | decision | ✅ Ja | ❌ Fließtext | ❌ Nein | ❌ Nein | ✅ Ja — „Key Decisions and Why“ als Header, aber alles in einem Absatz |
| 2 | `7cd18b16` | overview | decision | ✅ Ja | ❌ Fließtext | ❌ Nein | ❌ Nein | ✅ Ja — Entscheidung + „Important facts“ + „Entities“ gemeinsam |
| 3 | `082221e7` | overview | decision | ❌ Nein (Meta) | ❌ N/A | ❌ Nein | ❌ Nein | ✅ Ja — „Keine Entscheidungen existieren“ + Fakten zur Consolidation |
| 4 | `7347d1d4` | overview | decision | ✅ Ja | ❌ Fließtext | ❌ Nein | ❌ Nein | ✅ Ja — „Key decisions AND WHY“ im selben Satz wie Facts |
| 5 | `1bd2c84a` | overview | decision | ❌ Nein | ❌ N/A | ❌ Nein | ❌ Nein | ✅ Ja — „No key decisions“ + Skill-Update Fakten |
| 6 | `7b331118` | overview | decision | ✅ Ja | ❌ Fließtext | ❌ Nein | ❌ Nein | ✅ Ja — Entscheidung + Projekt-State im selben Block |
| 7 | `6e607ba3` | overview | semantic | ❌ Nein | ❌ N/A | ❌ Nein | ❌ Nein | ✅ Ja — Fakten über KnowWhere ohne Entscheidung |
| 8 | `d2b63c7a` | overview | semantic | ✅ Ja (Content hat Entscheidung) | ❌ Fließtext | ❌ Nein | ❌ Nein | ✅ Ja — „Key decisions made and WHY“ im Prompt-Format |
| 9 | `e51914ac` | overview | semantic | ✅ Ja | ❌ Fließtext | ❌ Nein | ❌ Nein | ✅ Ja — Entscheidung + Facts + Entities im Fließtext |
| 10 | `f438895d` | overview | decision | ❌ Nein | ❌ N/A | ❌ Nein | ❌ Nein | ✅ Ja — 3 Bulletpoints, aber semantisch kein echtes Decision-Objekt |
| 11 | `63983cdb` | summary | decision | ⚠️ Fragment | ❌ Nein | ❌ Nein | ❌ Nein | ✅ Ja — „Migration is not necessary“ — Entscheidung ohne Kontext |
| 12 | `a490945e` | summary | decision | ✅ Ja (1 Satz) | ❌ Nein | ❌ Nein | ❌ Nein | ❌ Nur 1 Satz, aber kein Why |
| 13 | `5af18447` | summary | decision | ✅ Ja (1 Satz) | ❌ Nein | ❌ Nein | ❌ Nein | ❌ Nur 1 Satz, kein Why |
| 14 | `e21712e6` | summary | decision | ❌ Nein | ❌ N/A | ❌ Nein | ❌ Nein | ✅ Ja — „No decision was made“ + Fakten |
| 15 | `1217f2d2` | overview | semantic | ⚠️ Indirekt | ❌ Nein | ❌ Nein | ❌ Nein | ✅ Ja — User-Wunsch + Technologie-Entscheidung vermischt |

### 3.2 Typische Konsolidierungs-Beispiele

**Beispiel 1: L1 — Entscheidung mit Rest vermischt (7cd18b16)**
```
Key decisions made and WHY: Ein `/retrieve_decisions` Endpoint wurde geschaffen,
um eine vollständige Liste aller Entscheidungen zu liefern, im Gegensatz zum
generischen `/retrieve_fractal`. Dieser Unterschied liegt darin, dass die
`/retrieve_decisions`-Query typisiert ist und keine Filterung mehr ermöglicht.

Important facts: Der `/retrieve_decisions`-Endpoint liefert eine Top-50 Liste
aller Entscheidungen oder alle Entscheidungen innerhalb eines bestimmten Zeitraums.

Entities and timestamps: Es handelt sich um einen speziellen Endpoint...
```
→ *Die Entscheidung ist im ersten Absatz, aber nicht als separates Feld extrahierbar.*

**Beispiel 2: L0 — Komplett geplättet (63983cdb)**
```
Migration is not necessary.
```
→ *Aus einer komplexen Diskussion über Fractal Nodes, Decision-Typing und Migration bleibt EIN Satz ohne Warum.*

**Beispiel 3: L0 — Entscheidung ohne Why (a490945e)**
```
A new `/retrieve_decisions` endpoint was created to provide a complete list
of decisions, differing from the generic `/retrieve_fractal`.
```
→ *Was wurde entschieden? Ja. Warum? Fehlt.*

---

## 4. „Warum?“-Query Recall-Test

Getestet wurden 5 Warum-Fragen gegen alle 1330 Nodes via Keyword-Matching (simulierte semantische Suche).

| Query | Exakte Matches (alle Keywords) | Partielle Matches | Recall-Qualität |
|-------|-------------------------------|-------------------|-----------------|
| „Warum wurde Docker entfernt?“ | 28 | 745 | ⚠️ Mittel — viele Matches, aber meist in langen Raw-Nodes, nicht in consolidierten L0/L1 |
| „Warum wurde MemoryType::Decision implementiert?“ | 8 | 662 | ❌ Schlecht — nur 8 Nodes enthalten alle Kontext-Keywords |
| „Warum Fractal Zooming implementiert?“ | 47 | 517 | ⚠️ Mittel — viele Raw-Konversationen, kaum konsolidierte |
| „Warum keine Migration der alten Decision Nodes?“ | **0** | 544 | ❌ Katastrophal — kein einziger Node enthält alle Keywords |
| „Warum wurde /retrieve_decisions erstellt?“ | **0** | 462 | ❌ Katastrophal — kein exakter Match |

### Recall-Statistik

- **Durchschnittliche exakte Recall-Rate: 17 von 1330 (1.3%)**
- **Durchschnittliche partielle Recall-Rate: 586 von 1330 (44%)**
- **Nodes mit „why“-Kontext in consolidierter Form: <5%**
- **Kritisch:** Zero-Matches für spezifische Warum-Fragen („keine Migration“, „retrieve_decisions erstellt“)

---

## 5. Root-Cause-Analyse

### 5.1 Was die Consolidation produziert

1. **L1 (Overview):** Narrative Prosa mit Prompt-Struktur („Key decisions and WHY“, „Important facts“, „Entities“). Struktur ist *im Prompt*, aber nicht *im Node-Schema*. Der Content ist ein einziger String.

2. **L0 (Summary):** Ein Satz. Verliert fast immer das „Why“. Beispiel: Aus „Die wichtigsten Entscheidungen sind die Implementierung eines optionalen `memory_type` Filters [...] um präzise Retrieval-Results zu liefern“ wird „Die Implementierung eines optionalen `memory_type` Filters wurde entschieden, um präzise Retrieval-Results zu liefern“ — das Why („präzise Retrieval-Results“) bleibt, aber alle Alternativen und Konsequenzen sind weg.

3. **Kein strukturiertes Decision-Objekt:** Es gibt keine Felder wie `decision_what`, `decision_why`, `decision_alternatives`, `decision_consequences`. Alles ist ein `content: String`.

### 5.2 Drei Kernprobleme

| Problem | Beschreibung | Impact |
|---------|-------------|--------|
| **P1: Text-Klumpen** | Content ist immer ein einziger String. Keine semantische Trennung von Entscheidung vs. Fakten vs. Kontext. | Retrieval findet den Node, aber der LLM-Consumer muss das Why selbst aus dem Fließtext extrahieren. |
| **P2: L0-Kompression tötet Why** | L0-Prompt: „ONE sentence (≤20 words). If any decisions were made, state the decision AND the reason.“ Aber 20 Wörter reichen selten für Decision+Reason. | L0-Nodes sind für Warum-Fragen unbrauchbar. |
| **P3: Keyword-basierte Decision-Erkennung** | `is_decision_content()` sucht nach „decision:“, „entscheidung“. Nodes wie `d2b63c7a` (Content beginnt mit „**Key decisions made and WHY:**“) werden NICHT als Decision getaggt, weil der String „decision:“ (mit Doppelpunkt) fehlt. | False negatives: Semantic Nodes enthalten Entscheidungen, sind aber nicht als Decision typisiert. |

### 5.3 Konkrete Beispiele geplätteter Entscheidungen

**Geplättet #1 — Docker-Entfernung:**
- L2 (Raw): Mehrere Konversationen über „Docker vollständig entfernen, 36 GB freigeben“
- L1 (Overview): „The decision to update the Prefetch Plugin Extension was made to double the firing of ZWEIMAL and store Decision-Nodes in the Index.“ — Docker wird gar nicht erwähnt!
- L0 (Summary): „The decision to update the Prefetch Plugin Extension was made to double the firing of ZWEIMAL and store Decision-Nodes in the Index.“
- **Verlust:** Der gesamte Docker-Kontext wurde von der Consolidation als unwichtig eingestuft.

**Geplättet #2 — Fractal Node Migration:**
- L2 (Raw): Ausführliche Diskussion über Migration vs. Nicht-Migration alter Decision-Nodes
- L1 (Overview): „1. Keine wichtigen Entscheidungen. 2. Semantische Suche funktioniert trotz Filter. 3. Migration ist nicht notwendig.“
- L0 (Summary): „Migration is not necessary.“
- **Verlust:** Das gesamte Reasoning (3 Gründe: nur 3 Nodes betroffen, semantische Suche findet sie, Overkill) ist auf 1 Satz reduziert.

---

## 6. Fazit

### Hypothese: BESTÄTIGT ✅

Die Dream-Consolidation produziert narrative Prosa, die Entscheidungen mit Fakten, Kontext und Meta-Kommentaren in einem einzigen Text-Klumpen vermischt. Es gibt keine programmatisch trembaren What/Why/Alternatives/Consequences-Felder.

**Spezifisch:**
- **L1** hat eine Prompt-Struktur (Sentence 1: Decisions, Sentence 2: Facts, Sentence 3: Entities), aber diese Struktur ist nicht im Node-Schema abgebildet
- **L0** komprimiert auf 1 Satz und verliert dabei fast immer das Why
- **Entscheidungserkennung** via Keywords ist fragil (false negatives)
- **0% Recall** für spezifische Warum-Fragen in der L0-Ebene

### Empfehlungen

1. **Strukturierte Decision-Felder im Node-Schema:** `decision_what`, `decision_why`, `decision_alternatives`, `decision_consequences` als separate Felder
2. **L0-Prompt überarbeiten:** Statt „ONE sentence ≤20 words“ → „State: [Decision]: X. [Why]: Y.“
3. **Entscheidungserkennung verbessern:** Statt Keywords → semantische Klassifikation oder Prompt-Output-Parsing
4. **Warum-Index:** Separate Embeddings für `decision_why`-Feld, um „warum?“-Queries direkt zu beantworten

---

*Generiert am 2026-05-04, 10:17 UTC von Hermes Agent*

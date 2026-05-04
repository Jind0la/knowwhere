# Competitive Landscape: Structured Knowledge Extraction in Memory Systems

**Datum:** 2026-05-04
**Analyst:** Hermes Agent
**Fragestellung:** Welches existierende Memory-System behandelt Entscheidungen/Claims als First-Class-Objekte — nicht nur als narrative Summaries?

---

## Vergleichstabelle

| System | Strukturierte Claims? | Decision-Typ? | Provenienz (Pointer)? | Kausale Verknüpfungen? | Intent-Aware Retrieval? | Claim-Extraktion |
|--------|----------------------|---------------|----------------------|------------------------|------------------------|-----------------|
| **Mem0** | ❌ Nur Fakten | ❌ | ❌ Kein Pointer zur Quelle | ⚠️ Im Mem0g-Graph (Entitäten-Relationen) | ❌ | ✅ Automatisch per LLM aus Konversationen |
| **Mem0g** | ❌ KG für Entitäten, nicht Claims | ❌ | ❌ | ✅ Entitäten-Relationen (Person-kennt-Person) | ❌ | ✅ Automatisch |
| **Letta/MemGPT** | ❌ Memory Blocks (Prosa) | ❌ Agent entscheidet selbst | ❌ Kein Audit-Trail | ❌ | ❌ | ⚠️ Agent schreibt eigenes Gedächtnis (Self-Reporting) |
| **CASS** | ⚠️ "Accomplishments, Decisions, Anti-Patterns" im Prompt | ❌ Freitext, kein Schema | ❌ | ❌ | ❌ | ✅ Automatisch aus Session-History |
| **GraphRAG (Microsoft)** | ⚠️ Extracted Claims + Entities | ❌ Claims sind generisch | ✅ TextUnit-Referenzen | ⚠️ Community-Hierarchie, keine expliziten Kausalkanten | ⚠️ Global vs Local Search | ✅ Claim-Extraktion aus TextUnits |
| **HugRAG** | ✅ Hierarchische Claims | ❌ | ✅ Entity-Level | ✅ Kausale Hierarchie (suppress spurious correlations) | ❌ | ✅ Automatisch |
| **CausalRAG** | ✅ Causal Claims | ❌ | ✅ Entity-Level | ✅ Explizite Kausalkanten (cause→effect) | ⚠️ Reranking nach Kausalität | ✅ Causal Graph Construction |
| **OpenAI Memory** | ❌ Fakten-Liste | ❌ | ❌ | ❌ | ❌ | ✅ Automatisch |
| **Zep / Graphiti** | ⚠️ Temporal KG | ❌ | ✅ Episode-Referenzen | ⚠️ Temporal, nicht kausal | ❌ | ✅ Automatisch |
| **Cognee** | ⚠️ KG aus Daten | ❌ | ✅ | ⚠️ Graph-Struktur | ❌ | ✅ KG-Construction Pipeline |

---

## Detaillierte Analyse pro System

### Mem0 (ECAI 2025)
- **Ansatz:** LLM extrahiert Fakten aus Konversationen → embeddet → speichert in Vektordatenbank
- **Mem0g-Erweiterung:** Fügt Knowledge Graph hinzu — aber KG mapped **Entitäten** (Personen, Objekte), nicht Entscheidungen
- **Benchmark:** LOCOMO — misst Fakt-Recall („What is the user's favorite color?"), nicht Entscheidungs-Reasoning
- **Ergebnis:** +1.5pp durch KG (66.9% → 68.4%). Irrelevant für Decision-Queries.
- **Fazit:** Gut für Personalisierung. Kein Decision-Typ, kein Why-Tracking, keine Provenienz.

### Letta/MemGPT (arXiv 2310.08560)
- **Ansatz:** LLM-as-OS — Agent managt eigenen Speicher in Tiers (Core/Archival/Recall)
- **Memory Blocks:** Strukturierte Blöcke (Persona, Human, Agent State) — aber Inhalt ist Prosa
- **Kritisches Problem:** Der Agent, der die Entscheidung trifft, entscheidet AUCH ob sie speicherwürdig ist. Wie ein Richter, der das Protokoll führt. Kein unabhängiger Extraktionsprozess.
- **Fazit:** Architektonisch interessant (Tiered Memory). Aber Self-Reporting ist der falsche Ansatz für Decision-Tracking. Kein Decision-Typ, keine Kausalität.

### CASS (github.com/Dicklesworthstone/cass_memory_system)
- **Ansatz:** „Procedural memory for AI coding agents" — extrahiert Accomplishments, Decisions, Anti-Patterns aus Session-History
- **Prompt-basiert:** Fordert im System-Prompt „DECISIONS:" als Kategorie — aber Output ist Freitext
- **Kein Schema:** Keine strukturierten Felder für What/Why/Alternatives
- **Keine Query-API:** Decisions sind Text im Working Memory, nicht abfragbar
- **Fazit:** Kommt KnowWhere's Idee am nächsten (Decisions als Kategorie). Aber bleibt auf Prompt-Ebene stecken — keine Struktur, keine Query, kein Pointer-Tracing.

### GraphRAG (Microsoft, arXiv 2404.16130)
- **Ansatz:** Extrahiert Knowledge Graph (Entities + Relationships + Claims) aus TextCorpus → Community-Hierarchie → Summaries
- **Claims:** Werden extrahiert! Aber als generische „Key Claims", nicht typisiert als Decision/Fact/Preference
- **Query:** Global Search (Community Summaries) und Local Search (Entity Fan-Out). Kein Intent-Aware Retrieval.
- **Stärke:** Claim-Extraktion + Hierarchie + TextUnit-Provenienz. Das sind Patterns, die wir übernehmen können.
- **Schwäche:** Kein Decision-Typ, keine kausalen Kanten zwischen Claims, kein Supersession-Tracking.
- **Fazit:** Beste existierende Claim-Extraktion. Aber Claims sind generisch — GraphRAG unterscheidet nicht zwischen „Die CPU hat 8 Kerne" (Fakt) und „Wir haben uns für PostgreSQL entschieden weil..." (Entscheidung).

### HugRAG (arXiv 2602.05143)
- **Ansatz:** Hierarchical Causal KG — modelliert kausale Beziehungen explizit, unterdrückt spurious correlations
- **Innovation:** Kausale Hierarchie (nicht nur Assoziation). Claims auf verschiedenen Abstraktionsebenen.
- **Schwäche:** Research-Framework, kein Production-System. Kein Decision-Typ. Fokussiert auf RAG-Qualität, nicht Agent-Memory.
- **Fazit:** Die kausale Hierarchie ist das interessanteste Pattern. „X caused Y" als First-Class-Relation — genau das, was KnowWhere für Entscheidungs-Claims braucht.

### CausalRAG (ACL Findings 2025)
- **Ansatz:** Integriert Causal Graphs in RAG-Pipeline. Konstruiert kausale Beziehungen, nutzt sie für Retrieval-Reranking.
- **Ergebnis:** Übertrifft reguläres RAG und GraphRAG auf mehreren Metriken
- **Relevanz:** Zeigt dass kausales Reranking funktioniert. „Warum?"-Queries profitieren von Causal-Priorisierung.
- **Schwäche:** General-Purpose RAG, kein Agent-Memory. Keine Decision-Typen, keine Persistenz über Sessions.
- **Fazit:** Beweist den Wert von Intent-Aware Retrieval. CausalRAG's Reranking-Ansatz ist direkt übertragbar auf KnowWhere's Warum-Query-Detektor.

### Zep / Graphiti
- **Ansatz:** Temporal Knowledge Graph — speichert Fakten mit Zeitstempeln und Episode-Referenzen
- **Stärke:** Temporal Awareness („Was war true im April vs. jetzt?")
- **Schwäche:** Temporal ≠ Causal. „X passierte vor Y" ≠ „X verursachte Y". Kein Decision-Typ.
- **Fazit:** Zeitstempel-basierte Provenienz ist nützlich, aber kein Ersatz für kausale Verknüpfungen.

### Cognee
- **Ansatz:** Knowledge Graph aus Daten vor Queries — „build graph first, query later"
- **Stärke:** KG-Construction Pipeline mit RAG-Integration
- **Schwäche:** Fokussiert auf Daten-Graphen (Dokumente, Codebases), nicht Konversations-Extraktion
- **Fazit:** Pipeline-Ansatz interessant, aber andere Domäne.

---

## Kernbefund

**Kein existierendes System behandelt Entscheidungen als First-Class-Objekte mit strukturiertem Schema (What/Why/Alternatives/Consequences) + Provenienz (Pointer zur Quelle) + kausalen Verknüpfungen (Supersedes/Caused-by).**

Die Lücke ist real und niemand hat sie geschlossen:

1. **Mem0/Mem0g** macht Fakten-Extraktion gut, aber ohne Decision-Typ
2. **Letta** hat die richtige Tiered-Architektur, aber Self-Reporting ist ungeeignet für Entscheidungen
3. **CASS** nennt Decisions beim Namen, bleibt aber auf Prompt-Ebene ohne Struktur
4. **GraphRAG** extrahiert Claims, aber generisch — kein Decision-vs-Fact-Unterschied
5. **HugRAG/CausalRAG** modellieren Kausalität, aber für RAG, nicht für Agent-Memory

KnowWhere kann das erste System sein, das **strukturierte Entscheidungs-Claims mit kausalen Verknüpfungen und Pointer-Provenienz** als Kern-Feature anbietet.

---

## Übernehmbare Patterns (Empfehlung)

### Von GraphRAG übernehmen:
- **Claim-Extraktion als separater Schritt** (nicht in narrative Summary eingebettet)
- **TextUnit-Referenzen** für Provenienz (→ KnowWhere's `session_id + turn_index`)
- **Hierarchische Community-Summaries** (→ KnowWhere's L2→L1→L0 Fractal)

### Von HugRAG/CausalRAG übernehmen:
- **Kausale Kanten** zwischen Claims („X caused Y", „Z superseded W")
- **Causal Reranking** bei Queries („Warum?" → priorisiere Claims mit `reason`-Feld)

### Von CASS übernehmen:
- **Decisions als explizite Kategorie** im Extraktions-Prompt
- **Anti-Pattern:** CASS's Freitext-Ansatz. Wir brauchen Schema.

### Von Letta übernehmen:
- **Tiered Memory-Architektur** (→ KnowWhere hat das schon: L2/L1/L0)
- **Anti-Pattern:** Self-Reporting. Extraktion muss passiv und unabhängig sein.

### Von Mem0g übernehmen:
- **Graph-Repräsentation für Beziehungen** (aber für Claims, nicht nur Entitäten)
- **Anti-Pattern:** LOCOMO-Benchmark. Misst Fakt-Recall, nicht Entscheidungs-Qualität.

---

## Fazit für KnowWhere

Die Recherche bestätigt: **KnowWhere's Ansatz (strukturierte Decision-Claims + Pointer-Provenienz) ist ein unbesetztes Feld.** Kein Konkurrent macht das. Der Weg ist:

1. Claim-Extraktion à la GraphRAG — aber typisiert (Decision/Fact/Preference)
2. Kausale Verknüpfungen à la HugRAG/CausalRAG — zwischen Claims
3. Pointer-Provenienz à la KnowWhere (existiert schon!) — `session_id + turn_index`
4. Intent-Aware Retrieval à la CausalRAG — Warum-Queries priorisieren Decision-Claims

Das ist kein neues Feature. Es ist die logische Weiterentwicklung dessen, was KnowWhere bereits tut.

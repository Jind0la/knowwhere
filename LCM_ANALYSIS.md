# hermes-lcm Analyse — Was KnowWhere davon lernen kann

> **Quelle**: https://github.com/stephenschoettler/hermes-lcm (v0.10.0)
> **Typ**: Hermes Agent Plugin — Lossless Context Management
> **Architektur**: DAG-basierte hierarchische Zusammenfassung mit Source-Tracking

---

## Architektur-Überblick

hermes-lcm ist ein Kontext-Management-Plugin für Hermes Agent. Es ersetzt den eingebauten ContextCompressor mit einem DAG-basierten System, das KEINE Nachricht verliert.

### Kernkomponenten

| Komponente | Datei | Funktion |
|---|---|---|
| SummaryDAG | `dag.py` | SQLite-backed DAG für hierarchische Summaries |
| Engine | `engine.py` | ContextEngine ABC-Implementierung |
| Escalation | `escalation.py` | 3-Level LLM→deterministic Fallback |
| Externalize | `externalize.py` | Auslagerung großer Tool-Outputs |
| Schemas | `schemas.py` | Agent-Tools: lcm_grep, lcm_expand, lcm_describe |

### DAG-Tiefen

```
D0 — Minuten-Zeitskala: Leaf-Summaries aus Raw-Messages
D1 — Stunden: Kondensation von D0-Nodes
D2 — Tage: Kondensation von D1-Nodes
D3+ — Wochen/Monate: Weitere Verdichtung
```

Jeder Node hat:
- `source_ids: List[int]` — IDs der Quell-Nodes/Messages
- `source_type: str` — "messages" oder "nodes"
- `depth: int` — Tiefe im DAG
- `expand_hint: str` — "Expand for details about: ..."

---

## Was KnowWhere übernehmen sollte

### 1. Source-Lineage (✅ JETZT implementiert)

**Problem**: KnowWhere-Claims wissen nicht, aus welchen Raw-Nodes sie entstanden sind. Eine konsolidierte L1-Claim "User mag X" hat keine Verbindung zu den L0-Claims die sie rechtfertigen.

**Lösung**: Jede Node speichert `source_ids` + `source_timestamps`. Bei Retrieval wird die Lineage mitgeliefert.

### 2. 3-Level Escalation (⬜ Phase 2)

LCM's Consolidation hat garantierten Fallback:
- L1: LLM-Summary (preserves detail)
- L2: Bullet-Point-Kondensation (half budget)
- L3: Deterministische Trunkierung (kein LLM, konvergiert garantiert)

KnowWhere hat KEINEN Fallback — wenn der LLM-Call fehlschlägt, passiert nichts.

### 3. Agent-Tools (⬜ Phase 3)

Für Multi-Turn-Agent-Mode:
- `kw_expand(id)` — Drill-Down von Summary zu Raw-Claims
- `kw_trace(source_id)` — Quellen einer Behauptung folgen
- `kw_grep(query)` — Volltextsuche über Raw-Claims
- `kw_describe(id)` — Metadaten zu einer Node

---

## Was KnowWhere NICHT übernehmen sollte

- **Immutable-First Store**: KnowWhere ist kein Chat-Verlauf, sondern ein Memory-System
- **Session-Filtering via Glob-Patterns**: Nicht relevant für externe Claims
- **Transcript GC Placeholders**: KnowWhere speichert keine Transkripte
- **Message-Token-Counting**: Anderes Metrik-Modell

---

## Relevanz für PersonaMem Benchmark

Der Benchmark testet Präferenz-Tracking über Zeit. Source-Lineage erlaubt:
1. Unterscheidung: Einzel-Claim vs. konsolidierte Summary
2. Zeitliche Einordnung: "Diese Präferenz wurde über 3 Sessions bestätigt"
3. Widerspruchserkennung: "Session 5 sagt X, Session 8 sagt Y"
4. Gewichtung: "2 Raw-Claims + 1 L1-Summary → hohes Vertrauen"

---

*Repo für spätere Referenz: `git clone https://github.com/stephenschoettler/hermes-lcm.git ~/hermes-lcm`*

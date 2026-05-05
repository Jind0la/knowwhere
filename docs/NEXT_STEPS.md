# Next Steps — KnowWhere v0.5.0 → v1.0.0

> Stand: 2026-05-05, nach Decision-Scoring-Fix und MemoryType::parse-Reparatur.
> Entscheidungen basieren auf Live-A/B-Tests mit 847 Decision-Nodes und 2154 Total-Nodes.

---

## 1. ⚡ JETZT: Consolidation erzeugt keine Decision-Typen

### Problem

Die `store_session`-API akzeptiert jetzt `memory_type: "decision"` und speichert korrekt als `MemoryType::Decision`. Aber die **automatische Consolidation** (`src/scheduler/consolidation.rs`, `src/summarizer/mod.rs`) erzeugt Summary-Nodes mit `memory_type: Semantic` — selbst wenn sie explizit Decision-Content enthalten.

**Beweis aus dem A/B-Test:**
```
Query: "why PostgreSQL instead of SQLite"
→ Position 1-3: semantic (trust=derived, mult=0.88)
→ KEIN Decision-Node, obwohl es PostgreSQL-Entscheidungen gab
```

Die 847 Decision-Nodes in der DB sind fast alle aus manuellen `store_session`-Calls oder dem Batch-Retype. Die automatische Pipeline produziert sie nicht.

### Fix (geschätzt 2-3 Stunden)

1. **`src/scheduler/consolidation.rs`:** `is_decision_content()` prüft bereits auf "DECISION:", "decided", "Entscheidung". Der erkannte Typ muss als `MemoryType::Decision` in den generierten Node übernommen werden.
2. **`src/summarizer/mod.rs`:** `LocalSummarizer::summarize()` muss bei Decision-Content `MemoryType::Decision` statt `Semantic` setzen.
3. **Test:** Consolidation-Loop durchlaufen lassen, dann `retrieve_fractal` mit `memory_type_filter=decision` — es sollten N＞0 Decision-Nodes aus Consolidation erscheinen.

**Begründung:** Ohne diesen Fix wächst der Decision-Bestand nur durch explizite API-Calls, nicht organisch. 80%+ der zukünftigen Decision-Nodes werden durch Consolidation entstehen müssen.

---

## 2. 🔜 Cross-Encoder Reranking aktivieren

### Problem

Der Cross-Encoder (`bge-reranker-v2-m3`, 491 Zeilen Code) ist vollständig implementiert, getestet und verbessert die Precision um **+33-42%**. Aber er ist **nicht kompiliert** — der `reranker` Feature-Flag fehlt im Default-Build.

**Aktivierung:**
```bash
SQLX_OFFLINE=true cargo build --release --features "postgres-storage,summarizer,reranker"
```

**Trade-off:** +2.5 GB RAM für das ONNX-Modell. Auf einem 8 GB M1 MacBook Air bedeutet das: Ollama (1 GB) + ONNX (2.5 GB) + KnowWhere (~500 MB) = 4 GB. Mit System-Overhead bleiben ~2 GB für andere Apps. Akzeptabel für Dev, aber für ein 8 GB Deployment muss der Reranker optional bleiben.

**Begründung:** +33-42% Precision ist der größte einzelne Qualitätssprung, der sofort verfügbar ist. Keine neuen Features nötig — nur ein Rebuild.

---

## 3. 🔜 Decision-Nodes aus existierenden Summaries extrahieren

### Problem

~880 `semantic` Summary-Nodes enthalten Decision-Content ("Key decisions made", "Decision:", "Entscheidung:"), sind aber als `semantic` getypt. Der Batch-Retype (PostgreSQL `UPDATE`) hat 474 davon auf `decision` umgestellt, aber pattern-basiertes Retyping ist ungenau — es erwischt False Positives ("No decision was made") und verpasst False Negatives (Entscheidungen ohne "Decision:" Prefix).

### Ansatz

Ein Admin-Endpoint `POST /maintenance/retype_decisions` der:
1. Alle `semantic` Nodes mit bestimmten Patterns findet
2. Den Content durch llama3.2 schickt mit Prompt: "Enthält dieser Text eine konkrete Entscheidung? Antworte nur YES oder NO."
3. Bei YES → `memory_type` auf `decision` updated

**Begründung:** LLM-basiertes Retyping ist präziser als Regex und skaliert (automatisch für zukünftige Summaries). Ersetzt den manuellen SQL `UPDATE` der nur einmal läuft.

---

## 4. 📋 Quality-of-Life

### 4a. `cargo test --features postgres-storage` ohne manuelles DATABASE_URL

Integration-Tests brauchen `DATABASE_URL`. `CI=true` aktiviert einen `FixedEmbeddingProvider(768)`, aber das Flag wird nicht dokumentiert. Entweder:
- Default `DATABASE_URL` in Test-Harness setzen, oder
- Integration-Tests mit `#[ignore]` markieren wenn `DATABASE_URL` fehlt

### 4b. USearch Warnings unterdrücken

"Reserve capacity ahead of insertions" erscheint bei fast jeder Insertion. Kein funktionaler Impact, aber füllt die Logs. `tracing::warn` → `tracing::debug` oder USearch-Kapazität vorab alloziieren.

### 4c. README Version Auto-Detect

`grep 'version' Cargo.toml` → README-Version auto-updaten via CI/Pre-Commit. Manuelles Sync produziert Staleness.

---

## 5. 🧪 Messung: Retrieval Quality Tracking

### Problem

Wir haben kein systematisches Tracking ob Retrieval *besser* wird. Der A/B-Test war manuell.

### Ansatz

Ein minimales Eval-Framework:
```bash
curl -X POST /eval/cases  # 20 hand-crafted Query→Expected-Node-ID pairs
curl -X POST /eval/run    # Returns precision@5, recall@5, MRR
```
Wird vor jedem Release ausgeführt. Regression wird sofort sichtbar.

**Begründung:** Ohne Metriken ist jede Änderung am Scoring ein Blindflug. Der LongMemEval-Benchmark (50 Cases) existiert bereits — er muss nur automatisierbar werden.

---

## Priorisierung

| # | Item | Impact | Effort | Risk | Order |
|---|------|--------|--------|------|-------|
| 1 | Consolidation → Decision-Typen | 🔴 Hoch | 2-3h | Niedrig | **1** |
| 2 | Cross-Encoder aktivieren | 🔴 Hoch | 30min | RAM (2.5GB) | **2** |
| 3 | LLM-basiertes Decision-Retyping | 🟡 Mittel | 3-4h | OpenAI-Kosten | **3** |
| 4a | Test-Ergonomie | 🟢 Niedrig | 1h | Kein | 4 |
| 4b | USearch Warnings | 🟢 Niedrig | 30min | Kein | 5 |
| 5 | Retrieval Eval | 🟡 Mittel | 4-5h | Kein | 6 |

**Gesamtaufwand für 1+2 (kritischer Pfad):** 3 Stunden. Danach ist die Decision-Pipeline vollständig: Store → Consolidate → Type → Score → Retrieve.

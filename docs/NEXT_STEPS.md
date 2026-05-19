# Next Steps — KnowWhere v0.6.0 → v0.7.0

> Stand: 2026-05-19, nach Phase 2 Completion (Retrieval Quality & Benchmarking).  
> v0.6.0: 82 tasks, 8 initiatives, 72.97% Recall@5 on LongMemEval.  
> [→ Completion Report](phase2-retrieval-quality-completion.md)

---

## Erledigt (v0.6.0)

- **Turn-Level Storage + Per-Turn Embeddings:** Migration 014-017, 26 tasks ✅
- **Stratified LongMemEval Benchmark:** 42 cases, all 6 types functional ✅
- **Hybrid BM25 + Dense Retrieval:** Keyword + semantic fusion ✅
- **Source-Type Weighting + Provenance:** Real > Synthetic, 13 tasks ✅
- **Cross-Encoder Reranking:** gte-modernbert ONNX, 11 tasks ✅
- **Fact Extraction Pipeline:** Symbolic knowledge from conversations ✅
- **Temporal-Aware Scoring:** Recency decay ✅
- **Compilation Fixes:** All pre-existing errors resolved ✅

---

## 1. 🔜 NÄCHSTES: Full 500-Case LongMemEval Run

### Warum

Der 42-Case stratified Run zeigt 72.97% Recall@5, aber für harte Claims (Paper, Vergleich mit AgentMemory) brauchen wir den vollen 500-Case-Durchlauf.

### Plan

1. Run auf allen 500 Cases (nicht nur stratified subset)
2. Per-Type Breakdown mit statistischer Signifikanz
3. Vergleich mit AgentMemory's 50.4% und GPT-4 Full Context 60.7% auf identischem Setup
4. Kosten: ~4-6h Rechenzeit, 500 store_session_batch + 500 retrieve calls

### Risiko

Der stratified Filter könnte "leichtere" Cases selektiert haben. Der Full Run zeigt, ob die 73% halten.

---

## 2. 📝 Paper Draft finalisieren

Der Draft ist aktualisiert (docs/paper-draft.md). Nächste Schritte:

1. **Ablation Table füllen:** −Cross-Encoder, −Source-Weighting, −Temporal-Decay
2. **Related Work section:** AgentMemory, Mem0, Hindsight, Zep, Letta
3. **Figure 1:** Fractal Architecture Diagram
4. **LaTeX formatting** für arxiv submission

---

## 3. 🔬 Ablation Experiments

Mit dem vollen 500-Case Run:
- KnowWhere Full: 72.97% (baseline)
- − Turn-Level (Session-Level only): 7.1% (gemessen)
- − Cross-Encoder (Dense+BM25 only): ?
- − Source-Type Weighting: ?
- − Temporal Decay: ?

---

## 4. 📦 v0.7.0 — UX & Integration

- **Hermes Plugin v2.0:** Turn-Level Context Injection
- **Web Dashboard:** Retrieval Analytics + Node Explorer
- **Multi-Tenant Support:** API Keys + Namespaces
- **Python SDK v2.0:** Type-safe + async

---

## Historische Priorisierung (v0.5.0 → v0.6.0)

| # | Item | Impact | Effort | Status |
|---|------|--------|--------|--------|
| 1 | Provenance + Retrieval-Diversität | 🔴 Hoch | 1 Tag | ✅ Done |
| 2 | Postgres-Fractal-Expansion | 🔴 Hoch | 1-2 Tage | ✅ Done |
| 3 | Current-vs-Historical Repair | 🔴 Hoch | 1 Tag | ✅ Done |
| 4 | Cross-Encoder aktivieren | 🟡 Mittel | 30min | ✅ Done |
| 5 | Test-Ergonomie / Warnings | 🟢 Niedrig | 1-2h | ✅ Done |

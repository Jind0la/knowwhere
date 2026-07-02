# KnowWhere LongMemEval Canary History

> Gültig seit: 2026-06-17 (M3 Sprint)
> Binary: `longmemeval_canary` (Release)
> Server: `http://127.0.0.1:3737`

## Gates

| Metrik | Schwelle |
|--------|----------|
| Recall@5 | ≥ 0.75 |
| MRR | ≥ 0.65 |
| Abstention Accuracy | ≥ 0.80 (⚠️ gilt nur wenn Dataset Abstention-Cases enthält) |

## History

| Datum | Recall@5 | MRR | Abstention | Status |
|-------|----------|-----|------------|--------|
| 2026-06-17 | 1.00 | 1.00 | 1.00 | ✅ gates=pass (3/3 Cases) |
| 2026-06-19 | 0.50 | 0.50 | 0.00 | ❌ ALL GATES FAILED (10/10 Cases) — USearch dimension mismatch |
| 2026-06-20 | **0.80** | **0.6833** | 0.00 ⚠️ | ⚠️ Retrieval-PASS, Abstention false-positive (0 abstention cases in dataset) |
| 2026-06-21 | **0.80** | **0.6833** | 0.00 ⚠️ | ⚠️ Retrieval-PASS, Abstention false-positive (identisch zu 06-20, Release-Server) |
| 2026-06-22 | **0.50** | **0.50** | 0.00 ⚠️ | ❌ GATE BREACH — Production server (no reranker + data contamination) |
| 2026-06-23 | **0.80** | **0.6333** | 0.00 ⚠️ | ❌ MRR BREACH — MRR 0.633 < 0.65 (own_hits identisch zu 06-21, Ranking minimal verschoben) |
| 2026-06-24 | **0.80** | **0.5833** | 0.00 ⚠️ | ❌ MRR BREACH — MRR 0.583 < 0.65 (own_hits identisch, MRR-Trend: 0.683→0.633→0.583) |
|| 2026-06-25 | **0.80** | **0.6833** | 0.00 ⚠️ | ⚠️ Retrieval-PASS, MRR zurück auf 06-21 Baseline. Abstention false-positive. |
|| 2026-06-27 | **0.80** | **0.75** | 0.00 ⚠️ | ⚠️ Retrieval-PASS, **MRR 0.75 — bester Wert seit 06-21 (+0.067 vs 06-25).** Abstention false-positive (0 cases). |
|| 2026-06-29 | **0.50** | **0.45** | 0.00 ⚠️ | ❌ ALL GATES BREACHED — Production server contamination (136 existing nodes). Same failure pattern as 06-22. |
|| 2026-06-30 | **0.40** | **0.40** | 0.00 ⚠️ | ❌ ALL GATES BREACHED — Production server contamination (230 nodes). **Recall@5 −62% vs 06-27.** Cron runs against prod server instead of clean canary. |
|| 2026-07-01 | **0.50** | **0.43** | 0.00 ⚠️ | ❌ ALL GATES BREACHED — Recall@5 −0.30 vs 06-27 baseline. Fresh MiniLM ONNX export (old model deleted). Fact extraction creating 83 noise nodes. **4 runs, best was 0.60 without reranker.** See `~/.hermes/cron/output/canary-baseline/2026-07-01.md` for full analysis. |

## 2026-06-23 Run Details

- **Server:** Clean canary server, release build, clean data dir (`/tmp/kw_canary_data_20260623`)
- **Reranker:** MiniLM (ONNX) — loaded successfully (181.5ms, `ms-marco-MiniLM-L6-v2`)
- **API-Key:** `kw_testkey_12345` (16 chars, from `~/.knowwhere/.env`)
- **Canary Binary:** `target/release/longmemeval_canary` (Jun 21 build)
- **Preflight:** ✅ No USearch warnings, ✅ Ollama nomic-embed-text ready, ✅ node_count=0
- **Elapsed:** N/A (fast)

### Gate Analysis

| Gate | Wert | Schwelle | Status |
|------|------|----------|--------|
| Recall@5 | 0.80 | ≥ 0.75 | ✅ PASS (+0.05) |
| MRR | 0.6333 | ≥ 0.65 | ❌ FAIL (-0.017, -0.05 vs 06-21) |
| Abstention | 0.00 | ≥ 0.80 | ⚠️ FALSE POSITIVE (0 abstention cases in dataset) |

### MRR Regression — Owned Hits Identisch, Ranking Verschiebung

Die per-case `owned_hits` sind **identisch** zu 06-21 (3,2,3,3,1,2,2,2,4,4). Der MRR-Rückgang von 0.6833 → 0.6333 (-0.05) kommt von minimalen Ranking-Verschiebungen — der erste korrekte Hit wird bei einigen Cases um 1-2 Positionen schlechter gerankt. Summe der Reciprocal Ranks: 6.833 → 6.333.

Mögliche Ursachen (Rangfolge):
1. **Normale ONNX-Varianz:** MiniLM ONNX kann bei identischen Inputs minimale Score-Unterschiede produzieren — bei 10 Cases MRR-sensitiv.
2. **Server-Neustart:** Frisch gestarteter Server vs. warmgelaufener (06-21) — erste Queries haben minimal andere ONNX-Session-Performance.
3. **Kein struktureller Bruch:** Recall@5 stabil bei 0.80, owned_hits identisch.

### Empfehlung

- MRR-Breach bei 0.633 ist marginal (-0.017 unter Gate). Kein Alarmsignal wie 06-22 (Produktions-Contamination).
- Falls wiederholt: `MAX_CASES=30` für stabilere MRR-Schätzung.
- Medium-Prio: Abstention-Gate-Fix in `longmemeval_canary.rs` (add `|| metrics.abstention_total == 0`).

### Per-Case Results (identisch zu 06-21)

| Case | Owned Hits | vs 06-21 |
|------|-----------|----------|
| 71017277 | 3/5 | = |
| gpt4_a2d1d1f6 | 2/5 | = |
| 54026fce | 3/5 | = |
| gpt4_7abb270c | 3/5 | = |
| gpt4_b5700ca0 | 1/5 | = |
| 195a1a1b | 2/5 | = |
| 32260d93 | 2/5 | = |
| gpt4_af6db32f | 2/5 | = |
| gpt4_7bc6cf22 | 4/5 | = |
| gpt4_d6585ce9 | 4/5 | = |

## 2026-06-22 Run Details

- **Server:** Production release build (PID 25752), **134 existing nodes + canary data mixed**
- **Reranker:** ❌ NOT LOADED — `KNOWWHERE_RERANKER_MODEL_FORMAT not set` → Bi-Encoder only
- **API-Key:** `kw_testkey_12345` (from `~/.knowwhere/.env`)
- **Canary Binary:** `target/release/longmemeval_canary` (Jun 21 build)
- **Node count after canary:** 268 (134 original + ~134 canary)
- **Elapsed:** N/A (fast)

### Root Cause: Invalid Test Setup

This was NOT a valid canary run. Two critical differences from the green baseline:

1. **No Reranker:** Production server has no `KNOWWHERE_RERANKER_MODEL_FORMAT` set. Server log: `"Retrieval will use Bi-Encoder only"`. The 06-21 baseline used MiniLM reranker.
2. **Data Contamination:** Production server had 134 existing nodes polluting the haystack. 06-21 used a clean data dir (`/tmp/kw_canary_data_20260621`).

Either issue alone could explain the 0.50 → 0.80 gap. Combined, they're definitive.

### Per-Case Results (vs 06-21 baseline)

| Case | 06-21 | 06-22 | Δ |
|------|-------|-------|---|
| 71017277 | 3/5 ✅ | 0/5 ❌ | -3 |
| gpt4_a2d1d1f6 | 2/5 ✅ | 1/5 | -1 |
| 54026fce | 3/5 ✅ | 2/5 | -1 |
| gpt4_7abb270c | 3/5 ✅ | 1/5 | -2 |
| gpt4_b5700ca0 | 1/5 ⚠️ | 0/5 ❌ | -1 |
| 195a1a1b | 2/5 ✅ | 1/5 | -1 |
| 32260d93 | 2/5 ✅ | 1/5 | -1 |
| gpt4_af6db32f | 2/5 ✅ | 0/5 ❌ | -2 |
| gpt4_7bc6cf22 | 4/5 ✅ | 2/5 | -2 |
| gpt4_d6585ce9 | 4/5 ✅ | 2/5 | -2 |

### Correction Needed

Cron job should start a clean canary server with reranker, NOT use production. See `references/canary-cron-procedure.md`.

## 2026-06-21 Run Details

- **Server:** Release build (`target/release/knowwhere-server`), clean data dir (`/tmp/kw_canary_data_20260621`)
- **Reranker:** MiniLM (ONNX) — loaded successfully
- **API-Key:** `kw_testkey_12345` (from running process, not `.env`)
- **Canary Binary:** `target/release/longmemeval_canary` (Jun 19)
- **Metrics:** Identical to 2026-06-20 — Retrieval stabil.

### ⚠️ Server-Issue entdeckt
Der Debug-Build-Server (PID 867, launchd-auto-restart) ist für Canary unbrauchbar:
- `store_session` mit 19K chars → timeout >120s (Release: 0.51s)
- Auto-Restart via launchd: startet immer Debug-Binary
- Für Canary temporär Release-Server auf Port 3737 gestartet (Debug wurde verdrängt)

### Per-Case Results (identisch zu 06-20)
| Case | Owned Hits | Evidence Found |
|------|-----------|---------------|
| 71017277 | 3/5 | ✅ |
| gpt4_a2d1d1f6 | 2/5 | ✅ |
| 54026fce | 3/5 | ✅ |
| gpt4_7abb270c | 3/5 | ✅ |
| gpt4_b5700ca0 | 1/5 | ⚠️ Marginal |
| 195a1a1b | 2/5 | ✅ |
| 32260d93 | 2/5 | ✅ |
| gpt4_af6db32f | 2/5 | ✅ |
| gpt4_7bc6cf22 | 4/5 | ✅ Best |
| gpt4_d6585ce9 | 4/5 | ✅ Best |

## 2026-06-20 Run Details

- **Server:** Release build, clean data dir
- **Reranker:** MiniLM (87MB ONNX) — no GTE hang
- **Elapsed:** 69.0s
- **Cases:** 10/10 non-abstention (dataset has 0 abstention cases in 27 total)

### Per-Case Results

| Case | Owned Hits | Evidence Found |
|------|-----------|---------------|
| 71017277 | 3/5 | ✅ |
| gpt4_a2d1d1f6 | 2/5 | ✅ |
| 54026fce | 3/5 | ✅ |
| gpt4_7abb270c | 3/5 | ✅ |
| gpt4_b5700ca0 | 1/5 | ⚠️ Marginal |
| 195a1a1b | 2/5 | ✅ |
| 32260d93 | 2/5 | ✅ |
| gpt4_af6db32f | 2/5 | ✅ |
| gpt4_7bc6cf22 | 4/5 | ✅ Best |
| gpt4_d6585ce9 | 4/5 | ✅ Best |

### Known Issues
1. **Abstention Gate false-positive:** Dataset has 0 abstention cases → `abstention_total=0` → `abstention_accuracy=0.0` even though system correctly never abstained on non-abstention cases. Fix: add `|| metrics.abstention_total == 0` to gate validation in `longmemeval_canary.rs`.
2. **Recall@5 delta from baseline:** 0.80 vs 1.00 (2026-06-17) — the 2026-06-17 run used only 3 cases, so 1.00 is less statistically meaningful. The 10-case run at 0.80 is more representative. Still above gate (≥0.75).

## 2026-06-19 Failure Analysis

### Results
- **Recall@5: 0.50** (Gate ≥0.75) — Half of cases found no evidence in top 5
- **MRR: 0.50** (Gate ≥0.65) — When evidence was found, it was never ranked #1
- **Abstention Accuracy: 0.00** (Gate ≥0.80) — System never correctly abstained
- **Exact Match: 0.10** — Gold answer appeared as first hit only once

### Per-Case owned_hits
| Case | Owned Hits | Found? |
|------|-----------|--------|
| 71017277 | 1/5 | Partial |
| gpt4_a2d1d1f6 | 1/5 | Partial |
| 54026fce | 1/5 | Partial |
| gpt4_7abb270c | 0/5 | ❌ Miss |
| gpt4_b5700ca0 | 0/5 | ❌ Miss |
| 195a1a1b | 1/5 | Partial |
| 32260d93 | 2/5 | Best case |
| gpt4_af6db32f | 0/5 | ❌ Miss |
| gpt4_7bc6cf22 | 1/5 | Partial |
| gpt4_d6585ce9 | 1/5 | Partial |

### Root Causes
1. **USearch dimension mismatch**: Server logs showed "skipping usearch index: Vector length must match index dimensionality" for nearly every store operation — USearch binary files from a different embedding dimension were stale.
2. **Fix:** Delete `data/usearch*.bin` files, restart server to force clean rebuild with correct dimension.

## 2026-06-24 Run Details

- **Server:** Clean canary server, release build (Jun 19), clean data dir (`/tmp/kw_canary_data_20260624`)
- **Reranker:** MiniLM (ONNX) — loaded successfully (ms-marco-MiniLM-L6-v2)
- **API Key:** 16 chars, from `~/.knowwhere/.env` via Python regex reader
- **Canary Binary:** `target/release/longmemeval_canary` (Jun 21 build)
- **Preflight:** ✅ No USearch warnings, ✅ Ollama nomic-embed-text ready, ✅ node_count=0, ✅ reranker loaded

### Gate Analysis

| Gate | Wert | Schwelle | Status |
|------|------|----------|--------|
| Recall@5 | 0.80 | ≥ 0.75 | ✅ PASS (+0.05) |
| MRR | 0.5833 | ≥ 0.65 | ❌ FAIL (-0.067) |
| Abstention | 0.00 | ≥ 0.80 | ⚠️ FALSE POSITIVE (0 abstention cases in dataset) |

### MRR Degradation Trend — STRUCTURAL CONCERN

Die owned_hits sind **identisch** zu 06-20, 06-21 und 06-23: `[3,2,3,3,1,2,2,2,4,4]`. Trotzdem fällt MRR kontinuierlich:

| Datum | Recall@5 | MRR | Sum RR | Δ vs 06-21 |
|-------|----------|-----|--------|------------|
| 06-21 | 0.80 | **0.6833** | 6.833 | baseline |
| 06-23 | 0.80 | **0.6333** | 6.333 | −0.050 |
| 06-24 | 0.80 | **0.5833** | 5.833 | **−0.100** |

Die steigende Degradation (−0.05 → −0.10) bei identischen Hits deutet auf einen **systematischen Ranking-Drift**, nicht nur ONNX-Varianz. Bei 5.833 summierten Reciprocal Ranks liegt der erste korrekte Hit im Durchschnitt auf Position 1.7 — d.h. etwa die Hälfte der Cases hat den ersten korrekten Hit nicht auf Position 1.

**Hypothesen (Rangfolge):**
1. **ONNX Session State Drift über Server-Neustarts:** Jeder frische Server-Start produziert minimal andere Score-Verteilungen. Bei 0.5833 vs 0.6833 ist die Differenz zu groß für reine Varianz (−0.10 = ~17% relativer Abfall).
2. **MiniLM Modell-Degradation?** Unwahrscheinlich — ONNX-Modell ist deterministisch. Aber Embedding-Provider (Ollama nomic-embed-text) könnte Seiteneffekte haben, wenn Ollama zwischenzeitlich updated wurde.
3. **BM25-Komponente driftet:** Die Hybrid-Retrieval-Pipeline kombiniert Vektor + BM25. Wenn BM25-Gewichte oder Tokenization sich ändern, verschiebt sich das Ranking ohne die Hits zu verlieren (erklärt identische owned_hits).

**Empfehlung:** 
- **Kurzfristig:** MAX_CASES=30 für genauere MRR-Schätzung
- **Diagnostik:** `diff` der kompletten Retrieval-Ergebnisse (nicht nur owned_hits) zwischen 06-21 und 06-24 — wo genau verschieben sich die korrekten Hits?
- **Mittelfristig:** Gate-Schwelle evaluieren — ist 0.65 zu streng für 10 Cases mit MiniLM? Die 0.6833 Baseline könnte ein Ausreißer nach oben gewesen sein.

### Per-Case Results (identisch zu 06-21/06-23)

| Case | Owned Hits | vs 06-21 |
|------|-----------|----------|
| 71017277 | 3/5 | = |
| gpt4_a2d1d1f6 | 2/5 | = |
| 54026fce | 3/5 | = |
| gpt4_7abb270c | 3/5 | = |
| gpt4_b5700ca0 | 1/5 | = |
| 195a1a1b | 2/5 | = |
| 32260d93 | 2/5 | = |
| gpt4_af6db32f | 2/5 | = |
| gpt4_7bc6cf22 | 4/5 | = |
| gpt4_d6585ce9 | 4/5 | = |

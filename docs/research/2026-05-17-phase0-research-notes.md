# KnowWhere v0.6 Phase 0 — Research Notes & Lean Decision Matrix

**Date:** 2026-05-17  
**Constraint:** Over-Engineering vermeiden. Nur was die Metrics (PersonaMem 20q, AMB, Temporal Queries) wirklich bewegt. Root-Cause zuerst. Clean Baseline + Delta pro Phase.

## Current State (Live Verification)
- Server: 2405 Nodes, healthy (`/health` OK)
- Hermes Plugin: Aktiv (`~/.hermes/plugins/knowwhere/__init__.py`)
  - Speichert bereits: `turn_index`, `session_id`, `observed_at`, `claim_scope`
  - `on_pre_compress` Hook existiert
- FractalNode: Hat `created_at` + `last_accessed` (DateTime<Utc>)
- Retrieval: `retrieve_fractal` + hybrid_retrieve nutzen diese Felder **nicht** für Ranking oder Evolution-Queries
- Embedding: `.env.native` = bge-m3; Code-Defaults mischen nomic. MODEL-EVALUATION empfiehlt nomic-embed-text v1.5

## Root Cause Analysis (Temporal Gap)
**Problem:** Konversationen sind isolierte Atome. Keine SEQUENCE/NEXT/BEFORE-AFTER Verknüpfung und kein zeitliches Ranking.

**Warum das das 70%-Plateau verursacht:**
- PersonaMem Queries brauchen Preference-Evolution ("Was hat sich geändert?").
- Aktuell: Retrieval holt semantisch ähnliche Claims ohne zeitliche Ordnung → Amalgamation (Issue 2 aus knowwhere-complete-repair).
- Hermes Plugin liefert turn_index, aber Server ignoriert es bei Scoring.

**Was bereits da ist (nicht neu bauen):**
- Metadata: turn_index, observed_at, created_at
- Plugin: Crash-safe Turn-Tracking + on_pre_compress

**Was fehlt (Root-Cause Fix):**
- Temporal Boost im Retrieval (wenn Scores nah beieinander liegen → recency + turn_proximity priorisieren)
- turn_range auf L1/L2 Nodes statt einzelnem turn_index
- Neue Golden Queries für Timeline + Preference-Evolution

## Lean Decision Matrix (Root-Cause First)

| Option | Impact on Metrics | Complexity | Risk | Recommendation |
|--------|-------------------|------------|------|----------------|
| **Temporal Metadata Boost** (turn_index + created_at in RRF) | Hoch (direkt auf Evolution Queries) | Niedrig | Niedrig | **Phase 1 Start** |
| Vollständiger Temporal Graph (NEXT/BEFORE Relations) | Mittel | Hoch | Mittel | Nur wenn Boost nicht reicht |
| Embedding Switch (nomic v1.5) | Hoch (bessere Base Quality für alles) | Niedrig | Niedrig | Frühe Phase 2 |
| Agentmemory UX Polish | Mittel (UX, nicht Core Metrics) | Mittel | Niedrig | Phase 3 |

**Lean Plan (angepasst an Constraints):**

**Phase 0 (jetzt)**
- Research Notes + aktualisierte Golden Queries (temporal)
- Baseline: Aktueller AMB + PersonaMem 20q mit 2405 Nodes + bge-m3

**Phase 1 (Temporal Layer — minimal)**
- Nur: Temporal Boost in `hybrid_retrieve` + `retrieve_fractal`
- turn_range auf Consolidation Nodes
- Verification: 10 neue temporale Golden Queries + Score-Delta

**Phase 2**
- Embedding Switch zu nomic-embed-text v1.5 + reembed_all
- Rerun Benchmarks

**Spätere Phasen** nur starten, wenn Phase 1 + 2 die Metrics nicht ausreichend bewegen.

## Nächster Schritt
1. Baseline messen (AMB + PersonaMem 20q)
2. Temporal Golden Queries definieren
3. Minimal Temporal Boost implementieren

**Vermeidung von Over-Engineering:**
- Kein neues Graph-Schema in Postgres zuerst.
- Kein komplexes Time-Series-Modell.
- Nur existierende Metadata nutzbar machen + messen.
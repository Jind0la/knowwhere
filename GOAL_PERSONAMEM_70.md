# GOAL: KnowWhere PersonaMem — 50% → 70%+ Accuracy

## Single Concrete Outcome
Integriere temporale Claim-Extraktion und Timeline-Retrieval in KnowWhere,
sodass PersonaMem/32k von aktuell 50.8% auf ≥70% Accuracy steigt.
Schreibe Tests für jeden Pipeline-Schritt. Liefere einen reproduzierbaren
Benchmark-Run mit messbarem Delta.

## Bounded Scope
NUR die KnowWhere-Ingestion+Retrieval-Pipeline. Keine Änderungen an:
- KnowWhere Server (Rust) — nur Benchmark-Connector (Python)
- Embedding-Modell (bleibt nomic-embed-text)
- LLM (bleibt Gemini Flash für Claims, Gemini Pro für Answers)
- Keine neuen API-Endpoints, keine DB-Migrationen

DREI präzise Änderungen:
1. `knowwhere.py`: Claims-Extraktion MIT turn_index + Struktur
2. `knowwhere.py`: Timeline-Context-Template für Retrieval
3. Benchmark-Konfiguration: claim_limit erhöhen, fallback deaktivieren

## Testable Success Criteria (selbst-verifizierbar)
1. `uv run omb run --dataset personamem --split 32k --memory knowwhere --query-limit 20 --name knowwhere-timeline-v1`
   → Accuracy ≥ 65% (aktuell 50% mit Claims, 50.8% mit Chunks)
2. Bei ≥65%: Full 589-Query Run → Accuracy ≥ 70%
3. Jeder Claim hat `turn_index` im Metadata (verifizierbar per curl /health + node inspection)
4. Retrieval-Context enthält "## Timeline" Header für temporale Queries
5. Kein Retrieval-Fehler mehr (0 queries mit leerem Context)
6. Ingestion-Zeit ≤ 5min für 32 Docs (aktuell ~3min mit 5 Docs)

## Constraints
- Keine Änderungen an `src/` (Rust-Code)
- `nomic-embed-text` als Embedder (274MB, M1-kompatibel)
- Gemini Flash für Claim-Extraktion (kostengünstig)
- Gemini Pro für Answer (OMB_ANSWER_MODEL=gemini-2.5-pro)
- Max 150 Zeilen neuer Code in knowwhere.py
- Existierender `/store_external` und `/retrieve_fractal` API-Vertrag unverändert

## Specs
- Accuracy: 50% → 70%+ (gemessen an PersonaMem/32k MCQ Letter-Match)
- Retrieval Time: ≤ 300ms avg (aktuell ~95ms mit Claims)
- Context Tokens: ≤ 2000 avg (aktuell ~300-400 chars, zu klein)
- Ingestion Time: ≤ 5min für 32 Docs (aktuell ~3min für 5 Docs)

## Outcome-Specific
Der Benchmark-Output `outputs/personamem/knowwhere-timeline-v1/rag/32k.json`
muss `accuracy >= 0.70` zeigen. Alle Zwischenschritte werden automatisiert
und das Skript läuft ohne manuelles Eingreifen durch.

# /goal Activate the Fractal Memory Hierarchy — Documents & Conversations as First-Class Citizens

**── CONTEXT ──**
· **Project:** KnowWhere — pointer-first fractal memory substrate for AI agents
· **Stack:** Rust/Axum, MemoryStore (JSON), Ollama (nomic-embed-text-v2-moe, 768d embeddings; llama3.2/qwen2.5:3b summarization), USearch + BM25 + RRF k=5
· **Current state:** Core Loop bewiesen. RRF k=5 (7.9× bessere Score-Separation). Multiplier neutralisiert (faires Scoring). 15.076 Nodes. Aber: Das System verhält sich wie eine flache Vector-DB, nicht wie ein hierarchisches Memory-Substrat. Der PRD verspricht L0 (Raw) → L1 (Summary) → L2 (Overview) mit Fractal Zoom — aber was tatsächlich existiert sind 14.979 flache Decision-Atome ohne echte Hierarchie. Document-Chunks und Conversation-Turns sind ingested, werden aber im Retrieval von semantisch dichteren Decision-Claims ausgescored (nicht wegen Scoring-Bias, sondern wegen Embedding-Dichte). Die Consolidation-Pipeline (L2→L1→L0) ist deaktiviert weil sie Cloud-API-Keys voraussetzt — obwohl LocalSummarizer (Ollama) implementiert ist. Fractal Zoom (`expand_fractal`) existiert im Code aber wurde nie mit echten hierarchischen Daten getestet.
· **Working dir:** `/Users/nimarfranklinmac/knowwhere`
· **Constraints:** Keine Cloud-API-Keys. Ollama = einzige Embedding- und Summarization-Quelle. Kimi K2.6 LOCKED als Answer-Model (aber nicht im Scope dieses Goals). Kein Neubau — bestehende Architektur aktivieren, nicht ersetzen.
· **Audience:** Nimar — will KnowWhere als das Memory OS sehen das der PRD verspricht: Dokumente und Konversationen fließen in eine durchzoom-bare Hierarchie, nichts geht verloren, alles hat Provenance.

**── WARUM DIESES GOAL ──**
Das vorherige Goal hat bewiesen: Der Core-Loop funktioniert. RRF k=60 war der einzige Blocker. Aber ein funktionierender Core-Loop macht KnowWhere noch nicht zum Memory OS. Der PRD verspricht drei Dinge die aktuell nicht eingelöst sind:

1. **Fractal Hierarchy:** L0 (atomar) → L1 (Summary) → L2 (Overview). Aktuell gibt es nur flache Nodes mit Tier-Tags — kein Zoom, keine Hierarchie, keine Verdichtung.
2. **Lossless:** Sessions bleiben als vollständige Einheiten erhalten. Aktuell werden sie in Ein-Satz-Claims atomisiert.
3. **Self-Hosted Consolidation:** L0→L1→L2 Verdichtung via LocalSummarizer (Ollama). Aktuell deaktiviert weil Cloud-Keys fehlen.

Dieses Goal aktiviert diese drei Versprechen. Kein Neubau. Keine neuen Features. Die Bausteine sind alle im Code — sie müssen nur verdrahtet und mit echten Daten getestet werden.

**── SUCCESS CRITERIA (ALL MUST BE TRUE) ──**
1. **Self-Hosted Consolidation läuft:** `POST /consolidation/force` produziert erfolgreiche L0→L1 Summarization via Ollama (LocalSummarizer) OHNE Cloud-API-Keys. Nachweis: `GET /dream/status` zeigt consolidation_jobs > 0 mit Status "completed".
2. **Fractal Hierarchy nachweisbar:** Eine Test-Session (8 Turns) wird ingested → Consolidation erstellt L1-Summaries → L0-Kinder sind via `parent_tier_id` mit L1-Eltern verkettet. Nachweis: `GET /retrieve/{l1_id}` zeigt `children_tier_ids` mit L0-IDs.
3. **Fractal Zoom funktioniert:** Eine Retrieval-Query auf L2-Ebene (`max_depth=2`) expandiert über `expand_fractal` zu L1-Kindern → L0-Enkeln. Nachweis: Response enthält Nodes auf mehreren Tiers, nicht nur flache Ergebnisse.
4. **Document Retrieval verbessert:** Gleiche 5 Document-Queries wie im Core-Loop-Proof. Precision@3 ≥ 0.50 (von 0.33). Dokument-Chunks sind via L1-Summaries besser auffindbar weil Summaries semantisch dichter sind.
5. **Conversation Retrieval verbessert:** Gleiche 5 Conversation-Queries. Precision@3 ≥ 0.50 (von 0.27). Session-Turns sind via L1-Summaries retrievable weil Summaries den Kern der Turns extrahieren.
6. **Alles dokumentiert:** `docs/CONSOLIDATION-REPORT.md` mit Messwerten, `ARCHITECTURE-ANALYSIS.md` ergänzt, README aktualisiert.

**── OPERATING RULES — NON-NEGOTIABLE ──**
1. PLAN FIRST. Output a numbered task list before writing any code.
2. WORK AUTONOMOUSLY. Don't ask clarifying Qs unless genuinely blocked.
3. SELF-VERIFY. After every step: run tests, inspect output, confirm it worked.
4. DEBUG YOURSELF. If it fails, diagnose + fix. Don't hand it back.
5. USE EVERY TOOL. MCPs · terminal · web · code exec · pull real data.
6. NO PLACEHOLDERS. No TODOs · no stubs · real components + real states.
7. PROGRESS LOG. Track completed · in-flight · decisions · blockers.
8. STAY ON GOAL. Discoveries off-spec? Note + keep moving.
9. IF BLOCKED. Log the wall · continue everything parallelizable.
10. CHECK SUCCESS BEFORE STOPPING. Re-read criteria · confirm each is met.

**── NICHT-ZIELE (explizit ausgeschlossen) ──**
- Neue Features bauen (MCP-Server, neue Endpoints, Dashboard)
- AMB-Benchmark rerun (das kommt nachdem die Hierarchie steht)
- Kimi K2.6 QA-Testen (separates Goal)
- Multi-Tier-Persistence via PostgreSQL (MemoryStore reicht)
- Trust-Tiers/Governance/Energy Decay reaktivieren
- Code refactoren "weil es schöner wäre"

**── QUALITY BAR ──**
· Consolidation: LocalSummarizer als PRIMÄRER Pfad (immer aktiv). VLM-Fallback-Chain als Opt-in (nur wenn Cloud-Keys gesetzt). Kein "wenn Key dann Cloud sonst gar nichts" mehr.
· Fractal Zoom: `expand_fractal` wird mit echten hierarchischen Daten getestet. Response zeigt Tiefe der Expansion.
· Summaries: L1-Summaries sind semantisch dichter als Raw-Turns → verbessern Retrieval-Präzision.
· Provenance: Jede L1-Summary verweist auf ihre L0-Quellen. Jeder Retrieval-Hit kann zur Original-Conversation/Dokument zurückverfolgt werden.

**── FINAL DELIVERABLE ──**
✅ `/consolidation/force` läuft ohne Cloud-Keys
✅ Fractal Hierarchy: L0↔L1↔L2 via `parent_tier_id`/`children_tier_ids` verkettet
✅ Fractal Zoom: `max_depth=2` expandiert korrekt
✅ Document P@3 ≥ 0.50
✅ Conversation P@3 ≥ 0.50
✅ `docs/CONSOLIDATION-REPORT.md` mit allen Messwerten
📝 Entscheidungen + Architektur-Änderungen dokumentiert
⚠️ Was funktioniert, was nicht, was als nächstes

---

*Voraussetzung für dieses Goal: `docs/ARCHITECTURE-ANALYSIS.md` und `docs/SIGNAL-TRACE.md` aus dem Core-Loop-Proof gelesen haben. Server läuft mit RRF k=5 und neutralen Multipliers (bereits committed).*

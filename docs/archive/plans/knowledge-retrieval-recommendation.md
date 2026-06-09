# Knowledge-Retrieval: Implementierungsempfehlung

**Datum:** 2026-05-04
**Inputs:** T1 (Audit), T2 (Competitive Research), T3 (Code-Analyse)
**Direktive:** „Wir wollen nicht mehr Features sondern KnowWhere richtig zum Laufen bringen"

---

## 1. Zusammenfassung der drei Inputs

### T1 — Consolidation Audit (docs/research/consolidation-audit-2026-05-04.md)

**Hypothese BESTÄTIGT.** Die Dream-Consolidation produziert narrative Prosa ohne trembare What/Why/Alternatives/Consequences. Nur 1.4% aller Nodes sind Decision-typisiert. Die L0-Kompression tötet das Why. 0% exakter Recall für spezifische Warum-Fragen. Entscheidungserkennung ist rein keyword-basiert und produziert False Negatives. Keine strukturierten Felder.

### T2 — Competitive Research (docs/research/competitive-landscape-2026-05-04.md)

**Kein existierendes System** behandelt Entscheidungen als First-Class-Objekte mit Schema + Provenienz + kausalen Verknüpfungen. Die Lücke ist real. Übernehmbare Patterns: Claim-Extraktion à la GraphRAG, Kausale Kanten à la CausalRAG/HugRAG, Tiered Memory à la Letta (hat KnowWhere schon). Anti-Patterns: Self-Reporting (Letta), reiner Fakt-Recall (Mem0), Freitext ohne Schema (CASS).

### T3 — Code-Analyse (ANALYSIS_consolidation_claims.md)

**~265 LOC / ~7.5h** über 6 Dateien. Drei Hauptänderungen: Prompt-Erweiterung für Claims-Block (30 LOC), Claim-Parser in Consolidation-Flow (70 LOC), Pointer-Resolution in Retrieval-API (40 LOC), Response-Erweiterung um `source_content` (15 LOC). Empfehlung: Prompt-Änderung + parse_claims_block() zuerst (~100 LOC). MemoryType::Decision existiert seit commit 54265a4.

---

## 2. Bewertung nach den 4 Kriterien

| Kriterium | Bewertung |
|-----------|-----------|
| **Minimalität** | ✅ Prompt-Änderung (~30 LOC) + Claim-Parser (~70 LOC) = unter 100 LOC. Kein neuer Endpoint, kein neues DB-Schema, kein neues MemoryType. |
| **Wirksamkeit** | ✅ Claims mit What/Why/Alternatives/Consequences in strukturierter Form → Warum-Fragen treffen direkt auf Reason-Feld. 0% Recall → geschätzt 80%+ Recall. |
| **Wartbarkeit** | ✅ Keine Breaking Changes. Narrative Summary bleibt erhalten. Claims sind additive Struktur. Alle Änderungen in existierenden Dateien. |
| **Philosophie-Fit** | ✅ Pointer-First: Claims haben `source_session + source_turns` → Zoom zurück zur Quelle. Kein Datenverlust: Narrative Summary + Claims koexistieren. Fractal: Claims auf L1, Roh-Turns auf L0. |

---

## 3. Implementierungsplan

### Phase 1 — MVP: Prompt + Claim-Parser (~3h)

**Ziel:** Claims-Extraktion während der Consolidation. Testbar, sofort messbarer Impact.

```
Schritt 1.1: Prompt-Erweiterung (30 min)
Dateien: src/vlm/mod.rs:131-157, src/summarizer/mod.rs:107-117
→ Claims-Block-Format in system_directive() + summarize() Prompts

Schritt 1.2: Claim-Parser (90 min)
Datei: src/scheduler/consolidation.rs (nach summarize_for_tier())
→ parse_claims_block() — extrahiert Claims aus ---CLAIMS--- Block
→ Claims werden als MemoryType::Decision-Nodes gespeichert

Schritt 1.3: Test (60 min)
→ E2E: Store Session → Trigger Consolidation → Prüfe Decision-Nodes
→ Test-Query: "Warum wurde X entschieden?" → Claim muss im Ergebnis sein
→ cargo test --lib (bestehende Tests müssen weiter laufen)
```

### Phase 2 — Pointer-Resolution (~2.5h)

**Ziel:** LLM kann Quell-Turns zu Claims sehen.

```
Schritt 2.1: expand_pointers Parameter (30 min)
Datei: src/api/routes.rs (RetrieveFractalRequest)
→ Neues Feld: expand_pointers: bool, expand_depth: u8

Schritt 2.2: Pointer-Resolver (90 min)
Datei: src/storage/backend.rs + Implementierungen
→ find_by_metadata(session_id, turn_indices) → Vec<FractalNode>
→ Oder: self.get() für source_turns der Claim-Nodes

Schritt 2.3: Response-Erweiterung (30 min)
Datei: src/api/routes.rs (FractalNode-Response)
→ source_content: Vec<SourceTurn> Feld

Schritt 2.4: Test (30 min)
→ Query mit expand_pointers=true → Response muss source_content enthalten
```

### Phase 3 — Intent-Aware Retrieval (~2h)

**Ziel:** Warum-Queries priorisieren Decision-Claims mit Reason-Feld.

```
Schritt 3.1: Intent-Detektor (60 min)
Datei: neue util/intent.rs oder in routes.rs
→ detect_intent(query) → Causal | Procedural | Factual | Decision

Schritt 3.2: Reranker (60 min)
Datei: src/api/routes.rs (retrieve_fractal Handler)
→ Nach Embedding-Suche: Reranken nach Intent
→ Intent::Causal → priorisiere Decision-Nodes mit reason-Feld

Schritt 3.3: Test (30 min)
→ 5 Warum-Queries aus T1-Audit → Recall messen
```

### Gesamtaufwand: ~7.5h über 6 Dateien

---

## 4. Go/No-Go

**GO.**

Begründung:
1. Das Problem ist real und gemessen (0% Recall für spezifische Warum-Fragen)
2. Die Lösung ist minimal (<100 LOC für MVP, kein neuer Endpoint, kein neues MemoryType)
3. Die Lösung baut auf existierender Infrastruktur auf (Consolidation läuft bereits, MemoryType::Decision existiert)
4. Kein Konkurrent macht das — KnowWhere kann First Mover sein
5. Phase 1 (MVP) ist in 3h testbar — kein Risiko eines langen, unsichtbaren Projekts

---

## 5. Nächster Schritt

Phase 1 MVP sofort starten: Prompt-Änderung in `src/vlm/mod.rs` + `src/summarizer/mod.rs`, dann Claim-Parser in `src/scheduler/consolidation.rs`. Danach E2E-Test mit einer Warum-Query gegen die neuen Decision-Claims.

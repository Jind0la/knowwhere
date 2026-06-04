# Reduce-to-Core Phase 2: Saubere Core/Policy-Trennung im Retrieval-Scoring

**Branch:** `feature/reduce-to-core` (bestehend, Quick-Fix-Stand)  
**Ziel-Branch:** bleibt `feature/reduce-to-core` oder `reduce-to-core-phase2`  
**Status vor Start:** 305 Unit + 41 Integration Tests grün (Quick-Fix: nur tier/mtype/explicit neutralisiert, Source + Intent + Temporal + trust_tier-Hard-Rules + Backend-Tiebreaker noch aktiv)  
**Definition of Done:** siehe unten

---

## Was "Echtes Reduce-to-Core" bedeutet (Zielzustand)

**Core (immer aktiv, auch bei FullFidelity):**
- Cosine-Similarity (bzw. fused retrieval signal aus RRF/WeightedSum/BM25)
- Ebbinghaus-Vergessenskurve (physikalisches Faktum: R(m,t) basierend auf r_m + n_m)

**Policy (nur bei UserFacing / AgentDebug, explizit deaktiviert bei FullFidelity):**
- tier_multiplier (primary=1.3, reference=1.1, derived=0.9, volatile=0.7)
- memory_type_multiplier (Decision=1.5, Preference=1.2, Procedural=1.15, Semantic=1.05, Rest=1.0)
- source_multiplier (real=1.0, synthetic=0.85, derived=0.70, unknown=0.95) — bleibt Feature!
- explicit_weight (explizites trust_weight oder node.weight)
- intent_metadata_multiplier (current_state, decision_why, procedure, preference etc.)
- governance_score_multiplier
- temporal_weight + recency_boost (die hybriden Recency-Boosts)

**Strukturelle Invarianten (garantiert):**
- Eine einzige Quelle der Wahrheit: `ScoringEngine` (in `src/retrieval/scoring.rs`)
- `FullFidelity.score == cosine × ebbinghaus` (für pure Vector-Pfade; für Hybrid = fused × ebbinghaus)
- `trust_tier()` respektiert explizite `metadata["trust_tier"]` — Hard-Rules (Decision→primary, Consolidation→derived etc.) wirken **nur** als Fallback-Defaults bei fehlendem Metadata
- Keine backend-abhängigen Tiebreaker (weder in_memory tier_ord noch implizite pg-Sort)
- Source-Weights und Intent-Boosts bleiben vollständig erhalten (als Policy unter !FullFidelity)
- Contract-Tests erzwingen die Invariante dauerhaft

**Constraints (nicht verhandelbar):**
- Kein Rewrite von HybridQuery, FractalNode, usearch, RRF-Fusion, postgres/in_memory storage
- Nach jedem Commit/Step: alle Tests grün (`cargo test`)
- Keine neuen Public-API-Brüche (ScoreDebug/ScoredNode-Serialisierung bleibt kompatibel)

---

## 1. Modul-Struktur (Ziel)

```
src/
├── retrieval/
│   ├── mod.rs                 # + pub mod scoring;
│   ├── scoring.rs             # NEU: ScoringEngine + Core/Policy-Logik
│   ├── source_weighting.rs    # bleibt (nur Docs + Tests anpassen)
│   ├── hybrid.rs              # unverändert (liefert "base signal")
│   └── cross_encoder.rs       # unverändert (post-retrieval, feature-gated)
├── storage/
│   ├── backend.rs             # RetrievalProfile + HybridQuery + ScoredNode/ScoreDebug bleiben;
│   │                          # score_* delegieren an Engine (altes Verhalten für !FF)
│   ├── in_memory.rs           # Tiebreaker neutralisieren, temporal/recency nur bei !FF
│   └── postgres_store.rs      # temporal/recency nur bei !FF
├── memory/
│   └── fractal_node.rs        # trust_tier() → explicit-first + Fallbacks
├── api/
│   └── retrieve.rs            # intent/governance/temporal nur bei !FF; FF-Pfad bypass't Policy
└── ...
```

**Keine Änderungen an:**
- `src/api/store.rs` (or_insert-Logik bleibt — ist korrekt)
- `src/retrieval/hybrid.rs`
- Datenmodellen (FractalNode Felder, HybridQuery Signatur)
- usearch / pgvector / BM25-Pfaden
- Reranker (wird als "bessere Ähnlichkeit", nicht als Trust-Policy gesehen)

---

## 2. ScoringEngine API-Design

```rust
// src/retrieval/scoring.rs
use crate::memory::FractalNode;
use crate::retrieval::source_weighting::SourceTypeWeights;
use crate::storage::backend::{RetrievalProfile, ScoreDebug, ScoredNode};

#[derive(Debug, Clone, Default)]
pub struct ScoringContext {
    pub source_type_weights: Option<SourceTypeWeights>,
    pub temporal_weight: Option<f32>, // wird nur bei !FF angewendet (Post-Processing)
}

pub struct ScoringEngine;

impl ScoringEngine {
    /// Core-Faktor (immer): nur Ebbinghaus.
    pub fn core_multiplier(node: &FractalNode) -> f32 {
        node.ebbinghaus_decay(chrono::Utc::now()) as f32
    }

    /// Reiner Core-Score (für Contract-Tests).
    /// Garantie: FullFidelity-Pfad produziert exakt diesen Wert (bei purem Cosine-Signal).
    pub fn core_score(signal: f32, node: &FractalNode) -> f32 {
        signal * Self::core_multiplier(node)
    }

    /// Effektiver Multiplier für ein Node unter gegebenem Profile.
    /// FullFidelity → exakt core_multiplier (Ebbinghaus).
    /// UserFacing/AgentDebug → tier * explicit * mtype * source * ebbinghaus.
    pub fn multiplier(
        profile: RetrievalProfile,
        node: &FractalNode,
        weights: Option<SourceTypeWeights>,
    ) -> f32 {
        let ebbi = Self::core_multiplier(node);
        if matches!(profile, RetrievalProfile::FullFidelity) {
            return ebbi;
        }
        let w = weights.unwrap_or_default();
        let src = crate::retrieval::source_weighting::source_multiplier(node, &w);
        let tier = Self::tier_multiplier(node.trust_tier());
        let mtype = Self::memory_type_multiplier(node);
        let expl = Self::explicit_weight(node);
        tier * expl * mtype * src * ebbi
    }

    pub fn score_node(
        profile: RetrievalProfile,
        base_score: f32,
        node: FractalNode,
        weights: Option<SourceTypeWeights>,
    ) -> ScoredNode {
        let debug = Self::score_debug(profile, base_score, &node, weights);
        ScoredNode {
            id: node.id,
            score: debug.final_score(),
            distribution_scores: None,
            debug: Some(debug),
            node,
        }
    }

    pub fn score_debug(
        profile: RetrievalProfile,
        base_score: f32,
        node: &FractalNode,
        weights: Option<SourceTypeWeights>,
    ) -> ScoreDebug {
        let src_type = crate::retrieval::source_weighting::detect_source_type(node).to_string();
        let w = weights.unwrap_or_default();
        let src_mult = crate::retrieval::source_weighting::source_multiplier(node, &w);
        let eff_mult = Self::multiplier(profile, node, Some(w));
        ScoreDebug {
            profile,
            trust_tier: node.trust_tier().to_string(),
            base_score,
            multiplier: eff_mult,
            source_type: Some(format!("{src_type} ({src_mult:.2}x)")),
            source_weight_applied: Some(src_mult),
            original_source: Some(src_type),
            ebbinghaus_factor: Some(Self::core_multiplier(node)),
            // recency/temporal/explanation werden von Callern (temporal apply) nachgetragen
            .. /* rest None / Default */
        }
    }

    // Extrahierte Policy-Funktionen (nur für !FF aufgerufen)
    fn tier_multiplier(t: &str) -> f32 { /* ... identisch zu altem Stand ... */ }
    fn memory_type_multiplier(node: &FractalNode) -> f32 { /* ... */ }
    fn explicit_weight(node: &FractalNode) -> f32 { /* ... ohne FF-Check ... */ }

    // Optional: zentrale Post-Processing-Helfer (für Konsistenz)
    pub fn apply_temporal_if_policy(results: &mut [ScoredNode], w: f32, profile: RetrievalProfile) {
        if matches!(profile, RetrievalProfile::FullFidelity) { return; }
        // ... bestehende apply_hybrid_temporal_scoring Logik hier oder Delegation
    }
}
```

**Rückwärtskompatibilität während Migration:**
```rust
// in backend.rs
impl RetrievalProfile {
    pub fn score_multiplier(self, node: &FractalNode, weights: Option<...>) -> f32 {
        ScoringEngine::multiplier(self, node, weights)
    }
    pub fn score_node(...) -> ScoredNode { ScoringEngine::score_node(...) }
    pub fn score_debug(...) -> ScoreDebug { ScoringEngine::score_debug(...) }
    // alte private fns (tier_*, memory_type_*, explicit_*) werden gelöscht oder delegiert
}
```

**Contract-Invarianten (per Test erzwungen):**
- `FullFidelity.score_node(cos, node, w).score == cos * ebbi(node)`
- `FullFidelity.score_node(...).debug.multiplier == ebbi(node)`
- `source_weight_applied` wird auch bei FF noch gefüllt (Observability), fließt aber nicht in score/multiplier ein.

---

## 3. Konkrete Änderungen pro Datei

### src/memory/fractal_node.rs
- `trust_tier(&self)` (L302–344) komplett umschreiben:
  1. Zuerst `if let Some(v) = metadata_text(TRUST_TIER_KEY)` → validiere + return (explizit gewinnt **immer**)
  2. Danach die bisherigen Hard-Rules **nur als Fallback** (Decision → primary, internal/consolidation/summary → derived, import rules, source=Conversation → primary, default reference)
- Unit-Tests hinzufügen (im selben File oder `src/memory/tests.rs`):
  - `trust_tier_explicit_overrides_decision()`
  - `trust_tier_explicit_overrides_consolidation()`
  - `trust_tier_absent_uses_fallbacks()` (stellt sicher, dass Default-Verhalten gleich bleibt)
- Keine Änderung an `explicit_trust_weight`, `ebbinghaus_decay`, `new_*` Konstruktoren.

### src/retrieval/mod.rs
- `pub mod scoring;` hinzufügen (nach source_weighting).

### src/retrieval/scoring.rs (NEU)
- Komplette Datei mit `ScoringEngine`, `ScoringContext`, `core_*`, `multiplier`, `score_node`, `score_debug` + den drei extrahierten Multiplier-Funktionen.
- `#[cfg(test)] mod tests { ... }` mit:
  - Contract-Tests für pure Core (mit ebbi=1.0 und ebbi<1.0 via manuell gesetztem `r_m`)
  - Tests, dass source/tier/mtype/explicit bei FF ignoriert werden
  - Tests, dass die gleichen Nodes bei UserFacing die Policy-Multiplier enthalten
  - Tests, dass `score_debug` weiterhin `source_*` Felder liefert (auch bei FF)

### src/storage/backend.rs
- `RetrievalProfile` Enum + `fetch_k`, `allows`, `as_str` bleiben unverändert.
- `score_multiplier`, `score_debug`, `score_node` werden zu 1-Zeiler-Delegates an `ScoringEngine`.
- `explicit_weight`, `tier_multiplier`, `memory_type_multiplier` werden gelöscht (Logik nur noch in scoring.rs).
- Doc-Comment bei `score_multiplier` (L108) und der Quick-Fix-Kommentar (L114) werden auf "Phase 2: nur noch Ebbinghaus als Core" aktualisiert.
- `ScoreDebug` / `ScoredNode` / `HybridQuery` bleiben exakt gleich (kein SerDe-Bruch).

### src/storage/in_memory.rs
- Sort in `hybrid_retrieve` (Trait-Impl, ca. L290):
  ```rust
  weighted.sort_by(|a, b| {
      b.score.partial_cmp(&a.score).unwrap_or(Equal)
          .then_with(|| a.id.cmp(&b.id))   // neutral, backend-unabhängig, determ.
  });
  ```
  Kommentar L287–289 löschen/ersetzen (kein "trust hierarchy" mehr).
- Recency-Boost-Handling (legacy): Boost-Applies aus dem Low-Level `pub async fn hybrid_retrieve` (L1573, L1636) entfernen. Die Low-Level-Fn liefert **immer** pure Signale zurück.
- Im Trait-`hybrid_retrieve` (L247 ff.):
  - Nach `let results = self.hybrid_retrieve(..., None, ...)` (recency jetzt hier):
    ```rust
    if let Some(b) = query.recency_boost {
        if query.profile != RetrievalProfile::FullFidelity {
            Self::apply_temporal_boost(&mut raw_results, b);
        }
    }
    ```
  - `apply_temporal_to_scored_nodes` (L307) ebenfalls mit `if query.profile != FullFidelity` wrappen.
- Die `apply_*` Helfer-Fns selbst bleiben (werden von Tests direkt aufgerufen).

### src/storage/postgres_store.rs
- Alle vier Apply-Stellen für `recency_boost` (apply_temporal_boost_scored) und die eine für `temporal_weight` (apply_hybrid_temporal_scoring) mit Guard wrappen:
  ```rust
  if let Some(b) = query.recency_boost {
      if !matches!(query.profile, RetrievalProfile::FullFidelity) {
          let _ = apply_temporal_boost_scored(&mut scored_nodes, b);
      }
  }
  if let Some(w) = query.temporal_weight {
      if !matches!(query.profile, RetrievalProfile::FullFidelity) {
          apply_hybrid_temporal_scoring(&mut scored_nodes, w);
      }
  }
  ```
- Die Apply-Fns selbst können später optional nach `scoring.rs` verschoben werden (nicht zwingend in Phase 2).

### src/api/retrieve.rs
- `finalize_retrieval_storage` (Intent + Dedupe + MMR) wird für `FullFidelity` umgangen:
  ```rust
  let results = if req.retrieval_profile == RetrievalProfile::FullFidelity {
      results // pure core scores aus Storage; keine Intent-Multiplikation, keine MMR
  } else {
      finalize_retrieval_storage(results, query_intent, &qv, req.top_k, allow_meta)
  };
  ```
- Im Governance-Pfad (ca. L1439):
  ```rust
  if req.retrieval_profile != RetrievalProfile::FullFidelity {
      for (entry, _, _) in &mut governed {
          entry.score *= intent_metadata_multiplier(...);
      }
  }
  ```
- `finalize_governed_retrieval` wird analog umgangen (oder der Sortierer darin bekommt ein "apply_gov" Flag). Für FF: reine Score-Sortierung aus Storage (kein `* governance_score_multiplier`).
- MMR und Evidence-Dedupe werden für FF bewusst **nicht** ausgeführt (Raw-Retrieval-Signal).
- Direkter `profile.score_node(...)` Aufruf (L1071) profitiert automatisch durch Delegate.
- Kommentare zu "full scoring pipeline" anpassen.

### src/retrieval/source_weighting.rs
- Modul-Doc (L8–11) aktualisieren: "source_multiplier is Policy. Called from ScoringEngine only for !FullFidelity. Debug fields are still populated for observability."
- Test-Überarbeitung (größter Test-Edit):
  - Alle Tests, die `FullFidelity.score_multiplier` / `score_node` benutzen, um zu prüfen, **dass Source angewendet wird**, auf `UserFacing` (oder `AgentDebug`) umstellen.
  - Test `test_score_node_all_profiles_apply_source_weights` (L1164) wird gelöscht oder in "all **policy** profiles" umbenannt + Assertion angepasst.
  - Tests, die nur `source_multiplier(...)` direkt aufrufen, bleiben unverändert.
  - Tests, die `score_debug` unter FF auf Source-Felder prüfen, bleiben (Info-Felder werden weiterhin geliefert).
  - Neue Contract-Tests am Ende (siehe Abschnitt 2).
- Keine Logik-Änderung an `source_multiplier` / `detect_source_type` / `SourceTypeWeights`.

### tests/integration.rs
- Test `retrieve_fractal_neutralized_trust_tiers` (ca. L440–473):
  - Explizit `full_fidelity` Profil verwenden.
  - Statt `assert_eq!` auf Label-Reihenfolge (die vom alten tier-Tiebreaker kam): prüfen, dass alle drei Scores (fast) gleich sind (`abs(diff) < 1e-6`).
  - Labels können als Set oder sortiert verglichen werden.
- Test `full_fidelity_profile_surfaces...` (L515): Kommentar anpassen ("multiplier == ebbinghaus (fresh node → 1.0); source + tier + mtype neutralisiert").
- Optional: einen HTTP-Level Contract-Test für pure Cosine × Ebbi mit altem Node hinzufügen.

### docs/ (nachgelagert)
- `docs/ARCHITECTURE.md`: Abschnitt zu retrieval/scoring aktualisieren (neue Datei `scoring.rs`, Core vs Policy explizit nennen).
- `CHANGELOG.md`: Eintrag unter "Unreleased".
- Dieser Plan selbst bleibt als Referenz.

---

## 4. Migrations-Pfad (exakte Reihenfolge – Tests nach jedem Step grün)

**Voraussetzung (einmalig):**
```bash
git checkout feature/reduce-to-core
cargo test
# 305+41 grün bestätigen
```

**Task 1 – Trust-Tier respektiert explizites Metadata (niedriges Risiko, Voraussetzung)**
1. `src/memory/fractal_node.rs` editieren (metadata-first).
2. 3 Unit-Tests für Override + Fallback hinzufügen.
3. `cargo test trust_tier --lib` (oder full).
4. Bei Bruch: nur betroffene Konstruktionen in Tests anpassen (erwartet: keine).
5. Commit: `refactor(arch): trust_tier() prefers explicit metadata; hard-rules are fallbacks only`

**Task 2 – ScoringEngine einführen (neue Datei, noch nicht verdrahtet)**
1. `src/retrieval/scoring.rs` anlegen (komplette Engine + Contract-Tests).
2. `src/retrieval/mod.rs` um `pub mod scoring;` erweitern.
3. `cargo check && cargo test scoring --lib` (die neuen Engine-Tests müssen grün sein, obwohl noch nicht im Pfad).
4. Commit: `feat(arch): add ScoringEngine with strict FullFidelity = core only`

**Task 3 – Engine verdrahten + alle betroffenen Tests auf neuen Contract umstellen (größter Step)**
1. `src/storage/backend.rs`: score_*-Methoden auf Engine umstellen (Source jetzt auch bei FF neutralisiert).
2. **Gleichzeitig** `src/retrieval/source_weighting.rs`:
   - Alle "policy wirkt unter FF" Tests umschreiben (Profile → UserFacing).
   - "all profiles apply source" Test löschen/umbenennen.
   - Neue FF-ignoriert-Source Tests + Contract-Tests aktivieren.
3. `tests/integration.rs`: Neutralized-Tier-Test auf Score-Gleichheit umstellen.
4. `cargo test` (sollte jetzt durchlaufen).
5. Alle verbleibenden Breaks fixen (suche nach weiteren direkten FullFidelity.score_node Aufrufen in Tests).
6. Commit: `feat(arch): wire ScoringEngine; FullFidelity now strictly core (source neutralized); tests updated`

**Task 4 – Backend-abhängige Tiebreaker entfernen**
1. `src/storage/in_memory.rs`: tier_ord-Sort durch `id`-basierten neutralen Tiebreaker ersetzen.
2. Kommentare anpassen.
3. `cargo test` (der angepasste Integration-Test aus Task 3 muss noch grün sein; Reihenfolge-Assertionen sind bereits entfernt).
4. Commit: `refactor(arch): remove backend-dependent trust-tier tiebreaker; use stable id tiebreaker`

**Task 5 – Temporal/Recency als Policy behandeln (gating)**
1. `src/storage/in_memory.rs`:
   - Boost-Applies aus Low-Level-Fn entfernen.
   - Conditional Apply im Trait-Impl (nach Raw-Results).
2. `src/storage/postgres_store.rs`: alle recency/temporal Applies mit `if profile != FullFidelity` wrappen.
3. `cargo test` (inkl. der temporal_scoring_tests am Ende von in_memory.rs).
4. Commit: `arch: temporal_weight and recency_boost are policy; ignored under FullFidelity`

**Task 6 – API-Layer Policy (Intent + Governance) neutralisieren**
1. `src/api/retrieve.rs`:
   - finalize_retrieval_storage für FF umgehen (reine Storage-Ergebnisse).
   - intent *= und gov_mult in gov-Pfad für FF überspringen.
   - finalize_governed für FF umgehen oder auf reine Score-Sort reduzieren.
2. MMR/Dedupe für FF bewusst weglassen (Raw-Signal).
3. `cargo test`
4. Commit: `arch: FullFidelity bypasses api-level policy (intent, governance, mmr)`

**Task 7 – Contract-Tests + End-to-End-Verifikation**
1. Sicherstellen, dass alle neuen Contract-Tests aus scoring.rs im normalen `cargo test` laufen.
2. Optional einen zusätzlichen Test in `tests/integration.rs` oder `tests/retrieval_quality.rs`, der über die HTTP-API einen reinen Vector-Query mit FullFidelity macht und Cosine × Ebbi prüft (mit Node, der r_m in der Vergangenheit hat).
3. `cargo test`
4. Manuell (falls Server läuft): curl mit `retrieval_profile: full-fidelity` + `include_debug: true` und prüfen, dass multiplier ≈ ebbinghaus_factor und source_weight_applied zwar da ist, aber nicht multipliziert wurde.
5. Commit: `test(arch): add/verify FullFidelity core contract tests end-to-end`

**Task 8 – Aufräumen, Docs, CI**
1. Tote Code-Pfade / veraltete Kommentare entfernen.
2. `docs/ARCHITECTURE.md` aktualisieren (Core/Policy, neue Datei scoring.rs).
3. `CHANGELOG.md` Eintrag.
4. `cargo clippy -- -D warnings` (oder was das Projekt verwendet) + `cargo test`.
5. Commit: `chore(arch): Reduce-to-Core Phase 2 – docs + cleanup`

**Task 9 – Finale Verifikation**
```bash
cargo test 2>&1 | tail -20
# 305+41 grün
git diff --stat
# grob 1 new + 7-8 changed files, ~400 LOC delta
```

---

## 5. Risiken und Edge Cases

| Risiko | Wahrscheinlichkeit | Impact | Mitigation |
|--------|--------------------|--------|------------|
| Tests erwarten alte Tier-Reihenfolge bei gleichen Scores unter FF | hoch (1 Test schon bekannt) | mittel | Task 1+3: Test explizit auf Score-Gleichheit umschreiben, nicht auf Rangfolge |
| Float-Precision bei Contract-Tests (cosine × ebbi) | mittel | niedrig | Immer `abs(a-b) < 1e-6` oder `relative_eq` |
| Nodes in Unit-Tests ohne trust_tier-Metadata + Decision | niedrig | mittel | Fallback-Logik im neuen trust_tier() stellt altes Default-Verhalten sicher |
| Altes Prod-Daten mit explizitem abweichendem trust_tier auf Decision (wurde bisher ignoriert) | niedrig | mittel | Gewolltes Verhalten (explizit gewinnt jetzt); UserFacing-Ranking kann sich für solche Nodes ändern |
| Code ruft HybridQuery mit temporal_weight + FullFidelity und erwartet Boost | niedrig | mittel | Per Spec ist temporal Policy → Ignorieren ist korrekt; Doc in HybridQuery ergänzen |
| Reranker läuft auch bei FF | niedrig | niedrig | Reranker ist feature-gated + "bessere Similarity", nicht Trust-Policy; nicht Teil dieses Scopes |
| MMR/Dedupe bei FF gewünscht? | mittel | niedrig | Per "NUR Core" bewusst weglassen; falls später gebraucht, eigener Flag |
| ScoreDebug-Serialisierung ändert sich | sehr niedrig | hoch | Wir ändern keine Felder, nur Werte von `multiplier` und `ebbinghaus_factor` (letzteres war schon da) |
| Performance-Regress durch eine Indirektion | sehr niedrig | niedrig | Inlining + trivialer Code |

---

## 6. Aufwandsschätzung

- **Neue Dateien:** 1 (`src/retrieval/scoring.rs` ≈ 180–220 LOC inkl. Docs + Tests)
- **Geänderte Dateien:** 7–8 (backend.rs, in_memory.rs, postgres_store.rs, api/retrieve.rs, fractal_node.rs, source_weighting.rs, integration.rs, mod.rs, optional ARCHITECTURE.md)
- **Gesamt-Delta (geschätzt):** +350 / –120 LOC (viel Test-Umschreiben, wenig neue Logik)
- **Tasks:** 9 (jeder Task ist bewusst klein und testbar)
- **Zeit (realistisch bei disziplinierter Umsetzung):** 6–10 Stunden reine Coding + Test-Zyklen (verteilt auf mehrere Sessions)
- **Test-Sicherheit:** Sehr hoch – source_weighting.rs allein hat >50 scoring-spezifische Tests; Integration + retrieval_quality decken End-to-End ab.

---

## Umsetzungsprinzipien (für den, der den Plan ausführt)

1. **Immer grün nach jedem Commit.** Kein "wir fixen das später".
2. **Keine Gold-Plating.** Keine neuen Felder in ScoreDebug, keine neuen Query-Flags, keine Refactorings außerhalb des Scopes.
3. **"Wir machen X, dann Y"** – nicht "man könnte".
4. **Contract-Tests sind heilig.** Wenn ein Contract-Test nach Task 7 rot wird, ist die Implementierung falsch.
5. **Falls ein Step > 1 Stunde hängt:** Abbrechen, diesen Plan updaten, AskUserQuestion stellen.

---

**Nächster Schritt nach Plan-Review:** Task 1 starten (trust_tier). Der Plan ist damit abgeschlossen und umsetzbar.

*Erstellt auf Basis der zweifachen vorherigen Analyse + Quick-Fix auf `feature/reduce-to-core` (Stand 2026-05).*

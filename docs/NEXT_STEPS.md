# Next Steps — KnowWhere v0.5.0 → v1.0.0

> Stand: 2026-05-05, nach Hermes Retrieval Hardening, Decision-Scoring-Fix und MemoryType::parse-Reparatur.
> Aktueller Fokus: Hermes bekommt sicheren, belegbaren, aktuellen Memory-Kontext statt bloß viele Treffer.

---

## Erledigt seit dem letzten Stand

- **Decision Parsing & Scoring:** `memory_type: "decision"` wird korrekt geparst; Decision-Nodes ranken als PRIMARY plus Type-Boost.
- **Hermes Retrieval Hardening:** `/retrieve_fractal` hat strikte Typfilter, keine Default-`<knowwhere_memory>`-Injection mehr und keine Meta/Reflect-Leakage im Hermes-Plugin.
- **Hermes Eval:** `scripts/eval_hermes_retrieval.py` misst Top-1 non-meta, Decision-Purity, Provenance Coverage, Repeated Top-1, Stale-Conflict Rate und Latenz.
- **Intent-Aware Retrieval:** `query_intent` erlaubt erste Routing-Hinweise für `current_state`, `decision_why`, `procedure`, `preference`, `debug`, `historical`.
- **Provenance-Konvention:** Hermes- und Consolidation-Pfade schreiben bessere Metadata (`observed_at`, `claim_scope`, `source_node_ids`, `source_session_ids`, `derived_from`, `decision_what`, `decision_why`).

---

## 1. ⚡ JETZT: Retrieval-Diversität und Provenance Coverage verbessern

### Problem

Die API- und Plugin-Verträge sind jetzt sauber, aber der bestehende Datenbestand ist noch nicht gleichmäßig hochwertig:

- `provenance_coverage` ist noch nicht nahe genug an 1.0, weil alte Nodes keine vollständigen `source_*`-Metadaten haben.
- `repeated_top1_rate` ist noch zu hoch: einzelne generische Decision-Nodes gewinnen zu viele unterschiedliche Query-Typen.
- Current-State-Observation funktioniert für neue Daten, aber alte historische Zustände sind noch nicht systematisch scoped/superseded.

### Fix (geschätzt 1 Tag)

1. **Backfill/repair provenance:** Admin- oder Script-Pfad, der vorhandene Hermes/Decision-Nodes mit ableitbarer Provenance ergänzt.
2. **Intent-Ranking verfeinern:** Für `open_recall` und `procedure` weniger aggressive Decision-Gewichtung; für `current_state` aktuelle Semantic/Diagnostic-Evidence bevorzugen.
3. **Golden Queries erweitern:** `scripts/eval_hermes_retrieval.py` mit echten erwarteten Trefferklassen/IDs anreichern.

**Begründung:** Der gefährliche Meta/Filter-Fehler ist behoben. Jetzt entscheidet Datenqualität darüber, ob Hermes wirklich bessere Antworten gibt.

---

## 2. 🔜 Postgres-Fractal-Expansion nachziehen

### Problem

`MemoryStore` kann über `expand_fractal` Kinder/Summary-Beziehungen nachladen. `PostgresStore` fällt aktuell weitgehend auf Hybrid Retrieval zurück. Dadurch ist Hermes auf PostgreSQL weniger „fraktal“ als das Architekturziel.

### Ansatz

1. `PostgresStore::expand_fractal` implementieren: `children_tier_ids`, `parent_tier_id` und ggf. source-node links nachladen.
2. Nach Expansion weiterhin `retrieval_profile`, `memory_type_filter`, Governance und Intent-Scoring anwenden.
3. Tests für Postgres-Fractal-Parität ergänzen.

**Begründung:** KnowWheres Kernversprechen ist Fractal Zoom. Produktions-Hermes nutzt PostgreSQL; daher muss die Postgres-Seite denselben Navigationswert liefern.

---

## 3. 🔜 Current-vs-Historical konsequent machen

### Problem

Alte Aussagen wie „KnowWhere ist deaktiviert“ bleiben historisch wahr, dürfen aber aktuelle Antworten nicht dominieren.

### Ansatz

Ein additiver Repair-/Governance-Pfad:
1. Current-State-Claims mit `claim_scope=current`, `observed_at`, `valid_from` schreiben.
2. Historische Zustands-Claims als `claim_scope=historical` markieren.
3. Bei klaren Nachfolgern `superseded_by` setzen, ohne alte Nodes zu löschen.

**Begründung:** Moderne temporal RAG Benchmarks zeigen, dass stale knowledge einer der größten Fehlerquellen ist.

---

## 4. 🔜 Cross-Encoder Reranking aktivieren

Der Cross-Encoder (`bge-reranker-v2-m3`) ist implementiert und feature-gated. Nach Provenance/Intent lohnt sich die Aktivierung als Qualitätshebel.

**Aktivierung:**
```bash
SQLX_OFFLINE=true cargo build --release --features "postgres-storage,summarizer,reranker"
```

**Trade-off:** +2.5 GB RAM für das ONNX-Modell.

---

## 5. 📋 Quality-of-Life

- `cargo test --features postgres-storage` ohne manuelles `DATABASE_URL` besser dokumentieren/ergonomisieren.
- USearch-Warnings reduzieren.
- README/Cargo-Version synchronisieren.

---

## Priorisierung

| # | Item | Impact | Effort | Risk | Order |
|---|------|--------|--------|------|-------|
| 1 | Provenance + Retrieval-Diversität | 🔴 Hoch | 1 Tag | Mittel | **1** |
| 2 | Postgres-Fractal-Expansion | 🔴 Hoch | 1-2 Tage | Mittel | **2** |
| 3 | Current-vs-Historical Repair | 🔴 Hoch | 1 Tag | Mittel | **3** |
| 4 | Cross-Encoder aktivieren | 🟡 Mittel | 30min | RAM | 4 |
| 5 | Test-Ergonomie / Warnings | 🟢 Niedrig | 1-2h | Niedrig | 5 |

**Kritischer Pfad:** Provenance Coverage hochziehen, repeated Top-1 senken, danach Postgres-Fractal-Parität herstellen.

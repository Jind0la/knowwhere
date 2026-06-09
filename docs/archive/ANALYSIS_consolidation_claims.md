# Consolidation + Pointer-Resolution: Minimaler Änderungspfad

> **Erstellt:** 2026-05-04 für Task `t_37ed8154`
> **Repo-Commit:** `54265a4` — `feat: MemoryType::Decision — structured decision retrieval`

---

## 1. Consolidation-Prompt: Von Prosa zu Claims

### Wo wird der Prompt gebaut?

Es gibt **zwei getrennte Prompt-Builder**, die beide geändert werden müssen:

#### A) VLM Worker Prompts — `src/vlm/mod.rs`

| Methode | Zeilen | Beschreibung |
|---|---|---|
| `SummaryContext::system_directive()` | **131–157** | System-Prompt pro Context-Tier (Summary/Overview/Detailed) |
| `SummaryContext::prompt_template()` | **161–173** | User-Prompt-Template mit `{content}` Platzhalter |

**Aktuelles Format (alle drei Tiers):** Narrative Prosa. Der Prompt verlangt "One sentence", "2-3 sentences", "3 paragraphs" — aber immer als Fließtext.

**Call-Site (wo Prompt + Content kombiniert wird):**
- `src/vlm/mod.rs` **Zeile 522–523**: `system_msg = context.system_directive(); user_prompt = context.prompt_template().replace("{content}", prompt);`
- `src/vlm/mod.rs` **Zeile 897–909**: `VlmWorker::process_job()` kombiniert Content für den Prompt (max 4000 chars pro Item)

#### B) LocalSummarizer Prompts — `src/summarizer/mod.rs`

| Methode | Zeilen | Beschreibung |
|---|---|---|
| `LocalSummarizer::summarize()` | **107–117** | L0-Prompt: "ONE sentence, mention decisions" |
| `LocalSummarizer::summarize_with_length()` | **172–189** | L1-Prompt: "2-3 sentences, structured" |

**Beide existieren bereits mit Entscheidungs-Fokus** (enthalten "decision" / "decided" im Prompt), aber die Ausgabe ist ebenfalls narrative Prosa ohne strukturierte Claims.

### Was muss geändert werden?

**Änderung A1 — VLM `system_directive()` (src/vlm/mod.rs:131–157):**

In jedem der drei Match-Arme muss am Ende ein neuer Absatz hinzugefügt werden:

```
After your prose summary, append a machine-readable claims block:

---CLAIMS---
- claim: <decision statement>
  reason: <why this decision was made>
  entities: [entity1, entity2]
- claim: <next decision>
  ...
---END---

Only include claims for explicit decisions. If no decisions were made, omit the block.
```

**Änderung A2 — VLM `prompt_template()` (src/vlm/mod.rs:161–173):**

Keine Änderung nötig — das Template ist nur `{content}`. Der System-Prompt steuert das Format.

**Änderung A3 — LocalSummarizer (src/summarizer/mod.rs:111–117 und 181–189):**

Gleicher "---CLAIMS---" Block ans Ende des Prompts. Da Ollama deterministisch ist (temperature=0, seed=42), ist das Format vorhersagbar.

**⚠️ Riskante Zeile — L229** (src/scheduler/consolidation.rs): Wenn `summarize_for_tier()` für den VLM-Fallback verworfen wird, weil LocalSummarizer fehlschlägt, wird der **original unprompted VLM** verwendet — der hat den neuen Claims-Prompt dann NICHT. Workaround: Prompt sowohl im LocalSummarizer als auch im VLM-Pfad ändern.

**LOCs: ~30 Zeilen** (je 10 pro Context-Tier in `system_directive()`, ~5 im `summarize()`, ~5 in `summarize_with_length()`)

---

## 2. Claim-Parsing: Wo wird der Claim-Block verarbeitet?

### Aktueller Flow

```
src/scheduler/consolidation.rs:294
  self.local_summarizer.summarize_for_tier(&content, Overview)
    ↓ erzeugt SummaryResult { text: "..." }
Zeile 299–306
  let l1_content = summary.text.clone();
  let l1_embedding = self.embed_text(&l1_content).await?;
  let l1_type = if is_decision_content(&l1_content) { Decision } else { Semantic };
    ↓ is_decision_content() in Zeile 23–29: keyword-Match auf "decision:", "decided", etc.
Zeile 308–321
  FractalNode::new_typed(Some(l1_content), ...) — der GESAMTE Text wird als content gespeichert
```

### Insertion Point für Claim-Parsing

**Primärer Insertion Point: `src/scheduler/consolidation.rs` Zeile 294–306**

Nachdem `summarize_for_tier()` zurückkommt, muss der Claim-Block aus `summary.text` extrahiert werden:

```rust
// Nach Zeile 297 (summary erhalten)
let (narrative, claims) = parse_claims_block(&summary.text);
// narrative = alles vor "---CLAIMS---"
// claims = Vec<Claim { statement, reason, entities }>
```

**Neue Hilfsfunktion `parse_claims_block()`** — entweder in `src/scheduler/consolidation.rs` (privat) oder in `src/memory/claims.rs` (neue Datei):

```rust
struct Claim {
    statement: String,
    reason: String,
    entities: Vec<String>,
}

fn parse_claims_block(text: &str) -> (String, Vec<Claim>) {
    // Split bei "---CLAIMS---"
    // Parse YAML-ähnliches Format darunter
    // Bei Parse-Fehler: gesamten Text als narrative zurückgeben (Graceful Degradation)
}
```

**Was passiert mit den Claims?**

Nach dem Parsen:
1. **Narrative Teil** → wird wie bisher als `content` der L1/L0-Nodes gespeichert
2. **Jeder Claim** → wird als separater `FractalNode` mit `MemoryType::Decision` erstellt
3. Diese Decision-Nodes bekommen `parent_tier_id` → L1-Overview-Node
4. Die L1-Node bekommt `children_tier_ids` erweitert um die Decision-Node-IDs

**Zweiter Insertion Point: `src/vlm/mod.rs` Zeile 947–991 (VlmWorker::process_job)**

Der VLM-Worker erzeugt auch Summary-Nodes. Gleiche Claim-Extraktion hier:

```rust
// Nach Zeile 928 (summary_text erhalten)
let (narrative, claims) = parse_claims_block(&summary_text);
let summary_text_clean = narrative; // Claims entfernt für den Embedding-Text
```

**LOCs: ~60–80 Zeilen**
- `parse_claims_block()`: ~40 Zeilen
- Claim-Node-Erstellung in `process_local_compaction()`: ~20 Zeilen
- Claim-Node-Erstellung in `VlmWorker::process_job()`: ~20 Zeilen

---

## 3. Pointer-Resolution: `expand_pointers` in `/retrieve_fractal`

### Ausgangslage

**Es gibt aktuell KEINE Methode, um Nodes via `session_id + turn_index` zu laden.**

Die `StoreSessionRequest` (routes.rs:373–401) akzeptiert `session_id` und `turn_index` **beim Speichern**, aber sie landen nur im `metadata` HashMap der FractalNode. Das `StorageBackend` Trait hat keine Query-Methode dafür.

### Implementierungsplan

**Schritt 3a — StorageBackend Trait erweitern (`src/storage/backend.rs`)**

Neue Trait-Methode (nach Zeile 343):

```rust
/// Find nodes by metadata key-value pairs (e.g., session_id + turn_index).
/// Returns all matching nodes.
async fn find_by_metadata(
    &self,
    filter: &HashMap<String, String>,
) -> anyhow::Result<Vec<FractalNode>>;
```

**Default-Implementierung** (für Backends, die es nicht nativ unterstützen):

```rust
async fn find_by_metadata(&self, filter: &HashMap<String, String>) -> anyhow::Result<Vec<FractalNode>> {
    let all = self.list_all().await?;
    Ok(all.into_iter()
        .filter(|node| {
            filter.iter().all(|(k, v)| {
                node.metadata.get(k)
                    .and_then(|val| val.as_str())
                    .map(|s| s == v.as_str())
                    .unwrap_or(false)
            })
        })
        .collect())
}
```

**Schritt 3b — `expand_pointers` Funktion (z.B. in `src/api/routes.rs`)**

Neue Utility-Funktion für den `/retrieve_fractal` Handler:

```rust
/// Löse Pointer (session_id + turn_index) zu Quell-Nodes auf.
/// Fügt source_content zu jedem ScoredNode hinzu.
async fn expand_pointers(
    store: &dyn StorageBackend,
    nodes: &mut [ScoredNode],
) -> anyhow::Result<()> {
    for node in nodes.iter_mut() {
        if let Some(pointer) = &node.original_pointer {
            // Parse pointer: "session:<session_id>:turn:<turn_index>"
            if let Some((session_id, turn_idx)) = parse_session_pointer(pointer) {
                let mut filter = HashMap::new();
                filter.insert("session_id".into(), session_id);
                filter.insert("turn_index".into(), turn_idx.to_string());

                if let Ok(sources) = store.find_by_metadata(&filter).await {
                    if let Some(source_node) = sources.first() {
                        node.source_content = source_node.content.clone();
                    }
                }
            }
        }
    }
    Ok(())
}
```

**Schritt 3c — Integration in `/retrieve_fractal` (routes.rs)**

In `retrieve_fractal()` (routes.rs:1676–1681), vor dem Return:

```rust
// Nach Zeile 1676 (non-governance path):
expand_pointers(state.store.as_ref(), &mut results).await
    .map_err(|e| {
        tracing::warn!("expand_pointers failed (non-fatal): {}", e);
        // Don't fail the request — pointer expansion is best-effort
    })?;
```

**ABER:** `ScoredNode` in routes.rs hat **kein `source_content` Feld**. Das muss hinzugefügt werden (siehe Frage 4).

**LOCs: ~50–60 Zeilen**
- `find_by_metadata` Trait-Methode + Default-Impl: ~15 Zeilen
- `find_by_metadata` für MemoryStore (in_memory.rs): ~10 Zeilen
- `find_by_metadata` für PostgresStore (postgres_store.rs): ~15 Zeilen (SQL WHERE jsonb)
- `expand_pointers` Utility: ~20 Zeilen

---

## 4. Response-Erweiterung: `source_content` in `ScoredNode`

### Wo wird FractalNode serialisiert?

**API-Response-Struct:** `src/api/routes.rs` **Zeile 53–79** — `ScoredNode`

Dies ist die API-Antwort-Struct (nicht zu verwechseln mit `src/storage/backend.rs` `ScoredNode` auf Zeile 254). Die API-Version hat:

| Feld | Typ | Zeile |
|---|---|---|
| `score` | `f32` | 55 |
| `id` | `Uuid` | 56 |
| `memory_type` | `MemoryType` | 58 |
| `content` | `Option<String>` | 62 |
| `original_pointer` | `Option<String>` | 63 |
| `metadata` | `HashMap<String, Value>` | 64 |
| `created_at` | `DateTime<Utc>` | 65 |
| ...Governance-Felder... | | 72–78 |

**Es gibt kein `source_content` Feld.**

### Änderung

**`src/api/routes.rs` Zeile 63 — nach `original_pointer` einfügen:**

```rust
/// When expand_pointers is enabled, contains the source node's full content.
#[serde(skip_serializing_if = "Option::is_none")]
pub source_content: Option<String>,
```

**`RetrieveFractalRequest` (routes.rs:1202–1225) — neuen Parameter hinzufügen:**

```rust
/// Expand pointer references to include source content (default: false).
#[serde(default)]
pub expand_pointers: bool,
```

**Im `retrieve_fractal` Handler (routes.rs:1676):**

```rust
if req.expand_pointers {
    // Temporärer Vec für Mutable-Borrow
    let mut expandable: Vec<_> = results.iter_mut().collect();
    expand_pointers(state.store.as_ref(), &mut expandable).await
        .unwrap_or_else(|e| tracing::warn!("expand_pointers: {}", e));
}
```

**LOCs: ~15 Zeilen**
- `source_content` Feld: 3 Zeilen
- `expand_pointers` Request-Param: 3 Zeilen
- Integration in Handler: 5 Zeilen

---

## 5. Aufwandsschätzung

| Schritt | Beschreibung | LOC | Zeit (geschätzt) | Dateien |
|---|---|---|---|---|
| **1. Prompt-Änderung** | Claims-Block in VLM + LocalSummarizer Prompts | ~30 | 30 min | vlm/mod.rs, summarizer/mod.rs |
| **2. Claim-Parsing** | `parse_claims_block()` + Decision-Node-Erstellung | ~70 | 2 h | scheduler/consolidation.rs, vlm/mod.rs (worker) |
| **3a. StorageBackend** | `find_by_metadata` Trait + Impls | ~40 | 1.5 h | backend.rs, in_memory.rs, postgres_store.rs |
| **3b. expand_pointers** | Utility-Funktion | ~20 | 1 h | api/routes.rs (oder neue Datei) |
| **4. Response** | `source_content` Feld + Request-Param | ~15 | 30 min | api/routes.rs |
| **Tests** | Unit-Tests für Parsing + Integration | ~80 | 2 h | scheduler/consolidation.rs (tests), memory/tests.rs |
| **Doku** | OpenAPI Schema (utoipa annot.) | ~10 | 15 min | api/routes.rs |
| **Gesamt** | | **~265** | **~7.5 h** | 6 Dateien |

---

## 6. Risiken & Pitfalls

| Risiko | Impact | Mitigation |
|---|---|---|
| **Breaking API Change:** `source_content` ist ein neues Feld — Clients die strikt validieren könnten brechen | Low | `#[serde(skip_serializing_if = "Option::is_none")]` — Feld ist nur da wenn `expand_pointers=true` |
| **VLM produziert keinen Claims-Block:** LLM ignoriert das "---CLAIMS---" Format | Medium | `parse_claims_block()` muss Graceful Degradation haben: bei Parse-Fehler → gesamter Text als narrative, keine Claims → kein Fehler |
| **Ollama-Determinismus:** LocalSummarizer mit temp=0, seed=42 ist deterministisch, aber nur wenn das Modell identisch ist. Modell-Update kann Output ändern. | Low | Tests mit festem Modell-Snapshot; CI prüft auf deterministische Ausgabe |
| **Performance `find_by_metadata`:** Default-Impl scannt ALLE Nodes → O(n) | Medium | Postgres-Store implementiert natives JSONB-Query (`metadata->>'session_id'`). MemoryStore für Dev-Akzeptanz OK. |
| **VLM Worker Pfad übersehen:** Claims-Parsing muss in BEIDEN Pfaden passieren (LocalSummarizer + VlmWorker::process_job) | High | Explizit dokumentiert; beide Pfade haben den gleichen `parse_claims_block()` Aufruf |
| **Claim-Embedding-Qualität:** Decision-Nodes mit nur der Claim-Statement-Embedding könnten weniger gut retrieven als narrative Summaries | Medium | Decision-Nodes bekommen separate Embedding; Tests mit `memory_type_filter=decision` in `/retrieve_fractal` |

---

## 7. Empfehlung: Absolut minimaler erster Schritt

**Prompt-Änderung (`src/vlm/mod.rs` `system_directive()`) + `parse_claims_block()` in `src/scheduler/consolidation.rs`.**

Das ist der kritische Pfad, alles andere baut darauf auf:

1. **Erst** die Prompts ändern (VLM + LocalSummarizer) → Claims tauchen im Output auf
2. **Dann** `parse_claims_block()` schreiben → Claims werden extrahiert und als Decision-Nodes gespeichert
3. **Dann** testen mit `POST /retrieve_fractal` + `memory_type_filter: "decision"` → Decision-Nodes sind retrievable
4. **Erst danach** `expand_pointers` / `source_content` implementieren → das ist ein separater Feature-Strang

**Warum dieser Einstieg?**
- Prompt-Änderung ist ~30 LOC, sofort testbar
- `parse_claims_block()` validiert ob das Prompt-Format überhaupt funktioniert
- Wenn die Claims-Extraktion nicht klappt (VLM zu inkonsistent), ist der Rest des Plans hinfällig → früh scheitern, nicht spät
- `expand_pointers` ist orthogonal und kann parallel entwickelt werden

---

## 8. Architektur-Diagramm (Änderungen im Flow)

```
┌──────────────────────────────────────────────────────────────┐
│                    Consolidation Flow                         │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  L2 (Raw) Content                                             │
│       │                                                       │
│       ▼                                                       │
│  ┌─────────────────┐     ┌──────────────────┐                │
│  │ LocalSummarizer │     │  VLM Worker      │                │
│  │ (Ollama)        │     │  (GPT/Grok/etc)  │                │
│  │                 │     │                  │                │
│  │ Prompt +        │     │  Prompt +        │                │
│  │ "---CLAIMS---"  │     │  "---CLAIMS---"  │  ← ÄNDERUNG 1  │
│  └────────┬────────┘     └────────┬─────────┘                │
│           │                       │                           │
│           └───────────┬───────────┘                           │
│                       ▼                                       │
│              ┌─────────────────┐                              │
│              │ parse_claims_   │  ← NEU (ÄNDERUNG 2)          │
│              │ block()         │                              │
│              └────────┬────────┘                              │
│                       │                                       │
│          ┌────────────┼────────────┐                          │
│          ▼            ▼            ▼                          │
│   Narrative     Claim 1      Claim 2                          │
│   → L1 Node     → Decision   → Decision                      │
│   (Overview)      Node          Node                          │
│       │              │            │                           │
│       └──────────────┼────────────┘                           │
│                      │ children_tier_ids                      │
│                      ▼                                        │
│              L0 (Summary) Node                                │
│                                                               │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│                  Retrieval Flow                               │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  POST /retrieve_fractal                                       │
│       │                                                       │
│       ▼                                                       │
│  ┌──────────────────┐                                        │
│  │ hybrid_retrieve  │  (vector + BM25)                       │
│  │ + governance     │                                        │
│  └────────┬─────────┘                                        │
│           │                                                   │
│           ▼                                                   │
│  ┌──────────────────┐                                        │
│  │ expand_pointers  │  ← NEU (ÄNDERUNG 3+4)                  │
│  │ (wenn requested) │     Holt source_content via             │
│  │                  │     find_by_metadata()                  │
│  └────────┬─────────┘                                        │
│           │                                                   │
│           ▼                                                   │
│  ┌──────────────────┐                                        │
│  │ ScoredNode[]     │  mit source_content Feld                │
│  │ → JSON Response  │                                        │
│  └──────────────────┘                                        │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

---

## 9. Abhängigkeits-Reihenfolge

```
Phase 1: Claims (abhängig von nichts)
  ├── 1. Prompt-Änderung (vlm/mod.rs + summarizer/mod.rs)
  ├── 2. parse_claims_block() (scheduler/consolidation.rs)
  └── Tests

Phase 2: Pointer-Resolution (abhängig von 3a, orthogonal zu Phase 1)
  ├── 3a. find_by_metadata Trait (backend.rs + impls)
  ├── 3b. expand_pointers (routes.rs)
  └── 4. source_content Response-Feld (routes.rs)

Phase 3: Integration
  └── expand_pointers in retrieve_fractal aktivieren
```

---

*Analyse abgeschlossen. Nächster Schritt: T4 (analyst) synthetisiert mit T1 (Audit) und T2 (Research).*

# KnowWhere Retrieval Quality Test Plan

**Erstellt:** 2026-03-28
**Letztes Update:** 2026-04-24
**Status:** Tier 1 + Tier 2 implementiert (MemoryStore + PostgresStore Baselines). Benchmark-Binaries (LongMemEval Canary/Retrieval/QA) existieren. Tier 3 (Real-World Traces) offen.

---

## 1. Zielsetzung

### Was wir testen wollen

KnowWhere's Hybrid Retrieval (semantic + BM25) muss folgende Eigenschaften erfüllen:


| Eigenschaft        | Bedeutung                                            | Warum wichtig                   |
| ------------------ | ---------------------------------------------------- | ------------------------------- |
| **Precision**      | Gefundene Results sind relevant                      | Kein Noise im LLM-Kontext       |
| **Recall**         | Alle relevanten Results werden gefunden              | Keine Informationslücken        |
| **Robustheit**     | Unterschiedliche Formulierungen finden dasselbe      | Natürliche Sprache funktioniert |
| **Zeitstabilität** | Performance bleibt mit wachsender DB gleich          | Langfristige Zuverlässigkeit    |
| **Deduplizierung** | Gleiche Information wird nicht doppelt zurückgegeben | Kein redundanter Kontext        |


### Was wir NICHT testen

- **LLM-Answer-Qualität** — das ist Aufgabe des LLM, nicht von KnowWhere
- **Embedding-Qualität des Modells** — Ollama's Modell ist extern
- **API-Performance/Latenz** — das ist Infrastructure, nicht Retrieval-Qualität

---

## 2. Test-Struktur

### Hierarchie: 3 Test-Level

```
Tier 1: Echo-Test (dieses Dokument)
├── Was: Schneller Smoke-Test für Core-Retrieval
├── Wann: Bei jedem Commit (CI)
├── Daten: 20 künstliche "Echo-Memories"
└── Ziel: Regressionen sofort erkennen

Tier 2: Wachsende-DB Regression Suite
├── Was: Recall/Precision bei steigender DB-Größe
├── Wann: Wöchentlich oder auf Anfrage
├── Daten: Echo-Memories + deterministische Noise-Memories (100/500/1000)
└── Ziel: Skalierungs-Probleme erkennen

Tier 3: Real-World Trace Test
├── Was: Echte Konversationen als Test-Daten
├── Wann: Monatlich
├── Daten: Extrahierte Hermes-Sessions
└── Ziel: Authentische Performance-Messung
```

---

## 3. Tier 1: Echo-Test (CI-optimiert)

### 3.1 Echo-Memory-Datensatz

20 künstliche Memories, die jeweils ein eindeutiges Faktum enthalten. Jedes Faktum ist so formuliert, dass es **semantisch unterscheidbar** aber **syntaktisch variabel** abgefragt werden kann.

```rust
// targets: was der Test zu finden erwartet
// queries: verschiedene Wege, das Ziel zu finden

static ECHO_MEMORIES: &[(&str, &[&str])] = &[
    // ---------------------------------------------------------
    // Gruppe A: Personbezogene Fakten
    // ---------------------------------------------------------
    (
        "Nimar works on KnowWhere, a fractal memory service for AI agents.",
        &[
            "Nimar's current project",
            "Was macht Nimar beruflich?",
            "the guy building a memory system",
            "fractal memory service",
            "KnowWhere developer",
        ],
    ),
    (
        "Nimar prefers the Pointer-First architecture for KnowWhere.",
        &[
            "Nimar's preferred architecture",
            "Wie baut Nimar KnowWhere?",
            "Pointer-First architecture",
            "architectural approach for memory",
        ],
    ),
    (
        "KnowWhere uses Axum 0.8, USearch for vectors, and BM25 for keyword search.",
        &[
            "KnowWhere technology stack",
            "Womit ist KnowWhere gebaut?",
            "USearch or BM25",
            "vector search implementation",
        ],
    ),
    (
        "The embedding model for KnowWhere is nomic-embed-text-v2-moe with 768 dimensions.",
        &[
            "embedding model name",
            "Wie heißt das Embedding-Modell?",
            "nomic embed text",
            "768 dimensions",
        ],
    ),
    (
        "KnowWhere stores memories with PostgreSQL on port 5433 in Docker.",
        &[
            "KnowWhere database setup",
            "Wo laufen die Memories?",
            "Docker PostgreSQL",
            "5433 port",
        ],
    ),
    // ---------------------------------------------------------
    // Gruppe B: Bug/Problem-Fakten
    // ---------------------------------------------------------
    (
        "BUG-007 was a PostgresStore count() bug caused by a SQL reserved word collision.",
        &[
            "BUG-007",
            "count() returned 0",
            "SQL reserved word",
            "PostgresStore bug",
        ],
    ),
    (
        "The fix for BUG-007 was changing try_get('count') to try_get(0) using index.",
        &[
            "BUG-007 fix",
            "try_get(0)",
            "count bug solution",
            "SQL alias fix",
        ],
    ),
    (
        "The embedding dimension for KnowWhere is 768, not 512 or 1024.",
        &[
            "embedding dimension size",
            "Wie viele Dimensionen?",
            "768",
            "vector size",
        ],
    ),
    (
        "OpenClaw uses Plugin Lifecycle Hooks 'before_prompt_build' for memory injection.",
        &[
            "OpenClaw hook name",
            "before_prompt_build",
            "OpenClaw integration method",
            "prompt hook",
        ],
    ),
    (
        "Dream Mode in KnowWhere reorganizes memories automatically in the background.",
        &[
            "Dream Mode function",
            "Was macht Dream Mode?",
            "automatic memory reorganization",
            "background consolidation",
        ],
    ),
    // ---------------------------------------------------------
    // Gruppe C: Konzept-Fakten (unterschiedliche Abstraktionsebenen)
    // ---------------------------------------------------------
    (
        "KnowWhere implements Pointer-First architecture: external data stays external.",
        &[
            "Pointer-First meaning",
            "Was bedeutet Pointer-First?",
            "external data pointer",
            "no raw file storage",
        ],
    ),
    (
        "KnowWhere uses Reciprocal Rank Fusion (RRF) to combine vector and BM25 results.",
        &[
            "Reciprocal Rank Fusion",
            "RRF",
            "fusion method",
            "how results are combined",
        ],
    ),
    (
        "Fractal zoom retrieval means accessing memories at different granularity levels.",
        &[
            "Fractal zoom",
            "Zoom retrieval",
            "granularity levels memory",
            "different detail levels",
        ],
    ),
    (
        "Governance policy in KnowWhere filters memories by confidence and sensitivity.",
        &[
            "Governance policy",
            "memory filtering",
            "confidence threshold",
            "sensitivity levels",
        ],
    ),
    // ---------------------------------------------------------
    // Gruppe D: Ablauf/Prozess-Fakten
    // ---------------------------------------------------------
    (
        "Tests for KnowWhere run with: cargo test --features postgres-storage.",
        &[
            "How to run tests",
            "cargo test postgres-storage",
            "test command",
            "running the test suite",
        ],
    ),
    (
        "KnowWhere API runs on port 3737 by default.",
        &[
            "default API port",
            "Which port?",
            "3737",
            "localhost port",
        ],
    ),
    (
        "The storage backend in KnowWhere is abstracted via the StorageBackend trait.",
        &[
            "StorageBackend trait",
            "backend abstraction",
            "trait for storage",
            "MemoryStore vs PostgresStore",
        ],
    ),
    // ---------------------------------------------------------
    // Gruppe E: Randfälle (Synonyme, Umkehrungen, Negationen)
    // ---------------------------------------------------------
    (
        "KnowWhere does NOT store raw files — only pointers to external sources.",
        &[
            "Does KnowWhere store raw files?",
            "Was speichert KnowWhere?",
            "pointers only",
            "external references only",
        ],
    ),
    (
        "KnowWhere does NOT use Redis — it uses USearch locally or pgvector in PostgreSQL.",
        &[
            "Redis used?",
            "Nutzt KnowWhere Redis?",
            "USearch vs Redis",
            "vector engine comparison",
        ],
    ),
    (
        "ARC<dyn StorageBackend> allows swapping MemoryStore and PostgresStore at runtime.",
        &[
            "Arc dyn StorageBackend",
            "dynamic backend switching",
            "runtime storage choice",
            "trait object storage",
        ],
    ),
];
```

**Design-Prinzipien des Datensatzes:**

- Jedes Memory enthält **genau ein** unterscheidbares Faktum
- Queries sind **semantisch verwandt** aber **syntaktisch unterschiedlich** (DE/EN, Frageform/Statement, vollstständig/teils)
- Group C/D/E testen **Konzepte** statt Fakten — wichtig für Robustheit
- 20 Memories sind **genug für Signal, schnell genug für CI**

---

### 3.2 Metriken

```rust
/// Result einer einzelnen Query
struct QueryResult {
    target_memory_id: usize,       // Index des erwarteten Memory
    found_in_top_k: Option<usize>,  // None = nicht gefunden, Some(n) = bei Position n
    retrieved_ids: Vec<usize>,      // Alle zurückgegebenen IDs
}

/// Aggregierte Metriken
struct EchoMetrics {
    // Precision@K: Anteil der Top-K Results die relevant sind
    precision_at_1: f64,   // Top-1 Genauigkeit
    precision_at_3: f64,   // Top-3 Genauigkeit
    precision_at_5: f64,   // Top-5 Genauigkeit

    // Recall@K: Anteil der relevanten Results die in Top-K gefunden werden
    recall_at_1: f64,
    recall_at_3: f64,
    recall_at_5: f64,

    // Mean Reciprocal Rank: Wie früh wird das Richtige gefunden?
    mrr: f64,

    // Spezielle Metriken
    fully_undiscoverable: usize,  // Queries die das Ziel NIE finden
    semantically_robust: f64,      // % Queries die das eigene Target finden
    false_positive_rate: f64,      // % irrelevanter Results in Top-5
}
```

---

### 3.3 Test-Implementation (Ist-Stand)

Tier 1 ist jetzt als echter Integrationstest umgesetzt:

- Testdatei: `tests/retrieval_quality.rs`
- Testname: `echo_retrieval_quality_baseline`
- Store: `MemoryStore` (deterministisch, ohne externe Modell-/Netzabhängigkeit)
- Query-Modus: Hybrid (`query_text` + deterministische `query_vector`)
- Metriken: `precision_at_1`, `recall_at_3`, `mrr`, `semantically_robust`
- Gates:
  - `precision_at_1 >= 0.70`
  - `recall_at_3 >= 0.85`
  - `mrr >= 0.75`
  - `semantically_robust >= 0.80`
- Zusätzlicher Latenz-Snapshot: `elapsed_ms` wird geloggt (noch kein harter Gate)

Historisches Pseudocode-Beispiel (als Referenz):

```rust
#[tokio::test]
async fn echo_retrieval_quality() {
    // Setup: Fresh store
    let store = fresh_postgres_store().await;

    // Phase 1: Echo-Memories einfügen
    let memory_ids: Vec<Uuid> = ECHO_MEMORIES
        .iter()
        .map(|(content, _)| {
            store.insert(FractalNode::new(content.to_string()))
        })
        .collect()
        .await;

    // Phase 2: Queries testen
    let mut all_results = Vec::new();

    for (idx, (content, queries)) in ECHO_MEMORIES.iter().enumerate() {
        for query_text in queries {
            let result = store
                .hybrid_retrieve(&HybridQuery {
                    query_text: Some(query_text.to_string()),
                    query_vector: None,  // BM25-only für reine Keyword-Tests
                    top_k: 5,
                    max_depth: 0,
                })
                .await
                .expect("retrieve failed");

            let found_position = result
                .iter()
                .position(|r| r.node.id == memory_ids[idx]);

            all_results.push(QueryResult {
                target_memory_id: idx,
                found_in_top_k: found_position.map(|p| p + 1),
                retrieved_ids: result.iter().map(|r| r.node.id).collect(),
            });
        }
    }

    // Phase 3: Metriken berechnen
    let metrics = compute_metrics(&all_results, ECHO_MEMORIES.len());

    // Assertions (CI-Gates)
    assert!(
        metrics.precision_at_1 >= 0.70,
        "Precision@1 too low: {:.2} (expected >= 0.70)",
        metrics.precision_at_1
    );
    assert!(
        metrics.mrr >= 0.75,
        "MRR too low: {:.2} (expected >= 0.75)",
        metrics.mrr
    );
    assert!(
        metrics.semantically_robust >= 0.80,
        "Semantic robustness too low: {:.2}% (expected >= 80%)",
        metrics.semantically_robust * 100.0
    );

    // Logging für Trends
    println!("ECHO METRICS: {:?}", metrics);
}
```

---

### 3.4 CI-Integration (Ist-Stand)

Die Echo-Baseline ist in den bestehenden CI-`test`-Job eingebunden:

- Workflow: `.github/workflows/ci.yml`
- Step: `Retrieval quality echo baseline`
- Kommando: `cargo test --test retrieval_quality`

Die Growing-DB Regression Suite läuft separat (manuell + geplant):

- Workflow: `.github/workflows/retrieval-regression.yml`
- Trigger: `workflow_dispatch` und wöchentlich per `schedule`
- Kommando: `cargo test --test retrieval_quality growing_db_retrieval_regression_suite -- --ignored`

Frühere Entwurfs-Variante mit separatem Workflow:

```yaml
# .github/workflows/retrieval-quality.yml
name: Retrieval Quality

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  echo-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Start PostgreSQL
        run: |
          docker run -d --name kw-test-pg \
            -e POSTGRES_DB=kw \
            -e POSTGRES_PASSWORD=kw \
            -p 5433:5432 \
            pgvector/pgvector:pg16
          sleep 5

      - name: Run Echo Test
        env:
          DATABASE_URL: postgresql://postgres:kw@localhost:5433/kw
        run: |
          cargo test --features postgres-storage echo_retrieval_quality

  growing-db-regression:
    # Wöchentlich oder manuell
    if: github.event_name == 'schedule' || contains(github.event.head_commit.message, '[regression]')
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # ... Tier 2 Implementation
```

---

## 4. Tier 2: Wachsende-DB Regression Suite

### 4.0 Implementierungsstand

Tier 2 ist als eigener ignorierter Integrationstest umgesetzt:

- Testdatei: `tests/retrieval_quality.rs`
- Testname: `growing_db_retrieval_regression_suite`
- Datenmengen: `100`, `500`, `1000` Noise-Memories
- Assertions pro Stufe:
  - `precision_at_1 >= 0.70`
  - `recall_at_3 >= 0.85`
  - `mrr >= 0.75`
  - `semantically_robust >= 0.80`
  - bei `1000` Nodes zusätzlich `p95_ms < 500`

### 4.1 Konzept

```
Woche 0:  20 Echo-Memories     → Baseline
Woche 1:  20 + 100 Mix-Memories → Regression?
Woche 2:  20 + 500 Mix-Memories → Regression?
Woche 4:  20 + 1000 Mix-Memories → Regression?
```

**Mix-Memories** = deterministisch erzeugte Noise-Memories, die thematisch unpassend sind, um größere DB-Volumina reproduzierbar zu simulieren.

### 4.2 Was wir tracken


| Metrik       | Erwartung              | Alarm wenn |
| ------------ | ---------------------- | ---------- |
| Precision@1  | Bleibt >= 0.70         | < 0.60     |
| Recall@3     | Bleibt >= 0.85         | < 0.75     |
| MRR          | Bleibt >= 0.75         | < 0.65     |
| Latenz (p95) | < 200ms bei 1000 Nodes | > 500ms    |


### 4.3 Deduplizierungs-Test

Zusätzlich: Ein wichtiger Test dass bei doppelten/redundanten Inputs das Retrieval nicht leidet:

```
Memory A: "Nimar works at Acme Corp"
Memory B: "Nimar is employed by Acme Corp"

Query: "Wo arbeitet Nimar?"
Expected: Genau EIN Result in Top-3, nicht zwei identische Resultate
```

---

## 5. Tier 3: Real-World Trace Test

### 5.1 Konzept

Echte Konversationen aus Hermes-Sessions als Test-Daten:

```
Session-Datum: 2026-03-27
Thema: KnowWhere PostgreSQL-Integration

Gespeicherte Messages:
- "Die count() Funktion gibt 0 zurück"
- "Wir haben BUG-007 gefunden"
- "Der Fix war try_get(0) statt try_get('count')"

Retrieval-Tests:
- "Erinnerst du den Bug von gestern?" → Soll BUG-007 finden
- "Was war das Problem mit count()?" → Soll BUG-007 finden
- "Wie haben wir das gelöst?" → Soll BUG-007 Fix finden
```

### 5.2 Sammlung der Test-Daten

```bash
# Export aus Hermes Session
hermes sessions export 2026-03-27 --format json > tests/fixtures/session-2026-03-27.json

# Automatisch extrahieren:
# 1. Alle store_session Calls
# 2. Die dazugehörigen retrieve Queries aus der Folgesession
```

### 5.3 Aufwand/Nutzen


| Aspekt          | Bewertung                          |
| --------------- | ---------------------------------- |
| Authentizität   | ★★★★★ — Echte User-Queries         |
| Wartbarkeit     | ★★☆☆☆ — Daten verändern sich       |
| Automatisierung | ★★★☆☆ — Braucht Session-Extraktion |
| Abdeckung       | ★★★★☆ — Alle Edge-Cases aus Praxis |


**Empfehlung:** Tier 3 erst nach Tier 1+2 stabil sind.

---

## 6. Test-Fixtures speichern

```
tests/
├── retrieval_quality.rs       # Tier 1 + Tier 2 (deterministische Rust-Fixtures)
└── integration.rs             # übrige API-/Auth-Integrationstests
```

---

## 7. Offene Fragen / Nächste Schritte

- **Database-Option:** Tier 1 nutzt jetzt `MemoryStore` als deterministische CI-Baseline; Postgres-Vergleich folgt in Tier 2.
- **Mix-Memory-Generator:** Für Tier 2 als deterministische Rust-Generatorfunktionen in `tests/retrieval_quality.rs` umgesetzt.
- **Test-Fixture-Struktur:** Für Tier 1/2 bewusst als Rust-Const + Generatoren umgesetzt (kein separates JSON nötig).
- **Recall ohne Ground Truth:** Bei Echo-Memories kennen wir die "richtige" Antwort. Bei echten Daten nicht. Wie definieren wir "gefunden"?
- **Persistenz:** Sollen Test-Resultate in eine JSON-Datei geloggt werden für Trends?
- **Schwellwerte:** Für Tier 1 aktiv gesetzt (`P@1 >= 0.70`, `Recall@3 >= 0.85`, `MRR >= 0.75`, `Robustheit >= 0.80`), werden nach Trenddaten weiter kalibriert.

---

## 8. Priorisierung


| Phase               | Aufwand          | Nutzen | Empfehlung                                                            |
| ------------------- | ---------------- | ------ | --------------------------------------------------------------------- |
| Tier 1: Echo        | ★★☆☆☆ (1-2 Tage) | ★★★★☆  | **Zuerst** — schnell Signal                                           |
| Tier 2: Growing DB  | ★★★☆☆ (3-5 Tage) | ★★★☆☆  | **Implementiert** — als ignored Regression Suite + separater Workflow |
| Tier 3: Real Traces | ★★★★☆ (1+ Woche) | ★★★★★  | **Später** — nur wenn nötig                                           |


---

## 9. Referenzen

- KnowWhere Core API: `src/api/routes.rs`
- Hybrid Retrieval: `src/storage/postgres_store.rs` — `hybrid_retrieve()`
- BM25 Implementation: `src/storage/postgres_store.rs` — `search_bm25()`
- RRF (Reciprocal Rank Fusion): `src/storage/in_memory.rs` — `hybrid_retrieve()`
- Similar: RAGAS Benchmark ([https://github.com/explodinggradients/ragas](https://github.com/explodinggradients/ragas))

---

## 10. Externe HuggingFace-Benchmarks (Tier 3/4)

### 10.1 Ziel und Abgrenzung

Interne Tier-1/2-Tests bleiben der harte CI-Guardrail. Externe Benchmarks dienen als produktnahe Validierung und kalibrieren, ob KnowWhere auch bei realistisch langen Konversations-Haystacks stabil bleibt.

- **Tier 3 (offline, nicht blockend):** LongMemEval + ConvoMem + LoCoMo-Subset lokal/manuell ausfuehren
- **Tier 4 (periodisch):** kleiner, fixer "canary subset" pro Benchmark 1x pro Woche
- **Nicht-Ziel:** jeder PR laeuft gegen komplette externen Datensaetze

### 10.2 Integrations-Matrix


| Benchmark                           | Primaeres Ziel                                     | Input-Format                                          | Eval-Fokus                                    | Empfohlener Startumfang                | CI-Stufe                |
| ----------------------------------- | -------------------------------------------------- | ----------------------------------------------------- | --------------------------------------------- | -------------------------------------- | ----------------------- |
| LongMemEval (`longmemeval-cleaned`) | Langzeit-Recall ueber viele Sessions               | timestamped `haystack_sessions`, QA pro `question_id` | Recall@K, MRR, QA-Exact/LLM-Judge, Abstention | `oracle` + kleiner `s_cleaned`-Subset  | Tier 3 -> Tier 4 canary |
| ConvoMem                            | Skalierungsstabilitaet ueber viele Kontextgroessen | `messages`, `question`, `answer`, `evidence_type`     | Accuracy je evidence_type + Kontextgroesse    | 3 evidence_types, 6/20/70/150 messages | Tier 3 -> Tier 4 canary |
| LoCoMo (legacy/Subset)              | Multi-Session + temporal/causal Robustheit         | `evidenceItems`, `conversations`, category-basiert    | QA-F1/EM, temporal reasoning slices           | nur `category_4_multi_session`-Subset  | Tier 3 (spaeter Tier 4) |


### 10.3 Daten-Mapping nach KnowWhere

Pointer-first bleibt strikt bestehen:

- **Session-/Dialogtexte:** als `store_session` (voller Text + Embeddings)
- **Externe Artefakte (z. B. Bildreferenzen):** als `store_external` mit `original_pointer`, ohne Rohdatei
- **Frage-Instanz-Metadaten:** in `metadata` (z. B. `benchmark`, `question_id`, `category`, `context_size`)

Konkretes Mapping:


| Feld in Benchmark                    | KnowWhere Speicherung                          | Zweck                          |
| ------------------------------------ | ---------------------------------------------- | ------------------------------ |
| `question_id` / Testfall-ID          | `metadata.benchmark_question_id`               | eindeutige Rueckverfolgbarkeit |
| Session/Conversation Turns           | `content` in Session-Node                      | Retrieval-Basis                |
| Gold-Answer                          | `metadata.gold_answer`                         | automatische Auswertung        |
| Evidence-Type/Category               | `metadata.evidence_type` / `metadata.category` | Slice-Analysen                 |
| Zeitstempel (`question_date` o. ae.) | `metadata.source_timestamp`                    | Temporal-Checks                |


### 10.4 Runner-Design (repo-nah)

Empfohlene additive Struktur:

```text
benchmarks/
├── hf/
│   ├── README.md
│   ├── longmemeval_runner.rs
│   ├── convomem_runner.rs
│   ├── locomo_runner.rs
│   ├── shared_metrics.rs
│   └── fixtures/
│       └── canary_subsets/*.jsonl
└── reports/
    └── retrieval_quality_external/
```

Ablauf pro Runner:

1. Datensatz laden (lokal heruntergeladen, kein CI-Download zur Laufzeit)
2. Dialoge in KnowWhere einspeisen (`store_session`/`store_external`)
3. Fragen ueber `retrieve_fractal` + optional `chat/subconscious` auswerten
4. Metriken als JSON + Markdown-Report schreiben
5. Exit-Code nur bei Canary-Gates in Tier 4 hart setzen

### 10.5 Metriken und Gates

Gemeinsame Kernmetriken:

- Retrieval: `Recall@3`, `Recall@5`, `MRR`
- Antwortqualitaet: `Exact Match` (wenn eindeutig), sonst semantischer Match/LLM-Judge
- Halluzinationskontrolle: `Abstention accuracy` bei no-evidence Fragen
- Latenz: `p95` pro Benchmark-Subset

Start-Gates fuer Tier-4-Canary (initial konservativ):

- `Recall@5 >= 0.75`
- `MRR >= 0.65`
- `Abstention accuracy >= 0.80`
- `p95 < 1200ms` (auf Canary-Daten)

### 10.6 CI-/Workflow-Design

- **PR-CI (bestehend):** nur Tier 1
- **Weekly Regression (bestehend):** Tier 2
- **Neu: external-regression.yml (weekly + manual):**
  - Job A: LongMemEval canary
  - Job B: ConvoMem canary
  - Job C: LoCoMo category_4 canary (optional am Anfang)
  - Artefakt: `benchmarks/reports/*.json` + `*.md`

### 10.7 Aufwand in Tagen (realistisch)


| Schritt                                     | Aufwand     | Ergebnis                                      |
| ------------------------------------------- | ----------- | --------------------------------------------- |
| Daten-Caching + Canary-Subsets definieren   | 1 Tag       | reproduzierbare kleine Testmenge              |
| Shared Metrics Layer                        | 1 Tag       | einheitliche Auswertung ueber alle Benchmarks |
| LongMemEval Runner (oracle + small subset)  | 2 Tage      | erster externer End-to-End Lauf               |
| ConvoMem Runner (3 types x 4 context sizes) | 2 Tage      | Skalierungsvergleich mit internen Metriken    |
| LoCoMo Runner (category_4 subset)           | 2 Tage      | temporale Multi-Session-Validierung           |
| CI Workflow + Artefakt-Reporting            | 1 Tag       | automatisierbarer Wochenlauf                  |
| **Gesamt (Tier 3 MVP)**                     | **~9 Tage** | externe Benchmark-Schicht produktiv nutzbar   |


### 10.8 Priorisierte Implementierungsreihenfolge

1. LongMemEval (niedrige Integrationskomplexitaet, hoher Signalwert)
2. ConvoMem (starke Skalierungsdiagnose)
3. LoCoMo (hoher Realismus, hoeherer Integrationsaufwand)

Damit bleibt die Architektur additive, pointer-first und CI-freundlich: intern schnell blockend, extern tief aber zeitlich entkoppelt.
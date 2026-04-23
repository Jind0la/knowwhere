# HuggingFace External Benchmarks (Tier 3/4)

Dieser Ordner ist die erste Integrationsstufe fuer externe Retrieval-Validierung.
Wir priorisieren bewusst **LongMemEval zuerst**, bevor ConvoMem und LoCoMo folgen.

## Warum zuerst LongMemEval?

- geringere Integrationskomplexitaet als LoCoMo
- klarer QA-Flow fuer ersten End-to-End Runner
- guter Signalwert fuer Langzeit-Retrieval + Abstention

## Aktueller Stand

- Canary-Runner: `cargo run --bin longmemeval_canary`
- Shared Metrics: `src/benchmarks/hf/shared_metrics.rs`
- LongMemEval Runner: `src/benchmarks/hf/longmemeval_runner.rs`
- Canary Fixture: `benchmarks/hf/fixtures/longmemeval_oracle_canary.json`

## Ausfuehrung

```bash
# Voraussetzung: KnowWhere Server laeuft lokal
# z. B. KNOWWHERE_API_KEY=kw_admin_default_change_me cargo run

# Pflicht
export KNOWWHERE_API_KEY=kw_admin_default_change_me

# optional
export KNOWWHERE_BENCH_BASE_URL=http://127.0.0.1:3737
export KNOWWHERE_BENCH_TOP_K=5
export KNOWWHERE_LONGMEMEVAL_CANARY=benchmarks/hf/fixtures/longmemeval_oracle_canary.json
export KNOWWHERE_BENCH_MAX_CASES=10

cargo run --bin longmemeval_canary
```

Hinweis: Der Canary-Runner nutzt deterministische Test-Vektoren und kann daher auch ohne aktive Embedding-Backend-Verbindung laufen.

## Initiale Canary Gates

- `Recall@5 >= 0.75`
- `MRR >= 0.65`
- `Abstention accuracy >= 0.80`

## Naechste Schritte

1. echtes LongMemEval `oracle`-Subset einhaengen
2. optional Antwort-Evaluation via `/chat/subconscious` ergaenzen
3. danach ConvoMem-Runner auf derselben Shared-Metrics-Basis aufbauen

## Vergleichbarer Retrieval-Run (LongMemEval)

Dieser Run nutzt echtes LongMemEval-Schema und schreibt einen JSON-Report mit `Top1`, `Recall@5`, `MRR`.

```bash
# Server muss laufen
export KNOWWHERE_API_KEY=kw_admin_default_change_me
export KNOWWHERE_BENCH_BASE_URL=http://127.0.0.1:3737

# Testweise tiny fixture:
export KNOWWHERE_LONGMEMEVAL_DATASET=benchmarks/hf/fixtures/longmemeval_retrieval_tiny.json
export KNOWWHERE_BENCH_TOP_K=5
export KNOWWHERE_BENCH_MAX_CASES=50
export KNOWWHERE_LONGMEMEVAL_REPORT=benchmarks/reports/retrieval_quality_external/longmemeval_retrieval_report.json

cargo run --bin longmemeval_retrieval_eval
```

Kurzlauf auf `s_cleaned` (Server muss laufen, Datei per `fetch_longmemeval_s_cleaned.sh`):

```bash
./scripts/bench/longmemeval_retrieval_s_cleaned_smoke.sh
```

Mit dem offiziellen Datensatz kannst du `KNOWWHERE_LONGMEMEVAL_DATASET` auf `longmemeval_oracle.json` oder `longmemeval_s_cleaned.json` setzen.

Zusätzliche Steuerung:

- `KNOWWHERE_BENCH_CASE_OFFSET` — Fälle überspringen (Fortsetzen / Teil-Runs).
- `KNOWWHERE_BENCH_STORE_DELAY_MS` — Pause zwischen `store_session` (Ollama-Last; Smoke-Skript setzt Default).
- `KNOWWHERE_EMBED_MAX_CHARS` — Max. Zeichen für Embedding nach `clean_for_embedding` (Default **512**, damit Ollama-Embedder nicht mit „context length“ abbricht; bei größerem Modell-Kontext z. B. `768` oder `1024` testen).

## QA-Hypothesen + Official Eval Hook

Dieser Run erzeugt `question_id`/`hypothesis` als JSONL und kann optional direkt `evaluate_qa.py` aufrufen.
Der QA-Runner nutzt den Server-Reader im Modus `answer_mode=qa` (kurze direkte Antwort, "I don't know" bei fehlender Evidenz).

```bash
export KNOWWHERE_API_KEY=kw_admin_default_change_me
export KNOWWHERE_BENCH_BASE_URL=http://127.0.0.1:3737
export KNOWWHERE_LONGMEMEVAL_DATASET=benchmarks/hf/fixtures/longmemeval_retrieval_tiny.json
export KNOWWHERE_LONGMEMEVAL_HYPOTHESES=benchmarks/reports/retrieval_quality_external/longmemeval_hypotheses.jsonl
export KNOWWHERE_BENCH_MAX_CASES=50
export KNOWWHERE_BENCH_TOP_K=5

# optional: offizielles Python-Eval
# export KNOWWHERE_LONGMEMEVAL_EVAL_SCRIPT=/pfad/zu/LongMemEval/src/evaluation/evaluate_qa.py
# export KNOWWHERE_LONGMEMEVAL_EVAL_MODEL=gpt-4o

cargo run --bin longmemeval_qa_eval
```

### Schnell-Smoke (Skript)

```bash
# Server starten, dann:
./scripts/bench/longmemeval_qa_smoke.sh
```

Per Umgebung steuerbar: `KNOWWHERE_LONGMEMEVAL_DATASET`, `KNOWWHERE_BENCH_MAX_CASES`, `KNOWWHERE_BENCH_FILTER_TYPES` (kommasepariert, z. B. `single-session-preference,multi-session`), `KNOWWHERE_LONGMEMEVAL_HYPOTHESES`.

Für realistischen **Retrieval**-Stress: `longmemeval_s_cleaned.json` nach `benchmarks/hf/third_party/longmemeval/data/` legen und `KNOWWHERE_LONGMEMEVAL_DATASET` darauf setzen; **oracle** bleibt nützlich, um **Reader**-Qualität von fehlendem Recall zu trennen.

Daten holen (offizielles HF-Dataset `xiaowu0162/longmemeval-cleaned`, ~265 MB):

```bash
./scripts/bench/fetch_longmemeval_s_cleaned.sh
export KNOWWHERE_LONGMEMEVAL_DATASET=benchmarks/hf/third_party/longmemeval/data/longmemeval_s_cleaned.json
./scripts/bench/longmemeval_qa_smoke.sh
```

Die Datei ist per `.gitignore` ausgeschlossen (zu gross fuer Git).

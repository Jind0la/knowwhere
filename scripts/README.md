# scripts/

Utility and automation scripts for KnowWhere development and operations.

## Benchmarking

| Script | Purpose |
|--------|---------|
| `benchmark.sh` | Standard benchmark run |
| `benchmark-full.sh` | Full benchmark with all modes |
| `benchmark-docker.sh` | Docker-based benchmark |
| `benchmark_amb_standard.py` | Agent Memory Benchmark standard runner |
| `benchmark_golden_queries.py` | Golden query evaluation |
| `setup_benchmark.sh` | Benchmark environment setup |
| `reset_benchmark.sh` | Reset benchmark state |
| `start_benchmark.sh` | Start benchmark server |

## Data & Migration

| Script | Purpose |
|--------|---------|
| `import_hermes_sessions.py` | Import Hermes Agent conversation sessions |
| `import_memories_to_pg.py` | Migrate memories to PostgreSQL |
| `ingest_longmemeval_bench.py` | Ingest LongMemEval benchmark data |
| `backfill_turn_storage.py` | Backfill turn-level embeddings |
| `test_migration_016.py` | Migration 016 validation |

## Evaluation & Quality

| Script | Purpose |
|--------|---------|
| `eval_retrieval.py` | Retrieval evaluation |
| `eval_retrieval_quality.py` | Retrieval quality assessment |
| `eval_hermes_retrieval.py` | Hermes-specific retrieval evaluation |
| `qualitative_retrieval_test.py` | Manual qualitative retrieval testing |
| `test_fractal_hierarchy.py` | Fractal hierarchy integrity test |
| `test_matryoshka.py` | Matryoshka embedding truncation test |

## Diagnostics

| Script | Purpose |
|--------|---------|
| `diagnose_consolidation.py` | Consolidation pipeline diagnostics |
| `diagnose_single_case.py` | Single-case retrieval debugging |
| `kw-health-check.sh` | Server health check |
| `repro-vector-bug.sh` | Vector storage bug reproduction |

## Development

| Script | Purpose |
|--------|---------|
| `seed.sh` | Seed database with test data |
| `pre-commit-hook.sh` | Pre-commit validation |
| `export_reranker_model.py` | Export ONNX reranker model |
| `knowwhere_memory_provider.py` | Hermes MemoryProvider plugin |
| `spike_claims_prompt.py` | Claims extraction prompt experimentation |

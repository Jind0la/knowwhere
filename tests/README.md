# tests/

Integration and quality tests for KnowWhere.

## Test Files

| File | What it tests |
|------|--------------|
| `integration.rs` | Full-stack integration: store → retrieve → consolidate cycle |
| `retrieval_quality.rs` | Retrieval precision, recall, MRR across all 6 memory types |
| `turn_storage.rs` | Per-turn embedding storage and retrieval correctness |
| `state_management.rs` | Server state, connection pooling, graceful shutdown |
| `openapi_contract.rs` | OpenAPI spec conformity |
| `test_soul.rs` | Soul.md / personality integration |

## Running Tests

```bash
# Unit tests (always work, no external dependencies)
cargo test --lib                          # 136 tests

# Integration tests (need PostgreSQL + Ollama)
DATABASE_URL="postgresql:///knowwhere_dev?host=localhost" \
OLLAMA_URL=http://127.0.0.1:11434 \
SQLX_OFFLINE=true \
cargo test --features postgres-storage --test integration
```

See the top-level [README.md](../README.md#development) for more details.

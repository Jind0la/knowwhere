# Embedding Dimension Migration: 768 → 1024

**Date:** 2026-05-07
**Reason:** Switched from nomic-embed-text-v2-moe (768-dim) to qwen3-embedding:0.6b (1024-dim)

## Steps Executed

### 1. Column Type Change
```sql
-- Check current constraint
SELECT atttypmod FROM pg_attribute
WHERE attrelid = 'memories'::regclass AND attname = 'embedding';
-- Result: 768 (fixed dimension)

-- Clear old embeddings (need re-embedding anyway with new model)
UPDATE memories SET embedding = NULL WHERE embedding IS NOT NULL;

-- Change to unrestricted vector type
ALTER TABLE memories ALTER COLUMN embedding TYPE vector;
-- Now accepts any dimension
```

### 2. Config Change
```bash
# ~/.knowwhere/.env
KNOWWHERE_EMBEDDING_PROVIDER=ollama
OLLAMA_MODEL=qwen3-embedding:0.6b
```

### 3. Batch Re-embedding (Python)
Instead of slow one-at-a-time re-embedding (8s/node via `/nodes/reembed_all`), used batch embedding:

```python
# Ollama supports batch embedding — 0.09s per document vs 8s individually
for batch in chunks(nodes, 20):
    resp = requests.post('http://127.0.0.1:11434/api/embed',
        json={'model': 'qwen3-embedding:0.6b', 'input': batch_texts})
    embeddings = resp.json()['embeddings']
    # Write each to DB
    for nid, emb in zip(batch_ids, embeddings):
        psql(f"UPDATE memories SET embedding = '{emb_str}'::vector WHERE id = '{nid}'")
```

### 4. Server Restart
```bash
# Capture env from running process FIRST
ps eww $(pgrep knowwhere-server) | tr ' ' '\n' | grep -E 'OPENAI|KNOWWHERE' > /tmp/kw_env_backup.txt

# Kill old, start with new config
kill $(pgrep knowwhere-server)
source ~/.knowwhere/.env
OLLAMA_MODEL=qwen3-embedding:0.6b ./target/release/knowwhere-server &
```

## Pitfalls Discovered

1. **Batch script bug**: First batch script wrote identical embeddings for all nodes in a batch (zip misalignment or psql UPDATE race). Fix: verify each batch with `psql -c "SELECT COUNT(DISTINCT embedding::text) FROM memories WHERE source='import'"`.

2. **Column without dimension constraint breaks TRUNCATE**: After `ALTER COLUMN TYPE vector` (unrestricted), PostgreSQL TRUNCATE fails with "column does not have dimensions". Use DELETE instead.

3. **qwen3 still truncates**: Empirical context is ~22K chars (not 32K tokens as advertised). Chunking remains essential.

## Rollback
To revert to nomic-embed-text-v2-moe:
```sql
UPDATE memories SET embedding = NULL WHERE embedding IS NOT NULL;
ALTER TABLE memories ALTER COLUMN embedding TYPE vector(768);
```
Then change `OLLAMA_MODEL=nomic-embed-text-v2-moe` in `.env` and re-embed.

## Current State
- Column type: vector (unrestricted)
- Active model: qwen3-embedding:0.6b (1024-dim)
- All active nodes re-embedded
- Server running with new config

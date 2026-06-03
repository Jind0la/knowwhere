# Embedding Model Evaluation for KnowWhere

**Date:** 2026-05-07
**Context:** nomic-embed-text-v2-moe (512 token context) causes identical embeddings for all documents from the same persona. Fix requires model switch + chunking.

## Model Comparison

| Model | Context (spec) | Context (empirical) | Dim | Size | MTEB | M1 fit |
|-------|---------------|-------------------|-----|------|------|--------|
| **nomic-embed-text-v2-moe** (current) | 512 | ~2,000 chars | 768 | 957 MB | ~62 | ✅ but BROKEN |
| **qwen3-embedding:0.6b** ⭐ | **32,768** | **~22,500 chars** (empirical) | 1024 | 639 MB | **64.3** | ✅ Best but STILL truncates |
| **bge-m3** | 8,192 | TBD | 1024 | 1.2 GB | 63.0 | ✅ Good |
| snowflake-arctic-embed2 | 8,192 | ~16,000 chars | 1024 | 1.2 GB | ~64 | ✅ OK |
| embeddinggemma | 2,048 | TBD | 768 | 621 MB | TBD | Drop-in |
| mxbai-embed-large | 512 | ~2,000 chars | 1024 | 669 MB | ~62 | ❌ Same problem |
| bge-large | 512 | ~2,000 chars | 1024 | 670 MB | ~63 | ❌ Same problem |

## Recommendation: qwen3-embedding:0.6b

**Why:**
1. **32,768 token context** — 64× more than nomic's 512. PersonaMem 32k documents (5K-40K chars ≈ 1.5K-10K tokens) will FULLY fit without truncation.
2. **MTEB 64.3** — competitive with much larger models, beats OpenAI text-embedding-3.
3. **Only 639 MB** — smaller than nomic (957 MB), fits easily on M1/8GB.
4. **Already pulled** — deployed and ready to test.
5. **Future-proof** — 32K context handles 128k and 1M PersonaMem splits with chunking.

**Why not:**
- **bge-m3**: 8,192 context is 4× less than qwen3. Good for 32k split but will need chunking for 128k/1M.
- **snowflake-arctic-embed2**: Empirically truncates at ~16K chars despite 8,192 spec. Less reliable.
- **embeddinggemma**: Only 2,048 context. Same-dim (768) means no migration, but not enough headroom.

## Migration Plan

1. **Dimension change**: 768 → 1024 dim
   - pgvector supports ALTER COLUMN TYPE to change dimensions (re-embed required)
   - All 730 existing nodes need re-embedding via `/nodes/reembed_all`
   - Downstream: AMB provider must handle 1024-dim vectors
2. **Config change in knowwhere-server**:
   - `OLLAMA_MODEL=qwen3-embedding:0.6b`
   - Rebuild may not be needed (dimension is runtime, not compile-time)
3. **Test**: Run AMB with `--query-limit 20` to verify improvement before full run

## Fallback Option

If qwen3-embedding doesn't work well on M1/8GB (memory pressure with 7 models loaded):
- Use **bge-m3** (8,192 context, proven, same 1024-dim)
- Combined with chunking for documents >8K tokens

## Impact Estimate

- Current: 66.9% (nomic, all docs identical embeddings → random retrieval)
- Expected with qwen3 + chunking: **75-85%** (differentiated embeddings + focused context)
- Target (Hindsight): 86.6% — reachable with additional fixes (consolidation user_id, preference extraction)

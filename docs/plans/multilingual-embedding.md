# Multilingual Embedding Research — KnowWhere

## Problem Statement

KnowWhere currently uses `nomic-embed-text-v2-moe` for embeddings (768-dim). QA testing revealed **German queries return 0% precision**, indicating the model is effectively English-only despite Ollama's description. The North Star metric (92% Context Fidelity) is unachievable without multilingual support.

## Current Implementation

**File**: `src/embedding/provider.rs`

```rust
pub struct LocalOllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,  // defaults to "nomic-embed-text-v2-moe"
}

fn dimension(&self) -> usize { 768 }
fn name(&self) -> &str { "local-ollama" }
fn document_prefix(&self) -> &str { "search_document: " }
fn query_prefix(&self) -> &str { "search_query: " }
```

**Key observation**: Switching models only requires changing `OLLAMA_MODEL` environment variable. No code changes needed if dimension remains 768.

---

## Candidate Models Evaluated

### 1. nomic-embed-text-v2-moe (CURRENT)
- **Status**: Already in use
- **Dimensions**: 768 (with Matryoshka reduction to 256/512)
- **Languages**: Claimed ~100 languages
- **Size**: 958MB, 512 context
- **Problem**: QA shows 0% precision on German — likely insufficient multilingual capability despite claims

### 2. snowflake-arctic-embed2 ⭐ RECOMMENDED
- **Dimensions**: 1024 (supports MRL reduction to 128, 256, 512, 768)
- **Languages**: Multilingual (English + French, Spanish, Italian, German per benchmarks)
- **Size**: 1.2GB, 8K context
- **Performance**: SOTA on MTEB retrieval for multilingual (55.65 nDCG@10)
- **Pros**: 
  - Excels at both English AND multilingual retrieval
  - MRL support allows dimension reduction to 768 if needed
  - 8K context (vs 512 in nomic)
  - Actively maintained
- **Cons**: Slightly larger (1.2GB vs 958MB)

### 3. bge-m3
- **Dimensions**: 1024
- **Languages**: 100+
- **Size**: 1.2GB, 8K context
- **Performance**: Best MIRACL (69.20) but lower BEIR (48.80)
- **Pros**: Multi-functionality (dense, multi-vector, sparse retrieval)
- **Cons**: Lower overall retrieval performance than alternatives

### 4. granite-embedding:278m
- **Dimensions**: 768
- **Languages**: English, German, Spanish, French, Japanese, Portuguese, Arabic, Czech, Italian, Korean, Dutch, Chinese
- **Size**: 563MB, 512 context
- **Pros**: Exactly 768 dimensions, smaller model
- **Cons**: 512 context (same as current), limited language list

### 5. qwen3-embedding (0.6b or 4b or 8b)
- **Dimensions**: Up to 4096 (flexible)
- **Languages**: 100+
- **Sizes**: 639MB (0.6b), 2.5GB (4b), 4.7GB (8b)
- **Pros**: #1 on MTEB multilingual leaderboard (70.58 score)
- **Cons**: 4b/8b models are large; 0.6b may underperform

---

## Recommendation

### Primary: Switch to `snowflake-arctic-embed2`

**Rationale**:
1. Designed specifically for multilingual with strong German support
2. Outperforms alternatives on multilingual retrieval benchmarks
3. Maintains English performance (not sacrificed for multilingual)
4. MRL support allows adapting to 768-dim if needed
5. 8K context is 16x larger than current 512

**Implementation**: 
```bash
ollama pull snowflake-arctic-embed2
# Then set environment variable:
export OLLAMA_MODEL=snowflake-arctic-embed2
```

### Alternative: Keep nomic-embed-text-v2-moe BUT run both

Could use a routing strategy:
- English queries → nomic-embed-text-v2-moe
- German/other → snowflake-arctic-embed2

However, this adds complexity. Recommendation is to consolidate on arctic-embed2.

---

## Compatibility Check

**Question**: Is nomic-embed-text-v2-moe actually multilingual?

Based on Ollama documentation, nomic-embed-text-v2-moe claims multilingual support with 100+ languages and 1.6B training pairs. However, QA results showing 0% German precision suggest either:
1. The model isn't properly quantized/loaded in Ollama
2. The specific variant being used lacks proper multilingual weights
3. The 512 token context is limiting multilingual performance

**Action Item**: Before switching, verify nomic-embed-text-v2-moe is actually loaded correctly:
```bash
ollama list
ollama show nomic-embed-text-v2-moe
```

---

## Implementation Effort

| Task | Effort |
|------|--------|
| Pull new model | Small (~5 min) |
| Test with German queries | Small (~30 min) |
| Update OLLAMA_MODEL env var | Trivial |
| Verify dimension compatibility | Small |
| **Total** | **Small** |

---

## Timeline Estimate

- **Day 1**: Pull model + basic testing (1-2 hours)
- **Day 2**: Integration testing with German queries (2-4 hours)
- **Day 3**: Production deployment if tests pass

**Total**: 0.5 - 1 day effort

---

## Risks & Concerns

1. **Dimension mismatch**: arctic-embed2 uses 1024 by default, not 768. Must verify vector store compatibility or use MRL to reduce.
2. **Performance regression on English**: Low risk — arctic-embed2 maintains English performance
3. **Model size**: 1.2GB vs 958MB — marginal increase
4. **QA verification**: Must re-run German precision tests to validate improvement

---

## Next Steps

1. [ ] Run `ollama pull snowflake-arctic-embed2`
2. [ ] Test German query precision with new model
3. [ ] Verify dimension compatibility with vector store
4. [ ] Update OLLAMA_MODEL configuration
5. [ ] Re-run full QA suite
6. [ ] Measure Context Fidelity metric improvement

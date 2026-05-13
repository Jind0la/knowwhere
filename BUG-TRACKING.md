# KnowWhere Bug Tracking

## BUG-016: Vector Retrieval Score Collapse — Query Embedding Missing Prefix

| Field | Value |
|-------|-------|
| **ID** | BUG-016 |
| **Reported** | 2026-05-08 (debug session), Fixed 2026-05-13 |
| **Severity** | CRITICAL — all vector retrieval scores collapsed to ~0.03 |
| **Status** | ✅ FIXED |
| **Component** | `src/api/routes.rs` — `retrieve_fractal()` |
| **Root Cause** | Query embedding used raw `embed()` without `"search_query: "` prefix |

### Expected Behavior

Self-similarity test: store text X, query text X → score should be ~0.8–1.0.
General retrieval: semantically similar content should score >0.5, exact matches >0.8.

### Observed Behavior

**Before fix:** All retrieval scores collapsed to ~0.03 (random-noise level).
- Self-similarity: 0.032786883 (should be ~0.83)
- The `/embed` endpoint (which correctly used `embed_query()`) produced fundamentally different vectors than the retrieval pipeline.

### Reproduction

```bash
# 1. Store a node with known content
curl -s -X POST http://localhost:3737/store_external \
  -H 'Content-Type: application/json' \
  -d '{"pointer":"test://regression","content":"KnowWhere ist ein fractales Memory-System mit Vektor-Retrieval","memory_type":"semantic"}'

# 2. Query with exact same text — score should be ~0.8+
curl -s -X POST http://localhost:3737/retrieve_fractal \
  -H 'Content-Type: application/json' \
  -d '{"query_text":"KnowWhere ist ein fractales Memory-System mit Vektor-Retrieval","top_k":3,"include_debug":true}' | jq '.[0].score'

# Before fix: 0.032... (random)
# After fix:  0.8+ (correct)
```

**Reproduction script:** `scripts/repro-vector-bug.sh`

### Root Cause Analysis

**Two asymmetric bugs converged:**

#### Bug A: Query embedding used wrong method (THIS FIX)

`retrieve_fractal()` called `state.embedding.embed(text)` — the raw `EmbeddingProvider` trait method.

For asymmetric embedding models (`nomic-embed-text`, `snowflake-arctic-embed2`), document and query embeddings live in **different vector subspaces**:
- **Documents** are embedded with `"search_document: "` prefix → stored correctly via `embed_document()`
- **Queries** should be embedded with `"search_query: "` prefix → was using raw `embed()` (no prefix)

Using no-prefix vectors against document-prefix vectors → cosine similarity collapses to ~0.03.

**Fix:** Changed `state.embedding.embed(text)` → `embed_query(&*state.embedding, text)` at two locations:
- Line 2240: main query embedding
- Line 2362: contrastive query embedding in diversity mode

#### Bug B: `store_external` embedded pointer instead of content (Partially addressed)

`store_external()` embeds `&req.pointer` (URI string like `"test://knowwhere-intro"`) instead of the actual content. The `StoreExternalRequest` struct lacks a `content` field.

**Impact:** For external nodes where `pointer` is a URI and content differs, the stored vector represents the URI semantics, not the content. This is a separate issue tracked for future fix.

**Note:** The MCP adapter (`knowwhere-memory` MCP server) works around this by passing content in metadata. A proper fix requires adding `content: Option<String>` to `StoreExternalRequest`.

### Data Flow (Fixed)

```
Store:  content → clean_for_embedding() → embed_document() → "search_document: {text}" → Ollama → vector
Query:  text    → embed_query()           → "search_query: {text}"       → Ollama → vector
                                                                                        ↓
                                                                              cosine_similarity()
                                                                                        ↓
                                                                              score ≈ 0.8+ ✓
```

### Code Changes

```diff
# routes.rs line 2240
- state.embedding.embed(text).await.map_err(|e| { ... })?
+ embed_query(&*state.embedding, text).await.map_err(|e| { ... })?

# routes.rs line 2362
- if let Ok(cq_vector) = state.embedding.embed(cq_text).await {
+ if let Ok(cq_vector) = embed_query(&*state.embedding, cq_text).await {
```

### Regression Test

`test_embed_query_single_with_prefix` in `src/embedding/provider.rs`:
- Verifies `embed_query()` prepends `"search_query: "` prefix
- MockProvider now records individual `embed()` calls
- Catches any future regression where raw `embed()` is used for queries

### Prevention

1. **Linter rule (future):** Ban direct calls to `provider.embed()` in production code. Enforce `embed_document()` / `embed_query()`.
2. **Architecture:** `EmbeddingProvider::embed()` should be `#[doc(hidden)]` or renamed to discourage direct use.
3. **Self-similarity test:** CI should run a store+query self-check and assert score > 0.7.
4. **Type safety:** Consider `QueryVector` and `DocumentVector` newtypes to prevent mixing at compile time.

### Verification Checklist

- [x] Root cause identified (wrong embedding method, missing query prefix)
- [x] Fix applied at both call sites (main query + contrastive query)
- [x] Regression test added (`test_embed_query_single_with_prefix`)
- [x] Full test suite passes: 141 passed, 0 failed
- [x] Build succeeds (`cargo build --release`)
- [x] Changelog updated
- [x] Skill reference doc updated (`knowwhere-embedding-retrieval-debug.md`)
- [ ] Live reproduction test with Ollama (requires Ollama running)
- [ ] Deploy to production
- [ ] Self-similarity CI check added

### Related

- Debug session: 2026-05-08 (see `skills/software-development/systematic-debugging/references/knowwhere-embedding-retrieval-debug.md`)
- BUG-012 to BUG-015: Earlier bugs in consolidation, summarizer, sqlx types (see git history)

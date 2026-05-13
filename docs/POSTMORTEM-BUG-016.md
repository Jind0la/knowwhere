# Post-Mortem: BUG-016 Vector Retrieval Score Collapse

**Date:** 2026-05-13  
**Fix applied by:** Hermes (autonomous operator) / Nimar  
**Severity:** CRITICAL — all semantic retrieval was effectively broken

## What Happened

Every vector retrieval query via `POST /retrieve_fractal` returned scores at random-noise level (~0.03) regardless of semantic relevance. A self-similarity test (querying with the exact text that was stored) returned 0.032786883 instead of the expected 0.83+.

This meant KnowWhere's core value proposition — semantic memory retrieval — was non-functional. The system was returning essentially random results for all vector-based queries.

## Why It Happened

The `retrieve_fractal()` function embedded query text using `state.embedding.embed(text)` — the raw trait method without any prefix. However, all stored document vectors were embedded using `embed_document()` which prepends `"search_document: "`.

For asymmetric embedding models (`nomic-embed-text`, `snowflake-arctic-embed2`), documents and queries are embedded in **different vector subspaces** via dedicated prefixes:
- Documents: `"search_document: {text}"`
- Queries: `"search_query: {text}"`

Using a no-prefix vector against document-prefix vectors produces near-zero cosine similarity. This is by design — the model separates the subspaces to improve retrieval quality when used correctly. It destroys retrieval quality when used incorrectly.

**Root cause:** Two lines in `retrieve_fractal()` called the wrong method:

```rust
// Bug — raw embed(), no query prefix:
state.embedding.embed(text).await       // Line 2240
state.embedding.embed(cq_text).await    // Line 2362 (contrastive)

// Fix — proper query embedding with "search_query: " prefix:
embed_query(&*state.embedding, text).await
embed_query(&*state.embedding, cq_text).await
```

## Why It Wasn't Caught Earlier

1. **The `/embed` endpoint worked correctly** — it already used `embed_query()`. This created a false sense that embedding was working fine, when in reality the retrieval pipeline was using a different code path.

2. **Non-asymmetric models would be unaffected** — OpenAI's `text-embedding-3-small` and Grok's `v3-embedding` don't use prefixes. If anyone tested with a cloud provider, the bug would be invisible.

3. **BM25 fallback masked the issue** — For hybrid queries (text + vector), BM25 results filled the gap when vector scores collapsed. Pure vector queries were entirely broken.

4. **No self-similarity CI check** — The single most effective test (store X, query X, assert score > 0.7) was never automated.

## What We Fixed

1. **Fixed query embedding** (routes.rs L2240, L2362): `embed()` → `embed_query()`
2. **Added regression test** (provider.rs): `test_embed_query_single_with_prefix`
3. **Modified MockProvider** (provider.rs): Now records individual `embed()` calls
4. **Created reproduction script**: `scripts/repro-vector-bug.sh`
5. **Documented in BUG-TRACKING.md**: Full root cause, reproduction, prevention

## What We Verified

| Check | Status | Evidence |
|-------|--------|----------|
| Embedding model consistency | ✅ | Same `OLLAMA_MODEL` env var for store and query |
| Vector dimension consistency | ✅ | `LocalOllamaProvider::dimension()` = 768 for nomic-embed-text |
| Cosine similarity metric | ✅ | Uniform `cosine_similarity()` in `fractal_node.rs` |
| No index rebuild needed | ✅ | Stored vectors correct; only query vectors were wrong |
| Preprocessing consistency | ⚠️ | `clean_for_embedding()` used at store but not query — pre-existing, minor (natural language queries unaffected) |
| Full test suite | ✅ | 141 passed, 0 failed |
| Build | ✅ | `cargo build --release` succeeds |

## How We Prevent Recurrence

1. **Regression test** (`test_embed_query_single_with_prefix`): Verifies `embed_query()` always prepends the query prefix. Any future change that breaks prefix behavior will fail this test.

2. **Documentation** (`BUG-TRACKING.md`, `CHANGELOG.md`, debug reference): The root cause is documented with concrete line numbers and code examples.

3. **Reproduction script** (`scripts/repro-vector-bug.sh`): Can be run against a live server to verify the fix holds.

4. **Future architectural improvement (recommended):** Make `EmbeddingProvider::embed()` crate-private or rename it to discourage direct use. Enforce `embed_document()` / `embed_query()` at the type level (e.g., `QueryVector` / `DocumentVector` newtypes).

## Remaining Work

- [ ] **Deploy to production** — Commit and push changes, rebuild server binary, restart daemon
- [ ] **CI self-similarity check** — Add automated store+query+assert test to CI pipeline
- [ ] **`StoreExternalRequest.content` field** — Separate fix needed for BUG-016-B (content vs pointer embedding)
- [ ] **`clean_for_embedding()` at query time** — Apply same preprocessing to queries that's applied to documents (minor impact)

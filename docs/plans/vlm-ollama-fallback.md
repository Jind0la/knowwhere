# VLM Fallback Plan

## Recommendation

**Option C (OPENAI_API_KEY)** is the recommended immediate fix. The VLM worker and consolidation scheduler are already fully implemented for OpenAI — just set an environment variable. Cost is negligible (~$0.025/1M tokens, ~$0.10/day at typical consolidation rates).

**Option B (Ollama as VLM)** is the recommended long-term solution for offline/privacy-sensitive deployments. It avoids external API dependencies entirely and uses the same model family (nomic) already used for embeddings.

**Option A (truncation_fallback)** is a last resort — it is already partially implemented in the VLM worker error path but provides poor quality summaries.

---

## Option A: truncation_fallback

### Current State
The truncation fallback is **already implemented** in two places:
- `src/vlm/mod.rs:763-787` — `VlmWorker::truncation_fallback_text()` called when all VLM models fail inside `process_job()`
- `src/memory/tiered.rs:132-166` — `truncation_fallback_overview()` and `truncation_fallback_summary()` (never called)

The real problem: when `vlm_worker` is `None` (main.rs:214-216), `ConsolidationScheduler` skips enqueueing entirely (consolidation.rs:126-131) and does **not** call any fallback.

### Changes Needed

**File: src/scheduler/consolidation.rs**
- Lines 126-131 — In the `else` branch where `vlm_worker` is `None`, add fallback logic:
  - Call `truncation_fallback_overview()` on the raw content
  - Directly write the truncated L1 node to storage (bypassing the VLM worker)
  - This requires the scheduler to have access to the `MemoryStore` or equivalent to write summary nodes directly

**Complexity: Medium** — Requires understanding of storage layer and node creation in scheduler context.

### Pros
- No new dependencies
- Quick to implement
- Already exists in VLM worker's error path

### Cons
- No real summarization — just truncation at ~200 token boundary
- No semantic understanding
- L0 summary = just first sentence of truncated text
- Only works when VLM is completely unavailable (not when VLM is slow/unresponsive)
- Would need to duplicate node-writing logic in scheduler

---

## Option B: Ollama as VLM

### Current State
Ollama is **already in use** for embeddings via `LocalOllamaProvider` (src/embedding/provider.rs:245-302):
- Base URL: `OLLAMA_URL` env var, default `http://localhost:11434`
- Model: `OLLAMA_MODEL` env var, default `nomic-embed-text-v2-moe`
- Endpoint: `/api/embeddings` (not a chat endpoint)

### Key Constraint
Ollama does **not** have a batch/summarization API equivalent to OpenAI's `/v1/responses`. The options are:
1. Use `/api/chat` (chat completions) — not batch-oriented, processes one item at a time
2. Use `/api/generate` — not designed for chat-style summarization

### Changes Needed

**File: src/vlm/mod.rs**

1. **Line 178-186** — Add `Ollama` variant to `VlmModel` enum:
   ```rust
   pub enum VlmModel {
       Gpt5Nano,
       Gpt4oMini,
       Grok4Fast,
       Ollama,  // NEW
   }
   ```

2. **Lines 188-228** — Implement `Ollama` methods on `VlmModel`:
   - `fallback_chain()` — add Ollama to end of chain
   - `model_id()` — return `OLLAMA_MODEL` env var (e.g., `nomic-embed-text-v2-moe` or a chat model like `llama3.2`)
   - `base_url()` — return `OLLAMA_URL` env var
   - `timeout_secs()` — return 60 (Ollama is slower than cloud APIs)

3. **Lines 358-376** — In `call_model()`, add Ollama branch:
   - Extract Ollama API key from env (or allow no-key for local)
   - Build request to `/api/chat` with the standard chat completion format
   - Parse response from Ollama's `message.content` field

4. **Lines 232-250** — In `VlmConfig`:
   - Add `ollama_url: Option<String>` and `ollama_model: Option<String>`
   - `from_env()` — read `OLLAMA_URL` and `OLLAMA_MODEL`
   - `is_configured()` — also checks Ollama config

5. **File: src/main.rs:208-217** — Update VLM worker spawning:
   - Worker starts if any API key is configured (OpenAI, Grok, **or Ollama**)
   - Pass Ollama config to worker

**Complexity: Large** — Requires substantial changes to the VLM model chain, new API client code for Ollama's chat format, and testing.

### Pros
- Local, no external API dependency
- Consistent model family (nomic) for embeddings AND summarization
- No cost per token
- Privacy-preserving (all processing local)

### Cons
- Ollama has no batch API — must use chat completions (slower, one-at-a-time)
- Ollama is typically on a local machine — slower than cloud APIs
- Need to choose an appropriate chat model (llama3.2, mistral, etc.) — nomic-embed-text-v2-moe is embedding-only
- Large implementation effort
- Adding a new model to the fallback chain changes production behavior

---

## Option C: OPENAI_API_KEY

### Current State
The entire VLM worker and consolidation scheduler are **already implemented** for OpenAI. Just set the env var.

### Changes Needed

**File: None** — No code changes required.

Set environment variable:
```bash
export OPENAI_API_KEY=sk-...
```

Or in deployment, add to environment or `.env` file.

**Complexity: None** — Just configuration.

### Pros
- Zero code changes
- Already implemented and tested
- GPT-5-nano is $0.025/1M tokens — extremely cheap
- ~50ms latency, fast consolidation
- 3-stage fallback already built-in (GPT-5-nano → GPT-4o-mini → Grok-4-fast)
- Most reliable option (multiple providers in fallback chain)

### Cons
- External API dependency (internet required)
- Small cost per token (negligible at typical usage)
- API key management required

---

## Recommended Implementation Steps

### Immediate (Option C — 5 minutes)
1. Add `OPENAI_API_KEY` to deployment environment or `.env` file
2. Verify VLM worker starts: check logs for `"VLM summarization worker started"`
3. Verify consolidation scheduler enqueues jobs

### Long-term (Option B — if offline/private deployment needed)

**Phase 1: Infrastructure**
1. `src/vlm/mod.rs` — Add `Ollama` variant to `VlmModel` enum (line ~179)
2. `src/vlm/mod.rs` — Implement `model_id()`, `base_url()`, `timeout_secs()` for Ollama
3. `src/vlm/mod.rs` — Add Ollama API key and URL to `VlmConfig` (lines ~232-250)
4. `src/main.rs` — Update worker spawn condition to include Ollama (line ~210)

**Phase 2: Ollama API Client**
5. `src/vlm/mod.rs` — Add `call_model()` branch for Ollama using `/api/chat` endpoint
6. `src/vlm/mod.rs` — Add Ollama to `fallback_chain()` (line ~190-191)
7. Test with local Ollama instance running a chat model (e.g., `llama3.2`)

**Phase 3: Integration**
8. `src/vlm/mod.rs` — Wire Ollama into `summarize_with_fallback()` loop
9. Update documentation for `OLLAMA_URL` and `OLLAMA_MODEL` env vars

### Fallback Path (Option A — if all VLM unavailable)
10. `src/scheduler/consolidation.rs` — Add fallback logic in `else` branch (lines 126-131):
    - Fetch node content directly
    - Call `truncation_fallback_overview()` from tiered.rs
    - Write summary node directly to storage
    - Set parent_tier_id on source nodes

---

## Summary Table

| Option | Complexity | Cost | Quality | Implementation | Reliability |
|--------|-----------|------|---------|----------------|-------------|
| A: truncation | Medium | Free | Poor (raw truncation) | Already in worker | Already exists |
| B: Ollama | Large | Free (local) | Good (local LLM) | New code needed | Depends on local Ollama |
| C: OPENAI_API_KEY | None | ~$0.025/1M | Good (GPT-5-nano) | No changes | High (3-stage fallback) |

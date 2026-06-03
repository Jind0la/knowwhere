# Chunking Strategy for PersonaMem / KnowWhere

**Date:** 2026-05-07
**Context:** PersonaMem documents are 5K-60K chars. Even with 32K-token embedding model, documents >32K tokens need chunking. Current approach (one embedding per full document) is broken for any model with finite context.

## Competitor Analysis

| System | Chunking Approach |
|--------|------------------|
| **Hindsight** | Bank-based isolation (per-user), documents stored as-is. Relies on Cross-Encoder reranking for precision. |
| **Mem0** | `add()` stores individual facts/memories, NOT full conversations. Natural chunking by design — each memory is small. |
| **Standard RAG** | Fixed-size chunks (512-1024 tokens) with 10-20% overlap. Or semantic: sentence/paragraph boundaries. |

## Chunking Strategies Evaluated

### Strategy A: Fixed-Size (simplest)
- Split at N characters, with M-character overlap
- For qwen3: N=4000 tokens (~16K chars), M=200 tokens (~800 chars)
- ✅ Simple, predictable
- ❌ Cuts mid-sentence, loses context at boundaries

### Strategy B: Semantic (conversation-aware)
- Split at session boundaries (PersonaMem already has sessions!)
- Each session = one chunk
- Session boundaries = system turns in the conversation
- ✅ Natural boundaries, preserves context
- ❌ Uneven sizes (1K-40K chars), long sessions still truncated

### Strategy C: Hybrid (RECOMMENDED)
- **Tier 1**: Split at session boundaries (PersonaMem's existing split)
- **Tier 2**: For sessions >16K chars, split at turn boundaries (user/assistant messages)
- **Tier 3**: For individual turns >16K chars, fixed-size split
- ✅ Preserves semantic context where possible, falls back to size limits
- ✅ Works with all embedding models
- ❌ Slightly more complex to implement

## Recommended Configuration

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| **Strategy** | Hybrid (session → turn → fixed) | Best context preservation |
| **Max chunk size** | 16,000 chars (~4,000 tokens) | Safe margin for qwen3's 32K context |
| **Overlap** | None (session boundaries) | Natural boundaries don't need overlap |
| **Min chunk size** | 100 chars | Skip empty/trivial chunks |
| **Max chunks per persona** | 50 | Prevent context overflow |

## Where to Implement

**Option A: AMB Harness (knowwhere.py)** — RECOMMENDED for immediate fix
- Chunk in `ingest()` before calling `/store_external`
- Each chunk = separate external document with same user_id
- Pros: Quick, no server changes, testable immediately
- Cons: KnowWhere-specific, doesn't help other clients

**Option B: KnowWhere Server (/store_external)** — Long-term
- Server auto-chunks large documents at ingest time
- Pros: Universal, all clients benefit
- Cons: More complex, requires server rebuild + deploy

**Decision: Start with Harness (Option A), migrate to Server (Option B) later.**

## Harness Implementation Sketch

```python
def _chunk_document(self, content: str, max_chars: int = 16000) -> list[str]:
    """Split PersonaMem session into chunks at turn boundaries."""
    # Split at [SYSTEM], [USER], [ASSISTANT] markers
    turns = re.split(r'\n(?=\[(?:SYSTEM|USER|ASSISTANT)\])', content)
    chunks = []
    current = ""
    for turn in turns:
        if len(current) + len(turn) > max_chars and current:
            chunks.append(current.strip())
            current = turn
        else:
            current += "\n\n" + turn if current else turn
    if current.strip():
        chunks.append(current.strip())
    return chunks
```

## Impact Estimate

- Current: 5 massive chunks (28K tokens each) → LLM drowns in noise
- With chunking + qwen3: 10-20 focused chunks (2K-4K tokens each) → LLM gets relevant signal
- Expected improvement: +10-15pp on inference-heavy question types

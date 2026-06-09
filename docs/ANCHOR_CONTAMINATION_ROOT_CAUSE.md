# Anchor Contamination — Root Cause Analysis & Fix

**Date:** 2026-05-15
**Session:** Telegram DM with Nimar
**Status:** FIXED — 70% honest plateau achieved

## The Bug

Session Anchors in the KnowWhere AMB provider were leaking cross-persona data:

1. **Claims ingestion path** (`_ingest_single`) stored a "session anchor" node with `content = doc.content` — the FULL raw PersonaMem session text
2. This included `[SYSTEM]` blocks describing ALL personas in the session, e.g.:
   ```
   [SYSTEM] Current user persona: Name: Arjun Patel
   Gender Identity: Transgender Male
   Racial Identity: South Asian
   Arjun Patel, born in 1981, is an avid underwater hockey enthusiast...
   ```
3. The anchor had **no `user_id` in metadata** → retrieved for every query regardless of persona
4. The fallback path (`_ingest_fallback`) correctly filtered `[SYSTEM]` blocks, but the claims path did not

## Evidence

In benchmark v6 (75%), Kanoa Manu's context contained:
```
## Kanoa Manu: Evolution

**Initially:**
  ✓ [SYSTEM] Current user persona: Name: Arjun Patel
  Gender Identity: Transgender Male
  Racial Identity: South Asian

  Arjun Patel, born in 1981, is an avid underwater hockey enthusiast...
```

This appeared in the Evolution section because the anchor's full session text was retrieved and its turns were categorized as "Initially" (turn_index 0).

## Why It Looked Like 75%

The contamination paradoxically HELPED accuracy:
- The unfiltered anchor provided massive context volume (6182 chars vs 2254 without)
- `suggest_new_ideas` queries benefited from narrative depth
- The LLM (Kimi K2.6) was smart enough to mostly ignore the cross-persona noise while using the extra context

But this was **measurement-invalid**: correct answers were achieved using another persona's data.

## Benchmark Comparison

5 runs on PersonaMem 32k (20 queries, 30 docs), all with same setup:

| Run | Anchor Strategy | `facts` | `prefs` | `reasons` | `suggest` | `evol` | **Total** | Valid? |
|-----|----------------|---------|---------|-----------|-----------|--------|-----------|--------|
| v6 | Full-text, no filter, no user_id | 3/4 | 4/4 | 3/3 | 3/6 | 2/3 | **75%** | ✗ contaminated |
| nosystem-v2 | Full-text* + ingestion regex | 3/4 | 4/4 | 3/3 | 2/6 | 3/3 | **70%** | ~ (partial) |
| compact-anchor | Claims summary, filtered, user_id | 3/4 | 4/4 | 3/3 | 2/6 | 2/3 | **70%** | ✓ |
| windowed-anchor | First 3000 chars, filtered, user_id | 2/4 | 4/4 | 3/3 | 3/6 | 2/3 | **70%** | ✓ |
| noanchor | No anchor | 3/4 | 4/4 | 3/3 | 1/6 | 2/3 | **65%** | ✓ |

*Fallback-path anchor only, claims-path still had unfiltered anchors

## The Honest Plateau: 70%

Three clean approaches converge at 70%:
- `provide_preference_aligned_recommendations`: **100%** (always perfect)
- `recalling_the_reasons_behind_previous_updates`: **100%** (always perfect)
- `track_full_preference_evolution`: **67%** (consistently 2/3)
- `recall_user_shared_facts`: **50-75%** (varies with anchor strategy)
- `suggest_new_ideas`: **33-50%** (needs narrative depth)

The bottleneck is `suggest_new_ideas` — atomic claims can't capture the narrative arc needed for creative suggestion tasks.

## The Fix

**Windowed Anchor Strategy** (chosen implementation):

```python
# Filter [SYSTEM] blocks (same regex as _ingest_fallback)
SYSTEM_BLOCK = re.compile(
    r'\n?\[SYSTEM\].*?(?=\n\[(?:USER|ASSISTANT|SYSTEM)\]|\Z)', re.DOTALL
)
cleaned = SYSTEM_BLOCK.sub('', doc.content).strip()

# Take first 3000 chars — enough for narrative, not so much it dilutes embedding
windowed = cleaned[:3000]

# Store with user_id for persona-scoped retrieval
"metadata": {
    "user_id": doc.user_id,
    "session_type": "anchor",
    ...
}
```

**Why 3000 chars?** Empirically determined: compact claims-summary (800 chars) recovered 2/6 on suggest_new_ideas; windowed session text (3000 chars) recovered 3/6 — matching the contaminated baseline on that category.

## What We Learned

1. **Auth middleware is always active** — the "auth disabled (dev mode)" log in main.rs is misleading. Without `KNOWWHERE_API_KEY` set, ALL protected routes (including `/store_external` and `/retrieve_fractal`) return 401. `/health` works because it's outside the `protected` router. Always start the server with `KNOWWHERE_API_KEY=kw_testkey_12345`.

2. **Session anchors matter** — removing them entirely drops accuracy from 70% to 65%. They provide narrative context that atomic claims miss, especially for `suggest_new_ideas` queries.

3. **Full session text is too noisy** — 16K+ character nodes produce weak embeddings that match queries poorly. 3000 chars is the sweet spot.

4. **`suggest_new_ideas` is the hardest category** — consistently 33-50% regardless of retrieval quality. Atomic fact extraction fundamentally can't capture the "why" and "how it evolved" that creative suggestions need.

## Next Steps (Beyond Current Architecture)

To break through the 70% plateau:
- **Richer claim extraction**: extract "narrative claims" alongside atomic facts (e.g., "Kanoa's music journey: started blending Pacific sounds → criticized for reviews → seeking new expression")
- **Better answer model**: Kimi K2.6 struggles with MCQ reasoning; a model with stronger reasoning might push suggest_new_ideas higher
- **Query-time synthesis**: Use Gemini/Grok to synthesize a persona narrative from retrieved claims at query time (the "Dreaming" pattern)

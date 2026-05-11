# KnowWhere Retrieval Fix Plan — Synthesis

**Date:** 2026-05-07
**Status:** Research complete, implementation sequence defined

## Executive Summary

KnowWhere's AMB score (66.9%) is capped by a **fundamental embedding truncation bug**: nomic-embed-text-v2-moe's 512-token context truncates PersonaMem documents to ~2000 characters, making all documents from the same persona indistinguishable. The fix requires three coordinated changes: (1) switch embedding model to qwen3-embedding:0.6b (32K context), (2) chunk documents at turn/session boundaries, (3) fix consolidation to propagate user_id. Combined impact: estimated **66.9% → 82-88%**.

## Findings Summary

| Task | Key Finding | Impact |
|------|------------|--------|
| T1: Embedding Models | **qwen3-embedding:0.6b** — 32K context, 64.3 MTEB, 639MB. 64× more context than nomic. | Foundation fix |
| T2: Chunking Strategy | Hybrid chunking (session → turn → fixed) with 16K char max. PersonaMem already split by sessions. | +10-15pp |
| T3: user_id Propagation | 192 decision + 29 semantic nodes have NULL user_id → invisible to queries. Copy from parent. | +5-8pp |
| T4: Consolidation Trigger | 0/390 import nodes consolidated. Source filter likely excludes 'import'. Fix after chunking. | +3-5pp (combined with T3) |

## Impact × Effort Matrix

| Fix | Impact (pp) | Effort (h) | Risk | Dependencies | Priority |
|-----|-----------|-----------|------|-------------|----------|
| **Switch to qwen3-embedding:0.6b** | +15-20 | 2-3 | Medium (dim migration 768→1024) | None | **P0** |
| **Hybrid Chunking in Harness** | +10-15 | 1-2 | Low | None | **P0** |
| **user_id in Consolidation** | +5-8 | 1-2 | Low (small code change) | P0 fixes done | P1 |
| **Consolidation on Import Nodes** | +3-5 | 1-2 | Medium | Chunking + user_id fix | P2 |

## Implementation Sequence

```
Phase 1 (P0 — Foundation)
├── 1a: Switch embedding model → qwen3-embedding:0.6b
│   Config change, re-embed all 730 nodes, dimension migration
│   Verify: AMB --query-limit 20 → differentiated retrieval scores
│
├── 1b: Implement hybrid chunking in knowwhere.py
│   Split PersonaMem sessions at turn boundaries, max 16K chars
│   Verify: each persona gets 5-15 chunks instead of 2-5 massive docs
│
└── 1c: Re-run AMB with both fixes
    Expected: 75-82%

Phase 2 (P1 — Consolidation)
├── 2a: Fix user_id propagation in consolidation
│   Copy user_id from parent import node to child decision/semantic nodes
│   Re-consolidate import nodes
│
└── 2b: Re-run AMB
    Expected: 80-85%

Phase 3 (P2 — Polish)
├── 3a: Fix consolidation trigger to include import nodes
│   (Only needed if P0+P1 fixes don't activate consolidation naturally)
│
└── 3b: Final AMB run
    Expected: 82-88%
```

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| qwen3 slower than nomic on M1 | Medium | Test latency first. Fallback: bge-m3 (8192 context) |
| 1024-dim migration breaks existing data | Low | pgvector handles dimension changes. Backup DB first. |
| Chunking loses cross-session context | Low | LLM prompt connects chunks from same user_id |
| Consolidation user_id fix has side effects | Low | Isolated change — only affects new child nodes |

## Estimated Timeline

- Phase 1: 4-6 hours (model switch + chunking + test)
- Phase 2: 2-3 hours (user_id fix + re-consolidation + test)
- Phase 3: 1-2 hours (trigger fix + final test)

**Total: 7-11 hours to 82-88% AMB.**

## Open Questions

1. Does qwen3-embedding:0.6b really deliver 32K context on Ollama/M1? → Test empirically before full migration
2. How does chunking affect the AMB gold answer evaluation? → Need to verify AMB harness compatibility
3. Should we keep nomic as fallback during migration? → Yes, keep both configured

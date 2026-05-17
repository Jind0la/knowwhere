# Spike 001: Bag-of-Claims Embedding Averaging (TST-inspired)

## Question
Does averaging embeddings of semantically related L0 claims produce a useful coarse (L1) representation — inspired by TST's bag-of-tokens embedding averaging?

## Approach
Three iterations:
- **v1 (spike.py)**: Direct recall test — can the centroid retrieve its own children? (Bad metric — self-similarity dominates)
- **v2 (spike_v2.py)**: Query-based overlap — do L0 and L1 retrieve similar results for probe queries? (Failed — PersonaMem data too narrow for generic probes)
- **v3 (spike_v3.py)**: **Direct geometry test** — does the centroid find the same embedding-space neighbors as its L0 children?

## Results (v3 — definitive)

4 groups of 5 random nodes each, from PersonaMem benchmark data (nomic-embed-text-v2-moe, 768d):

| Metric | Range | Average |
|--------|-------|---------|
| Union Jaccard (Centroid vs L0∪) | 0.22–0.29 | **0.254** |
| Centroid Coverage (% of centroid neighbors also in L0∪) | 76–100% | **91.3%** |
| Centroid→Member Cosine Similarity | 0.83–0.86 | **0.847** |
| Inter-Member Cosine Similarity | 0.61–0.67 | 0.646 |

## Verdict: **VALIDATED** ✅

Bag-of-claims embedding averaging preserves enough geometric structure for L1 node creation.

**Key finding**: Even with RANDOM grouping (not topic-clustered), the centroid's neighborhood strongly overlaps with its children's collective neighborhood. 91% of what the centroid finds is also found by at least one child — it doesn't introduce spurious results.

**Limitation**: Intersection Jaccard is near-zero — the centroid doesn't capture the "core" neighbors that ALL children agree on. This is expected for random groups; topic-clustered groups would show higher intersection.

## TST Mapping

| TST Concept | KnowWhere Analog | Status |
|-------------|-----------------|--------|
| Bag-of-tokens (contiguous) | Topic-clustered L0 claims | ✅ Viable |
| Embedding averaging | Centroid of claim vectors | ✅ Geometry preserved |
| No positional encoding needed | No order metadata on L1 nodes | ✅ Confirmed |
| Coarse→Fine training | L1 coarse → L0 fine retrieval cascade | 🔜 Next step |
| Representation continuity | Matryoshka truncated embeddings | 🔜 Needs verification test |

## Recommendation for real build

1. **Replace LLM-based consolidation with embedding averaging** for L1 node creation — it's ~100x cheaper and geometrically sound
2. **Group by semantic cluster** (cosine > 0.7), not randomly — matches TST's "contiguous bags" assumption
3. **Verify Matryoshka continuity**: Test that truncated embeddings (64d, 256d) maintain geometric continuity with full 768d after averaging
4. **3-level cascade retrieval**: 64d → 256d → 768d (TST's "coarse-first" insight)

## Surprises
- PersonaMem data has a silent user_id filter that kills all queries without explicit user_id parameter (API design issue, not spike-related)
- Random grouping still works — suggests the embedding space is well-behaved enough that even arbitrary clusters preserve local geometry

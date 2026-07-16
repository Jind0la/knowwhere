# Titans / MIRAS Surprise Metric — Research Synthesis

**Date:** 2026-07-16
**Sources:** Behrouz et al., "Titans: Learning to Memorize at Test Time" (arXiv:2501.00663); Behrouz et al., "It's All Connected: MIRAS Framework" (arXiv:2504.13173); Google Research Blog (Dec 2025); 6 independent analyses

**⚠️ Correction:** The arXiv ID in the task body (2411.06886) is a math paper on differential geometry. The correct ID for Titans is **2501.00663**. MIRAS is **2504.13173**.

---

## Summary

Titans (Google Research, Dec 2024) introduces a neural long-term memory module that learns to memorize at test time. Its core mechanism is a **surprise metric**: the model computes the gradient of an associative memory loss (||memory(k_t) - v_t||²₂) with respect to the input. This gradient IS the surprise signal — large gradients mean "this input violates expectations, store it"; small gradients mean "routine, skip." To avoid missing important tokens after a single surprising event, the paper refines this into two components: **momentary surprise** (current gradient) and **past surprise** (a momentum term S_t = η_t·S_{t-1} - θ_t·∇ℓ, equivalent to gradient descent with momentum). A separate **forgetting gate** α_t acts as weight decay: ℳ_t = (1-α_t)·ℳ_{t-1} + S_t, adaptively clearing stale information. The MIRAS framework (2025) generalizes this: every sequence model is an associative memory defined by 4 choices — memory architecture (vector/matrix/MLP), attentional bias (L2/dot-product/Huber), retention gate (forgetting), and learning algorithm (SGD/momentum). Titans is one instance in this framework using deep MLP memory + L2 attentional bias + adaptive weight decay + momentum-based SGD.

---

## CLAIMS vs PROOFS Table

| # | Claim | Evidence from Paper | Confidence |
|---|-------|-------------------|------------|
| 1 | Surprise = gradient of loss w.r.t. input | Eq. 8: ℳ_t = ℳ_{t-1} - θ_t·∇ℓ(ℳ_{t-1}; x_t). Explicitly stated in §3.1 "Learning Process and Surprise Metric." | **PROVEN** — core definition |
| 2 | Surprise determines keep vs. discard: high gradient → store, low → skip | Google Research blog: "High surprise...must be prioritized for permanent storage. Low surprise...can safely skip." Paper §3.1: "an event that violates the expectations...is more memorable." | **CLAIM** — mechanism described qualitatively, no ablation isolating surprise vs. random retention |
| 3 | Past + momentary surprise (momentum) prevents missing follow-up tokens after one surprising event | Eq. 9-10: S_t = η_t·S_{t-1} - θ_t·∇ℓ. Paper §3.1: "the gradient can become extremely small after several surprising steps...To improve...we break the surprise metric into past surprise and momentary surprise." | **CLAIM** — mechanism is defined, but no controlled experiment proves it fixes the "local minima after big surprise" failure mode |
| 4 | Forgetting gate α_t is equivalent to weight decay | Eq. 13-14: ℳ_t = (1-α_t)·ℳ_{t-1} + S_t. Paper §3.1: "we show that this weight decay mechanism is closely related to the gating mechanism in modern RNNs." Also §C: mathematical equivalence to Gated DeltaNet + weight decay. | **PROVEN** — mathematical equivalence demonstrated |
| 5 | Deep MLP memory (≥2 layers) outperforms linear/vector memory | §5.5 "The Effect of Deep Memory": ablation shows deeper MLP → lower perplexity on language modeling. | **PROVEN** — supported by ablation experiments (perplexity improvement, better scaling) |
| 6 | Titans outperforms Transformers on long-context (>2M tokens) tasks | BABILong benchmark: Titans (MAC)-FT > GPT-4, Mamba, DeltaNet, all baselines. Section 5. | **PROVEN** — benchmark results reported, but peer review caveat: paper published in NeurIPS 2025? Community discussion flagged need for independent reproduction |
| 7 | MIRAS generalizes ALL sequence model architectures to 4 design choices | Table 1 in MIRAS paper: maps RetNet, Transformer, LA, Linear Transformer, Mamba, DeltaNet, Titans, TTT to the 4 dimensions. | **CLAIM** — mapping is conceptually coherent, but "everything is X" frameworks often have edge cases. Table covers 10+ models. |
| 8 | Titans can scale to 2M context with linear inference cost | Paper §5: "Titans scale to larger than 2M context window size." But inference STILL requires test-time gradient computation on the memory module. | **PARTIAL** — YES for throughput vs. Transformers (linear vs quadratic), but NO on being "free" — test-time training adds constant-factor overhead |
| 9 | Surprise metric automatically discovers what matters (no human rules needed) | Google blog: "No human engineer had to define 'here's what matters.' The architecture discovered it." | **CLAIM** — the gating parameters (α_t, θ_t, η_t) ARE learned from data, so in that sense it's automatic. But "what matters" == "what has high gradient under L2 associative memory loss" — not necessarily aligned with human notions of importance |
| 10 | Titans is practical for always-on agent memory | Paper targets language modeling (batch processing of sequences). Test-time weight updates mean every forward pass computes gradients. 170M-780M total params. | **CLAIM** — no evidence in paper. Gradient computation per token at inference time is expensive for always-on agents. This is NOT demonstrated. |

---

## KnowWhere Cross-Reference

| Titans/MIRAS Concept | KnowWhere Equivalent | Gap |
|---------------------|---------------------|-----|
| Surprise = gradient w.r.t. input | Salience = base × explicit × decision × recall × emotional × decay | Titans' surprise is learned/automatic from prediction error; KnowWhere's salience is hand-tuned multiplicative. KnowWhere has MORE explicit control but LESS automation. |
| Past + momentary surprise (momentum) | UCB exploration (score = usefulness + c·√(ln(total)/impressions)) | Both use temporal momentum. UCB is exploration-focused; Titans' momentum is surprise-propagation. Different purpose. |
| Forget gate α_t = weight decay | Decay factor 0.95^days in salience | Same concept! KnowWhere's decay is fixed-rate; Titans' α_t is data-dependent (learned). |
| Deep MLP memory | Vector embeddings + HNSW indices | Fundamentally different: Titans compresses info INTO model weights; KnowWhere stores EXTERNALLY and retrieves. |
| Associative memory loss: ||ℳ(k) - v||²₂ | Embedding similarity (cosine/HNSW) + Cross-Encoder reranking | Titans learns mapping from scratch each sequence; KnowWhere uses pre-trained embeddings. |
| MIRAS: "everything is 4 choices" | KnowWhere: 5-process architecture (Ingest/Subconscious/DeepRecall/Reflection/Sleep) | MIRAS is a framework for designing seq models; KnowWhere is a system architecture for agent memory. Different levels of abstraction. |
| Test-time training | No test-time training (fixed embeddings) | Titans pays gradient cost but gets adaptation; KnowWhere pays storage cost but gets zero-shot retrieval. Trade-off. |

---

## Actionable Recommendation for KnowWhere

**Adopt the "past + momentary surprise" decomposition pattern for KnowWhere's salience model, but using embedding-space prediction error instead of gradient descent.**

Concretely: Replace KnowWhere's current fixed-rate salience decay (0.95^days) with a two-component signal:
1. **Momentary salience boost**: When a new memory's embedding cosine-similarity to the nearest existing memory is LOW (i.e., it's "surprising" relative to what's stored), apply a 2-5× multiplier to its initial salience.
2. **Past salience momentum**: Recent similar surprises propagate into the current salience update, preventing the "one surprising event overshadows its context" failure mode.
3. No gradient computation needed — this is pure vector similarity, cheap relative to Titans' approach. This bridges the gap between KnowWhere's explicit salience model and the automatic surprise detection that makes Titans compelling, without requiring test-time model training.

**Effort:** ~100-150 LOC in consolidation/salience pipeline. ~4h implementation. Zero infrastructure changes.

**What NOT to adopt:** Test-time gradient-based memory updates. KnowWhere's pointer-first architecture (embeddings point to original text; retrievals are vector search, not model weight adaptation) is fundamentally incompatible with Titans' "compress info into MLP weights" approach. Titans solves a different problem (sequence modeling with a single model), not agent memory across sessions.

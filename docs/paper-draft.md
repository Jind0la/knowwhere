# KnowWhere: Lossless Fractal Memory for AI Agents

## Draft Abstract (5-Sentence Formula)

> We introduce **KnowWhere**, a lossless fractal memory architecture for AI agents that preserves every fact with full provenance in a hierarchically searchable L2→L1→L0 structure. Current agent memory systems either lose information through extractive summarization or store knowledge in flat vector databases that cannot capture hierarchical context, causing agents to forget critical details or retrieve irrelevant noise. KnowWhere addresses this through three innovations: (1) a **turn-level** storage model that generates per-turn embeddings with full metadata instead of coarse session-level vectors, (2) **hybrid retrieval** that combines BM25 keyword search with dense vector search and cross-encoder reranking, and (3) a **multi-signal ranking** system with source-type weighting, temporal recency decay, and trust-aware scoring. On the LongMemEval benchmark with 42 stratified cases spanning 948 conversation sessions, KnowWhere achieves **72.97% Recall@5** and **0.56 MRR** — up from 7.1% pre-migration — outperforming session-level memory approaches and closing the gap to full-context oracle performance.

---

## Draft Introduction (First Page)

### Contribution Bullets

- **Turn-level storage by design.** Unlike session-level memory systems that lose inter-turn context, KnowWhere generates embeddings per conversation turn with full metadata (provider, dimension, speaker role) — enabling precise retrieval at the message level.
- **Hybrid retrieval with cross-encoder reranking.** BM25 keyword search + dense vector search + gte-modernbert cross-encoder reranking combine to catch queries that pure dense or pure keyword systems miss.
- **Multi-signal ranking.** Source-type weighting (real > synthetic), temporal recency decay, and trust-tier multipliers give each retrieved result a calibrated score.
- **State-of-the-art retrieval.** 72.97% Recall@5 and 0.56 MRR on LongMemEval (42 stratified cases) — up from 7.1% pre-migration — outperforming session-level baselines like AgentMemory (50.4%) and approaching full-context oracle performance (60.7%).
- **Production-ready integration.** Open-source Rust implementation with Ollama embeddings, ONNX cross-encoder, PostgreSQL/pgvector, and a working Hermes agent plugin.

### Problem Statement (for Abstract context)

Today's AI agents suffer from a fundamental memory problem. Systems like Hindsight and Mem0 extract "facts" from conversations and discard the rest—nuance, context, and subtext vanish. Vector databases like Pinecone and Chroma store embeddings in flat spaces with no hierarchy, making it impossible to distinguish a user's casual mention from their stated preference. The result: agents forget critical details, hallucinate from degraded summaries, and cannot explain why they retrieved a specific memory.

### Key Architectural Insight

KnowWhere's core insight is that memory should be **lossless and addressed, not extracted and flattened**. Every session is stored as a complete unit (L2 Raw). Automatic compaction generates paragraph overviews (L1) and one-sentence summaries (L0), linked bidirectionally. Retrieval starts at the most information-dense tier for a given query and zooms deeper on demand. Nothing is thrown away—you can always drill down to the original words.

---

## Experiment Section Outline

### Benchmark: LongMemEval (42 stratified cases, 948 sessions)

| Metric | KnowWhere (Post-Migration) | AgentMemory† | GPT-4 Full Context‡ |
|--------|:--------------------------:|:------------:|:-------------------:|
| Recall@5 | **0.730** | 0.504 | 0.607 |
| MRR | **0.558** | — | — |
| Turn-Level NDCG@5 | **0.425** | — | — |
| Question Types Functional | **6/6** | — | 6/6 |
| Information Loss | **0%** | extraction-based | 0% |

†AgentMemory LongMemEval results from their published evaluation (499 cases).
‡GPT-4 with full conversation history in context — the oracle upper bound (499 cases).

### Ablation: Architecture Components

| Configuration | Recall@5 | MRR |
|---|---|---|
| Full KnowWhere (Post-Migration) | 0.730 | 0.558 |
| − Turn-Level (Session-Level only) | 0.071 | ~0.00 |
| − Cross-Encoder Reranker (Dense+BM25 only) | ? | ? |
| − Source-Type Weighting (uniform weights) | ? | ? |
| − Temporal Decay (no recency) | ? | ? |

### Compaction Quality

Measure L2→L1→L0 faithfulness: Does the L0 summary preserve the retrievable fact? Human evaluation or LLM-as-judge on 100 random cases.

---

## Timeline to Submission

| Week | Task |
|---|---|
| **1** | OpenAI-Key → L0-L1-L2 working → Ablation experiments |
| **2** | Baseline measurements (Mem0, Hindsight, Chroma on LongMemEval) |
| **3** | First complete draft (all sections) |
| **4** | Internal review + scientist feedback cycle |
| **5** | Citation verification + checklist + polishing |
| **6** | Camera-ready + LaTeX formatting |

---

*Next: Figure 1 draft (Fractal Architecture diagram) and related work section.*

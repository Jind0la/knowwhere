# Entity Graph Layer — Spike Document

**Status:** Research Spike
**Date:** May 20, 2026
**Source:** H-Mem Paper (arXiv 2605.15701), Section 2B
**Related:** `docs/HMEM_PAPER_ANALYSIS.md`, `docs/THEORETICAL_FOUNDATIONS.md`

---

## 1. Executive Summary

**Recommendation: Implement.** The entity graph layer is a medium-effort, architecture-extending addition that fills one of KnowWhere's 3 key gaps identified against H-Mem. The existing `Relation` struct on `FractalNode` is an unused seam that makes integration natural — the data model already supports cross-cutting graph edges; the code just never populates them.

---

## 2. H-Mem Paper Findings — KG+Tree > Tree Alone

### 2.1 What H-Mem Proves (Section 2B)

H-Mem combines two structures:

| Structure | Role | KnowWhere Status |
|-----------|------|------------------|
| **Temporal-Semantic Tree (T)** | Models memory evolution from L1 (day fragments) through L4 (year summaries). Consolidation merges nodes above similarity threshold α_l. | **Implemented**: Fractal L0→L1→L2 hierarchy, Dream Pipeline consolidation |
| **Knowledge Graph (G)** | Captures entity-centered information across time periods. Nodes = normalized entities (Persons, Locations, Organizations). Edges = extracted relations. Entities map back to source tree leaves (bidirectional). | **Not implemented**: `relations` field exists but is always empty |

**Ablation results** (from H-Mem paper, reproduced in `HMEM_PAPER_ANALYSIS.md` Section "Ablation Findings"):

1. **Removing Tree** → Largest performance drop (tree is critical)
2. **Removing KG** → Moderate but meaningful drop (graph handles multi-hop queries)
3. **Removing Robustness (R)** → Significant degradation

The KG is not the most important component (tree is), but it provides a **complementary signal** that the fractal tree alone cannot produce. H-Mem's key insight: the KG is bidirectional — seed entities found in the graph point back to source leaves in the tree for evidence retrieval.

### 2.2 Why Pure Vector is Provably Insufficient

H-Mem's analysis in Section 2B confirms what KnowWhere's THORETICAL_FOUNDATIONS.md already identifies:

> A vector similarity search retrieves "similar" content but has no explicit concept of _who did what to whom_. Multi-hop questions like "What did the user decide about X in March after talking to Y?" require entity-level reasoning that pure cosine similarity cannot deliver.

The tree gives temporal organization. The graph gives entity-centric organization. Together they handle queries that require _both_ dimensions.

### 2.3 Competitive Landscape

| System | Graph Approach | KnowWhere Differentiation |
|--------|---------------|--------------------------|
| **H-Mem** | Bidirectional KG + Tree, LLM entity extraction | KnowWhere's fractal retrieval (multi-perspective) is richer than H-Mem's single-path scoring |
| **Mem0** | Vector + Graph + KV, automatic extraction | KnowWhere's trust tiers and cross-encoder provide provenance-aware scoring |
| **Zep** | Temporal Knowledge Graph, fact evolution tracking | KnowWhere's 6 memory types + conflict detection handle contradictions better |
| **Cognee** | Document-to-Graph from unstructured docs | KnowWhere's turn-level granularity is finer than document-level |

KnowWhere does NOT need to replicate H-Mem's full KG. A **lightweight entity graph** that sits alongside the fractal tree is architecturally sufficient and avoids H-Mem's main drawback (LLM extraction cost at every write).

---

## 3. Entity Extraction from FractalNodes

### 3.1 Extraction Points (3 Options)

| Option | When | What | Cost |
|--------|------|------|------|
| **A: Inline regex extraction** | At storage time (`store_session`, `store_external`) | Regex-based: Persons (capitalized names), Locations (preposition "in/at"), Organizations (acronyms, Inc.) | ~0 cost (regex, no LLM). Already partially done in `fact_extraction.rs` |
| **B: Consolidation-time LLM extraction** | During Dream Pipeline consolidation | LLM extracts entities + relations from cluster content | ~1 LLM call per cluster (already done in VLM summarizer `Detailed` mode) |
| **C: Post-hoc batch extraction** | Scheduled, asynchronous | LLM processes all unprocessed nodes | ~1 LLM call per node (expensive) |

**Recommendation: Option A (inline) + Option B (consolidation-time).**

The VLM summarizer in `src/vlm/mod.rs` already prompts for "Entities, timestamps, relationships" in `SummaryContext::Detailed` mode (line 170). The LLM _already generates_ this information — it's just discarded because no code stores it.

### 3.2 What to Extract

Modeling entity extraction after H-Mem's three entity classes, extended with KnowWhere-specific types:

| Entity Type | Detection Method | Examples |
|------------|------------------|----------|
| **Person** | Regex (capitalized names) + LLM disambiguation | "Nimar", "Jiawei Yu" |
| **Project** | Regex (proper nouns in context) + LLM | "KnowWhere", "H-Mem", "petgraph" |
| **Technology** | Regex (acronyms, versions) + LLM | "Rust", "PostgreSQL", "Ollama" |
| **Concept** | LLM extraction only | "fractal retrieval", "pointer-first architecture" |
| **Decision** | Already tracked as `MemoryType::Decision` — reuse! | "DECISION: migrate from Docker to native macOS" |

### 3.3 Normalization Strategy

H-Mem normalizes and disambiguates entities (e.g., "Nimar" vs "the user" vs "you" all map to the same entity node). For KnowWhere:

- **Dedup by canonical form**: First LLM extraction → canonical name. Subsequent regex hits match against canonicals.
- **Simple disambiguation**: Entity + context window → if confidence < 0.7, flag for review (don't auto-merge).
- **Merge on similarity**: Two entities with cos_sim > 0.95 across their embedding → likely same entity → prompt LLM to confirm.

This is less aggressive than H-Mem's full disambiguation pipeline but sufficient for a v1.

---

## 4. petgraph Integration Sketch

### 4.1 Why petgraph

The `petgraph` crate (v0.8.3, 3M+ downloads, Apache-2.0/MIT licensed) is the standard Rust graph library. It provides:

- `StableGraph`: Nodes and edges keep stable indices after removals — critical when entity IDs (Uuid) must remain valid across updates
- `GraphMap`: Hash-backed, node-is-key — natural when entity nodes are keyed by name/Uuid
- Built-in algorithms: `dijkstra` (shortest path for multi-hop), `min_spanning_tree` (entity cluster detection), BFS/DFS (subgraph extraction for query-time expansion)
- `serde-1` feature: Serialize/deserialize entity graph alongside state
- DOT export: Visualize entity graph for debugging

### 4.2 Data Structures

```rust
// New module: src/memory/entity_graph.rs

use petgraph::stable_graph::StableGraph;
use petgraph::graph::NodeIndex;
use uuid::Uuid;

/// An entity node in the KG.
struct EntityNode {
    /// Stable canonical name (normalized)
    name: String,
    /// Entity type classification
    entity_type: EntityType,
    /// Source node IDs in the fractal tree (bidirectional link)
    source_node_ids: Vec<Uuid>,
    /// Embedding for similarity-based merge detection
    embedding: Option<Vec<f32>>,
    /// Last consolidation timestamp
    last_consolidated: DateTime<Utc>,
}

/// An edge between two entities.
struct EntityEdge {
    relation_type: String,     // "works_on", "is_part_of", "decided", "prefers"
    source_node_id: Uuid,      // which FractalNode this relation was extracted from
    confidence: f64,            // extraction confidence
    evidence: String,           // short text snippet proving the relation
    created_at: DateTime<Utc>,
}

pub struct EntityGraph {
    /// The petgraph backing store
    graph: StableGraph<EntityNode, EntityEdge>,
    /// Fast lookup: canonical name → NodeIndex
    name_index: HashMap<String, NodeIndex>,
    /// Fast lookup: source FractalNode ID → graph node indices
    source_index: HashMap<Uuid, Vec<NodeIndex>>,
}
```

### 4.3 Core Operations

| Operation | Description | petgraph API |
|-----------|-------------|-------------|
| `add_entity(name, type, source_id)` | Create or get existing entity node | `graph.add_node()` + `name_index` insert |
| `add_relation(from, to, type, evidence)` | Add or strengthen a relation edge | `graph.add_edge()` or `graph.update_edge()` |
| `get_seed_entities(query_embedding)` | Find top-K entities relevant to query | Cosine similarity against entity embeddings |
| `multi_hop_expand(seeds, max_depth)` | BFS from seed entities to collect subgraph | `petgraph::visit::Bfs` |
| `get_source_nodes(entity)` | Map entity back to fractal tree nodes | `source_index` lookup |
| `merge_entities(a, b)` | Combine two entities (dedup) | Relabel edges, update indices |

### 4.4 Integration into Retrieval Pipeline

Currently, `src/retrieval/hybrid.rs` uses only vector similarity (dense) + BM25 (sparse) via RRF fusion. Adding entity graph retrieval as a 4th perspective:

```
Query Q
  ↓
┌──────────┬──────────┬──────────┬──────────────┐
│  Dense   │  BM25    │  Hybrid  │  Entity Graph │  ← NEW
│ (USearch)│ (BM25)   │ (RRF)    │  (petgraph)  │
└──────────┴──────────┴──────────┴──────────────┘
  ↓
Multi-Factor RRF Fusion (k=60)
  ↓
Cross-Encoder Rerank (existing)
  ↓
Source-Weighted Scoring (existing)
```

**Entity graph retrieval sub-flow:**

1. Embed query → find similar entity embeddings (cosine similarity)
2. For top-K seed entities → BFS from seeds up to 2 hops
3. Map all entities in subgraph → source FractalNode IDs via `source_index`
4. Fetch source nodes → add to candidate pool with entity-graph-derived boost

### 4.5 Cargo.toml Addition

```toml
[dependencies]
petgraph = { version = "0.8", features = ["serde-1", "stable_graph"] }
```

This adds ~50KB of additional binary size and zero new system dependencies. No native libraries required — petgraph is pure Rust.

---

## 5. Effort/Benefit Assessment

### 5.1 Effort Estimate

| Task | Files | Estimated Hours | Complexity |
|------|-------|----------------|------------|
| **Entity extraction — inline regex** | `src/memory/entity_graph.rs` (new), `src/memory/fact_extraction.rs` (modify) | 3-4h | Low: extend existing regex rules |
| **Entity extraction — consolidation LLM** | `src/vlm/mod.rs` (modify), `src/memory/dream/consolidation.rs` (modify) | 4-6h | Medium: parse LLM output into structured entities |
| **EntityGraph struct + petgraph** | `src/memory/entity_graph.rs` (new) | 6-8h | Medium: new data structure, index management |
| **Retrieval integration** | `src/retrieval/hybrid.rs` (modify), `src/retrieval/mod.rs` (modify) | 6-8h | Medium: new retrieval perspective, RRF fusion |
| **Serialization + storage** | `src/storage/postgres_store.rs` (modify), `src/storage/in_memory.rs` (modify) | 3-4h | Low: petgraph has `serde-1` feature |
| **Tests + benchmarks** | Various test files | 4-6h | Medium: need entity-specific eval |
| **Total** | | **26-36 hours** | |

### 5.2 Benefit Estimation

**Quantitative (based on H-Mem ablation):**

H-Mem's ablation shows KG removal causes a measurable but not dramatic drop. For KnowWhere on LongMemEval:

- **Current Recall@5**: 72.97%
- **Estimated improvement**: +2-4 percentage points (multi-hop queries currently fail completely)
- **Most impacted query types**: Questions requiring entity traversal ("What project was Nimar working on after the Rust migration?", "Who else discussed the consolidation bug?")

**Qualitative benefits:**

| Benefit | Impact | Justification |
|---------|--------|--------------|
| Multi-hop reasoning | High | Currently impossible — queries spanning multiple entities return noise |
| Entity-centric memory browsing | Medium | Users can explore "everything about Project X" |
| Decision graph visualization | Medium | `MemoryType::Decision` nodes + graph edges = decison DAG, debuggable |
| Future-proofing | High | H-Mem and Mem0 both invest in graphs — this is table-stakes in 2026 |

### 5.3 Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Entity extraction noise (false positives) | Medium | Start with high-confidence regex only. LLM extraction gated behind confidence threshold. |
| Graph bloat (too many entities) | Medium | Periodic pruning of stale/low-degree entities. Max entities cap (configurable). |
| Retrieval latency increase | Low | Entity graph is in-memory (petgraph operates on ~hundreds-not-millions of nodes). Lookups are O(1) via HashMap; BFS is O(V+E) and bounded by max_depth=2. |
| LLM cost for entity extraction | Low | Inline regex extraction costs zero LLM calls. Consolidation-time extraction reuses existing LLM calls (VLM already generates entity text). |

---

## 6. Architectural Fit

### 6.1 Near-Decomposability (Simon 1962)

The entity graph is a **Tier-2 addition** (Consolidation Pipeline in Steele's 3-Tier Architecture). As `THEORETICAL_FOUNDATIONS.md` establishes:

> Each tier admits modification independently of the others. A new adapter at Tier 2 does not destabilize Tier 1 (Embedding) or Tier 3 (Retrieval).

Adding entity extraction to consolidation is local: it touches only the consolidation step and adds a new retrieval perspective. The existing tree structure, embedding providers, and cross-encoder pipeline are unchanged.

### 6.2 Existing Seam: The `relations` Field

The `FractalNode` struct already has:

```rust
pub relations: Vec<Relation>,
```

where `Relation` = `{target_id: Uuid, relation_type: String, strength: f64}`. This field is:
- **Defined** but **never populated** (always `Vec::new()` on construction)
- **Type-compatible** with entity graph edges (target_id → entity node, relation_type → edge label)
- **Already serialized** in the storage layer

The entity graph does NOT need a schema migration. It populates an existing field. This is the strongest signal that the architecture anticipated this layer.

### 6.3 Decision Memory as Implicit Graph

`MemoryType::Decision` nodes with `superseded_by` edges already form a _decision DAG_:

```
Decision A (2026-03-14) → superseded_by → Decision B (2026-04-20)
```

The entity graph extends this from "decisions-only" to "all entities". Decision nodes become a subgraph of the larger entity graph.

---

## 7. Implementation Roadmap

### Phase 1: Entity Extraction (Low-Risk, Immediate Value)

1. Add entity regex rules to `fact_extraction.rs` (Persons, Projects, Technologies)
2. Modify `ExtractedFact::to_fractal_node()` to populate `relations` when entities detected
3. Store extracted entities in FractalNode metadata

### Phase 2: EntityGraph Struct (Core)

1. Create `src/memory/entity_graph.rs` with `EntityGraph`, `EntityNode`, `EntityEdge`
2. Add `petgraph` to Cargo.toml
3. Integrate into `src/memory/mod.rs` (behind feature flag `entity-graph`)
4. Serialize/deserialize with `serde-1`

### Phase 3: Consolidation-Time Extraction

1. Parse VLM `Detailed` output for entities (already prompted on line 170)
2. Insert entities + relations into `EntityGraph`
3. Wire bidirectional links: entity → source FractalNode IDs

### Phase 4: Retrieval Integration

1. Add entity graph perspective to `src/retrieval/hybrid.rs`
2. Implement BFS-based entity expansion
3. Fuse entity graph scores into RRF

### Phase 5: Evaluation

1. Run on LongMemEval with multi-hop subset
2. Compare against baseline (72.97% Recall@5)
3. Measure latency impact

---

## 8. What NOT to Build

H-Mem has several features KnowWhere should NOT replicate in v1:

| H-Mem Feature | Why Skip for Now |
|---------------|------------------|
| Full entity profile system | Adds complexity without clear retrieval gain for v1 |
| Automatic entity disambiguation | H-Mem uses GPT-4o-mini for this — open-weight LLMs may be less reliable |
| Bridge queries (missing-info detection) | Separate feature; included as Priority 4 in HMEM_PAPER_ANALYSIS.md |
| Multi-hop query decomposition | Separate feature; Priority 3 in HMEM_PAPER_ANALYSIS.md |

The v1 entity graph is intentionally **lightweight**: extract entities, link them, use basic BFS for retrieval expansion. Let H-Mem's ablation findings guide scope — the KG provides meaningful but incremental gains; don't overbuild.

---

## 9. References

1. **H-Mem Paper (arXiv 2605.15701)**, Section 2B: Knowledge Graph & Hybrid Retrieval
2. **KnowWhere HMEM_PAPER_ANALYSIS.md**: Full analysis, ablation findings, priority ranking
3. **KnowWhere THEORETICAL_FOUNDATIONS.md**: Steele's 3-Tier Architecture, Near-Decomposability
4. **petgraph crate (v0.8.3)**: https://crates.io/crates/petgraph | https://docs.rs/petgraph/latest/petgraph/
5. **Mem0 Graph Memory**: https://mem0.ai/blog/graph-memory-solutions-ai-agents — Competitor analysis
6. **Fountain City Agent Memory Comparison (2026)**: https://fountaincity.tech/resources/blog/agent-memory-knowledge-systems-compared/

---

## Appendix A: Entity Regex Rules (Draft)

```
Rule 1: Person detection
    Pattern: \b[A-Z][a-z]+ [A-Z][a-z]+\b  (two capitalized words)
    Confidence: 0.60 (verify with context)

Rule 2: Project/Product detection
    Pattern: \b[A-Z][a-zA-Z]+(?:-[A-Za-z0-9]+)*\b  (CamelCase or kebab-case)
    Confidence: 0.55

Rule 3: Technology detection
    Pattern: \b(Rust|Python|TypeScript|PostgreSQL|Ollama|Docker|Kubernetes|...)\b
    Confidence: 0.90 (known tech list)

Rule 4: Relation detection
    Pattern: <entity> (works on|uses|decided on|prefers|migrated to|built) <entity>
    Confidence: 0.70
```

## Appendix B: Entity Graph Benchmark Sketch

A spike-only benchmark to validate the approach before full implementation:

```rust
// Quick validation: does entity graph retrieval find nodes 
// that pure vector retrieval misses?

let eg = EntityGraph::new();
// ... extract entities from ~500 nodes ...

// Multi-hop query: "What did Nimar decide about the database after the Docker migration?"
let query = "database decision after migration";
let baseline = fractal_retrieve(query, 10);  // dense + BM25
let boosted = entity_aware_retrieve(query, 10);  // dense + BM25 + entity expansion

// Expected: boosted retrieves Decision nodes that baseline misses
// because entity expansion follows Nimar→KnowWhere→Database decision chain
```

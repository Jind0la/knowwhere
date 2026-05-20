# Entity Graph Layer — Implementation Plan

**Status:** Approved  
**Date:** May 20, 2026  
**Source:** `docs/ENTITY_GRAPH_SPIKE.md` (parent task t_d112844c)  
**Related:** `docs/HMEM_PAPER_ANALYSIS.md`, `docs/THEORETICAL_FOUNDATIONS.md`

---

## Go/No-Go Decision: **GO**

### Justification

| Factor | Assessment | Weight |
|--------|-----------|--------|
| **Architectural seam** | `FractalNode.relations: Vec<Relation>` exists but is never populated — no schema migration needed | Critical |
| **H-Mem ablation evidence** | KG removal causes measurable drop; KG+Tree > Tree alone for multi-hop queries (arXiv 2605.15701, Section 2B) | High |
| **LLM extraction already exists** | VLM summarizer (`src/vlm/mod.rs:170`) prompts for "Entities, timestamps, relationships" in `SummaryContext::Detailed` mode — data is generated, just discarded | High |
| **Zero new system deps** | petgraph is pure Rust, ~50KB binary increase, no native libraries | Medium |
| **Estimated impact** | +2–4% Recall@5 on LongMemEval (multi-hop queries currently fail completely) | Medium |
| **Risk** | Entity extraction noise, graph bloat — both mitigatable with confidence thresholds and periodic pruning | Low |

### Decision

Implement the entity graph as a **4th retrieval perspective** alongside Dense, BM25, and Hybrid (RRF). Gate behind a `entity-graph` cargo feature flag. Roll out in 5 sequential phases with tests at each stage.

**Effort:** 26–36 hours | **Risk:** Low-Medium | **Impact:** Medium-High

---

## 1. Technology Stack

### 1.1 Graph Backend: petgraph v0.8

| Criterion | petgraph | Alternatives |
|-----------|----------|-------------|
| **Stable node indices** | `StableGraph` — indices survive removals | `graph` crate lacks this |
| **Node-as-key** | `GraphMap` — natural for entity name → node mapping | Neo4j requires external DB |
| **Built-in algorithms** | `dijkstra`, `min_spanning_tree`, BFS/DFS | Purpose-built for KnowWhere's needs |
| **Serialization** | `serde-1` feature — serialize entity graph alongside state | Critical for persistence |
| **License** | Apache-2.0 / MIT dual-licensed | Compatible with KnowWhere's MIT |
| **Binary impact** | ~50KB, pure Rust, zero system deps | Docker image unaffected |

**Cargo.toml addition:**

```toml
[dependencies]
petgraph = { version = "0.8", features = ["serde-1", "stable_graph"], optional = true }

[features]
entity-graph = ["dep:petgraph"]
```

### 1.2 Entity Extraction: Hybrid Regex + LLM

Two-tier extraction strategy matching the spike's recommendation:

| Tier | Method | Trigger | Cost | Confidence |
|------|--------|---------|------|------------|
| **Tier 1 (Inline)** | Regex rules in `fact_extraction.rs` | Every `store_session` / `store_external` call | Zero (regex) | 0.55–0.90 per rule |
| **Tier 2 (Consolidation)** | Parse VLM `Detailed` output | Dream Pipeline consolidation | Zero marginal (reuses existing LLM call) | High (LLM-generated) |

---

## 2. Data Model

### 2.1 Entity Graph Structures

```rust
// src/memory/entity_graph.rs

use petgraph::stable_graph::StableGraph;
use petgraph::graph::NodeIndex;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Entity type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    Person,
    Project,
    Technology,
    Concept,
    Location,
    Organization,
    /// Existing MemoryType::Decision nodes — auto-linked
    Decision,
}

/// A node in the entity knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityNode {
    /// Canonical normalized name (e.g., "KnowWhere" not "KnowWhere project")
    pub name: String,
    /// Classification
    pub entity_type: EntityType,
    /// Source FractalNode IDs this entity was extracted from (bidirectional link)
    pub source_node_ids: Vec<Uuid>,
    /// Embedding for similarity-based entity matching and merge detection
    pub embedding: Option<Vec<f32>>,
    /// Last time this entity was encountered/updated
    pub last_seen: DateTime<Utc>,
    /// Extraction confidence (aggregate across sources)
    pub confidence: f64,
}

/// A directed relation edge between two entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEdge {
    /// Relation predicate (e.g., "works_on", "uses", "built")
    pub relation_type: String,
    /// Which FractalNode this relation was extracted from (provenance)
    pub source_node_id: Uuid,
    /// Extraction confidence
    pub confidence: f64,
    /// Short evidence snippet from the source text
    pub evidence: String,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

/// The entity knowledge graph — a consumer of FractalNodes via extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityGraph {
    /// Backing petgraph store (stable indices survive deletions)
    graph: StableGraph<EntityNode, EntityEdge>,
    /// Fast lookup: canonical entity name → petgraph node index
    name_index: HashMap<String, NodeIndex>,
    /// Fast lookup: source FractalNode UUID → entity graph nodes that reference it
    source_index: HashMap<Uuid, Vec<NodeIndex>>,
    /// Configuration
    config: EntityGraphConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityGraphConfig {
    /// Maximum number of entities before pruning kicks in
    pub max_entities: usize,
    /// Minimum confidence for LLM-extracted entities (regex uses per-rule confidence)
    pub llm_confidence_threshold: f64,
    /// Minimum degree for entity retention during pruning
    pub min_degree_for_retention: usize,
    /// Maximum BFS depth for multi-hop expansion during retrieval
    pub max_expansion_depth: usize,
    /// Maximum entities returned from seed entity lookup
    pub max_seed_entities: usize,
}

impl Default for EntityGraphConfig {
    fn default() -> Self {
        Self {
            max_entities: 10_000,
            llm_confidence_threshold: 0.70,
            min_degree_for_retention: 1,
            max_expansion_depth: 2,
            max_seed_entities: 10,
        }
    }
}
```

### 2.2 Relationship to Existing `FractalNode.relations`

The `FractalNode.relations: Vec<Relation>` field becomes the **bridge** between the fractal tree and the entity graph:

```
EntityGraph (petgraph)          FractalNode tree
┌────────────────────┐          ┌──────────────────┐
│ EntityNode "Nimar" │          │ node-abc123      │
│  source_node_ids:  │─────────▶│  relations:       │
│   [abc123, def456] │          │   [{target: "KnowWhere", type: "works_on"}] │
└────────────────────┘          └──────────────────┘
```

**Populating `relations` from entity graph extractions:**
- When Tier 1 regex detects a relation (e.g., "Nimar uses Rust") → adds `Relation { target_id: EntityNode.id, relation_type: "uses", strength: 0.7 }` to the source FractalNode
- When Tier 2 LLM extraction finds entities → same population path

This means `relations` is populated for the FIRST time in KnowWhere's history. Existing code that reads `relations` (currently always empty) gains data without any code changes.

---

## 3. Implementation Phases

### Phase 1: Entity Extraction — Inline Regex (3–4 hours)

**Goal:** Catch obvious entities at storage time with zero LLM cost.

**Files:**
- **Create:** `src/memory/entity_graph.rs` — EntityGraph struct, types, new()
- **Modify:** `src/memory/fact_extraction.rs` — add entity extraction regex rules
- **Modify:** `src/memory/mod.rs` — add `pub mod entity_graph` (behind `entity-graph` feature)
- **Modify:** `Cargo.toml` — add petgraph dependency + `entity-graph` feature flag

**Regex rules to implement (from spike Appendix A):**

```rust
// Rule 1: Person detection — two capitalized words
// Pattern: \b[A-Z][a-z]+ [A-Z][a-z]+\b
// Confidence: 0.60 (verify with context: "I"/"my"/"decided" nearby)

// Rule 2: Project/Product detection — CamelCase or kebab-case
// Pattern: \b[A-Z][a-zA-Z]+(?:-[A-Za-z0-9]+)*\b
// Confidence: 0.55

// Rule 3: Technology detection — known tech list
// Pattern: \b(Rust|Python|PostgreSQL|Ollama|Docker|Kubernetes|SQLite|Redis|...)\b
// Confidence: 0.90

// Rule 4: Relation detection — entity + action verb + entity
// Pattern: <entity> (works on|uses|decided on|prefers|migrated to|built|configured) <entity>
// Confidence: 0.70
```

**Integration point in `fact_extraction.rs`:**

The existing `ExtractedFact::to_fractal_node()` method creates a `FractalNode` for each fact. Extend it to:
1. After creating the fact node, call a new function `extract_entities_from_text(content) -> Vec<(String, EntityType, f64)>`
2. Upsert extracted entities into the `EntityGraph`
3. Create `Relation` entries on the source node linking to entity nodes
4. Update `EntityGraph.source_index` to map FractalNode ID → entity nodes

**EntityGraph public API added in this phase:**

```rust
impl EntityGraph {
    /// Create a new, empty entity graph.
    pub fn new(config: EntityGraphConfig) -> Self;

    /// Add or get an existing entity node by normalized name.
    /// Returns the petgraph NodeIndex (stable identifier).
    pub fn upsert_entity(
        &mut self,
        name: &str,
        entity_type: EntityType,
        source_node_id: Uuid,
        confidence: f64,
    ) -> NodeIndex;

    /// Add a relation edge between two entity nodes.
    /// Strengthens existing edges instead of duplicating.
    pub fn add_relation(
        &mut self,
        from: NodeIndex,
        to: NodeIndex,
        relation_type: &str,
        source_node_id: Uuid,
        confidence: f64,
        evidence: &str,
    );

    /// Return all entity node indices linked to a source FractalNode.
    pub fn entities_for_node(&self, node_id: &Uuid) -> Vec<NodeIndex>;
}
```

**Tests to write (Phase 1):**
- `test_extract_person_entity` — "Nimar configured the server" extracts Person "Nimar"  
- `test_extract_technology_entity` — "migrated from Docker to native macOS" extracts Technology "Docker"
- `test_extract_relation` — "Nimar uses Rust" creates relation edge with confidence 0.70
- `test_entity_upsert_dedup` — two nodes mentioning "PostgreSQL" → single entity, two source_node_ids
- `test_relation_strengthening` — same relation extracted twice → edge confidence increases (not duplicate)
- `test_fact_extraction_integration` — `ExtractedFact::to_fractal_node()` populates `relations` field

**Feature flag gating:**
All entity_graph code is behind `#[cfg(feature = "entity-graph")]`. The `EntityGraph` field is added to AppState as `Option<EntityGraph>` (None when feature disabled). This ensures zero behavioral change for the default build.

**Commit:** `feat: add entity-graph feature flag, EntityGraph struct, inline regex extraction`

---

### Phase 2: EntityGraph Core Operations (6–8 hours)

**Goal:** Full EntityGraph with all retrieval-related operations, integration into AppState.

**Files:**
- **Modify:** `src/memory/entity_graph.rs` — all remaining methods
- **Modify:** `src/main.rs` — instantiate EntityGraph at startup
- **Modify:** `src/api/routes.rs` — add `entity_graph` to AppState

**AppState addition:**

```rust
// src/main.rs — inside AppState construction
#[cfg(feature = "entity-graph")]
let entity_graph = Arc::new(tokio::sync::RwLock::new(
    EntityGraph::new(EntityGraphConfig::default())
));
```

**Full EntityGraph public API:**

```rust
impl EntityGraph {
    // --- Construction ---
    pub fn new(config: EntityGraphConfig) -> Self;
    
    // --- Entity CRUD ---
    pub fn upsert_entity(&mut self, name: &str, entity_type: EntityType, 
                         source_node_id: Uuid, confidence: f64) -> NodeIndex;
    
    /// Find top-K entity nodes by cosine similarity to a query embedding.
    /// Only returns entities that have embeddings set.
    pub fn find_seed_entities(&self, query_embedding: &[f32], k: usize) 
        -> Vec<(NodeIndex, f32)>;
    
    /// Look up an entity by canonical name.
    pub fn find_entity(&self, name: &str) -> Option<NodeIndex>;
    
    /// All entity nodes linked to a source FractalNode.
    pub fn entities_for_node(&self, node_id: &Uuid) -> Vec<NodeIndex>;
    
    // --- Relations ---
    pub fn add_relation(&mut self, from: NodeIndex, to: NodeIndex,
                        relation_type: &str, source_node_id: Uuid,
                        confidence: f64, evidence: &str);
    
    /// Get all relations (edges) for an entity node.
    pub fn relations_for_entity(&self, entity: NodeIndex) 
        -> Vec<(NodeIndex, &EntityEdge)>;
    
    // --- Multi-Hop Expansion ---
    
    /// BFS from seed entity nodes up to config.max_expansion_depth.
    /// Returns all entity nodes reachable in the subgraph.
    pub fn multi_hop_expand(
        &self,
        seeds: &[NodeIndex],
    ) -> Vec<NodeIndex>;
    
    /// Map entity graph nodes back to source FractalNode UUIDs.
    /// This is the critical bidirectional link for retrieval.
    pub fn source_nodes_for_entities(
        &self,
        entities: &[NodeIndex],
    ) -> HashSet<Uuid>;
    
    /// Full retrieval sub-flow:
    /// 1. embed query → find seed entities
    /// 2. BFS from seeds → collect subgraph entities
    /// 3. map to source FractalNode IDs
    /// Returns (fractal_node_ids, entity_match_scores)
    pub fn entity_aware_retrieve(
        &self,
        query_embedding: &[f32],
    ) -> (HashSet<Uuid>, Vec<(Uuid, f32)>);
    
    // --- Maintenance ---
    
    /// Remove stale entities (below min_degree_for_retention, last_seen > threshold).
    /// Returns count of pruned entities.
    pub fn prune_stale(&mut self, older_than: DateTime<Utc>) -> usize;
    
    /// Merge two entity nodes (disambiguation). Relabels edges, updates indices.
    /// Returns the preserved NodeIndex.
    pub fn merge_entities(&mut self, primary: NodeIndex, secondary: NodeIndex) 
        -> anyhow::Result<NodeIndex>;
    
    // --- Serialization ---
    pub fn to_json(&self) -> serde_json::Result<String>;
    pub fn from_json(json: &str) -> serde_json::Result<Self>;
    
    // --- Stats ---
    pub fn entity_count(&self) -> usize;
    pub fn edge_count(&self) -> usize;
}
```

**BFS implementation detail:**

```rust
pub fn multi_hop_expand(&self, seeds: &[NodeIndex]) -> Vec<NodeIndex> {
    use petgraph::visit::Bfs;
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    
    for &seed in seeds {
        let mut bfs = Bfs::new(&self.graph, seed);
        while let Some(node) = bfs.next(&self.graph) {
            if visited.insert(node) {
                result.push(node);
            }
        }
    }
    result
}
```

**Tests to write (Phase 2):**
- `test_entity_graph_new_empty` — new EntityGraph has 0 entities, 0 edges
- `test_upsert_entity_creates` — first upsert creates entity
- `test_upsert_entity_reuses` — second upsert for same name returns existing index
- `test_add_relation` — edge created with correct source_node_id and evidence
- `test_find_seed_entities` — cosine similarity returns correct top-K
- `test_multi_hop_expand_single_layer` — BFS from 1 seed with 3 neighbors returns 4 nodes
- `test_multi_hop_expand_two_hops` — BFS depth=2 reaches indirect neighbors
- `test_source_nodes_for_entities` — maps entity nodes ↔ FractalNode UUIDs
- `test_prune_stale` — low-degree old entity removed, high-degree retained
- `test_merge_entities` — edges relabeled after merge, secondary index removed
- `test_serialize_roundtrip` — to_json → from_json preserves all entities and edges

**Commit:** `feat: EntityGraph core operations — upsert, BFS, seed lookup, prune, merge`

---

### Phase 3: Consolidation-Time LLM Extraction (4–6 hours)

**Goal:** Parse VLM `Detailed` output for entities and populate EntityGraph.

**Files:**
- **Modify:** `src/vlm/mod.rs` — parse entity/relationship data from LLM `Detailed` output
- **Modify:** `src/memory/dream/consolidation.rs` — wire entity extraction into consolidation pipeline
- **Modify:** `src/memory/dream/mod.rs` — pass EntityGraph reference to ConsolidationScheduler

**What changes:**

The VLM summarizer's `SummaryContext::Detailed` prompt already instructs the LLM to produce:
```
THIRD PARAGRAPH: Entities, timestamps, relationships.
```

And the claims block format:
```
---CLAIMS---
- claim: <what was decided>
  reason: <why>
  alternatives: [...]
  consequences: [...]
- claim: <next claim>
---END---
```

**New: Add an `---ENTITIES---` block to the Detailed prompt:**

```
After your claims block, add an entities block:
---ENTITIES---
- entity: KnowWhere
  type: Project
  relations:
    - relation: built_with
      target: Rust
      evidence: "KnowWhere is written in Rust"
    - relation: uses
      target: Ollama
      evidence: "embedding via Ollama nomic-embed-text"
- entity: Nimar
  type: Person
  relations:
    - relation: created
      target: KnowWhere
      evidence: "Nimar created KnowWhere"
---END---
```

**Parser implementation in `src/memory/entity_graph.rs`:**

```rust
impl EntityGraph {
    /// Parse entity extraction from LLM summary output (Detailed mode).
    /// Expected format: ---ENTITIES--- block with entity/type/relations.
    pub fn ingest_from_summary(
        &mut self,
        summary_text: &str,
        source_node_id: Uuid,
        confidence_threshold: f64,
    ) -> usize {
        // 1. Find ---ENTITIES--- block
        // 2. For each entity entry: parse name, type, relations
        // 3. upsert_entity() for each entity
        // 4. add_relation() for each relation (resolving target entity name)
        // 5. Return count of entities added/updated
    }
}
```

**Integration into ConsolidationScheduler:**

In `src/memory/dream/consolidation.rs`, after LLM summarization returns the `Detailed` summary text:

```rust
#[cfg(feature = "entity-graph")]
if let Some(entity_graph) = &self.entity_graph {
    let mut eg = entity_graph.write().await;
    let added = eg.ingest_from_summary(
        &summary_text, 
        parent_node_id, 
        self.config.entity_confidence_threshold
    );
    tracing::info!(
        entities_added = added,
        parent_node_id = %parent_node_id,
        "Entity graph updated from consolidation summary"
    );
}
```

**Tests to write (Phase 3):**
- `test_ingest_from_summary_single_entity` — parses one entity correctly
- `test_ingest_from_summary_with_relations` — parses entity + relations + populates edges
- `test_ingest_from_summary_multiple_entities` — handles 3+ entities in one block
- `test_ingest_from_summary_no_block` — gracefully handles missing ---ENTITIES--- block
- `test_ingest_from_summary_malformed` — survives malformed entity entries
- `test_consolidation_with_entity_graph` — end-to-end: consolidate → summary → EntityGraph populated
- `test_entity_extraction_idempotent` — same summary ingested twice → entities updated, not duplicated

**Commit:** `feat: LLM entity extraction via VLM Detailed summary ---ENTITIES--- block`

---

### Phase 4: Retrieval Integration (6–8 hours)

**Goal:** Add entity graph as a 4th perspective in hybrid retrieval, fused into RRF.

**Files:**
- **Modify:** `src/retrieval/hybrid.rs` — add entity graph scoring to hybrid pipeline
- **Modify:** `src/storage/backend.rs` — extend `HybridQuery` with entity graph options
- **Modify:** `src/storage/in_memory.rs` — wire EntityGraph into `hybrid_retrieve`
- **Modify:** `src/api/routes.rs` — pass entity graph to retrieval calls

**HybridQuery extension:**

```rust
pub struct HybridQuery {
    // ... existing fields ...
    
    /// Enable entity graph expansion as a retrieval perspective.
    /// When true, entity-aware retrieval runs alongside dense/BM25.
    #[serde(default)]
    pub entity_graph: bool,
    
    /// Weight for entity graph scores in final RRF fusion.
    /// 0.0 = entity graph disabled in scoring, 1.0 = equal to other perspectives.
    /// Recommended: 0.3–0.5 for balanced impact.
    #[serde(default = "default_entity_weight")]
    pub entity_graph_weight: f32,
}

fn default_entity_weight() -> f32 { 0.4 }
```

**Retrieval pipeline extended:**

```
Query Q
  ↓
┌──────────┬──────────┬──────────┬──────────────┐
│  Dense   │  BM25    │  Hybrid  │  Entity Graph │  ← NEW
│ (USearch)│ (BM25)   │ (RRF)    │  (petgraph)  │
└──────────┴──────────┴──────────┴──────────────┘
  ↓
Multi-Factor RRF Fusion (k=60, 4 perspectives)
  ↓
Cross-Encoder Rerank (existing)
  ↓
Source-Weighted Scoring (existing)
  ↓
Temporal Decay (existing)
```

**Entity graph retrieval sub-flow (in `hybrid.rs`):**

```rust
/// Run entity graph perspective and return scored nodes.
#[cfg(feature = "entity-graph")]
async fn entity_graph_retrieve(
    entity_graph: &EntityGraph,
    query_embedding: &[f32],
    query_text: &str,
    top_k: usize,
) -> Vec<(Uuid, f32)> {
    // 1. Find seed entities by embedding similarity
    let seeds = entity_graph.find_seed_entities(query_embedding, 10);
    if seeds.is_empty() {
        return vec![];
    }
    
    // 2. BFS from seeds to build subgraph
    let entities = entity_graph.multi_hop_expand(
        &seeds.iter().map(|(idx, _)| *idx).collect::<Vec<_>>()
    );
    
    // 3. Map entities to source FractalNode IDs
    let source_ids = entity_graph.source_nodes_for_entities(&entities);
    
    // 4. Return (node_id, entity_graph_score) pairs
    //    Score = max(seed_similarity * edge_confidence) for each node
    source_ids.into_iter().map(|id| {
        let score = compute_entity_score(id, &seeds, entity_graph);
        (id, score)
    }).take(top_k).collect()
}
```

**Multi-Factor RRF with 4 perspectives:**

The existing RRF fusion in `hybrid.rs` currently combines 2–3 perspectives (dense, BM25, sometimes query-expanded variants). The entity graph adds a 4th:

```rust
// Current RRF: combine dense and BM25 ranks
// Extended: combine dense + BM25 + query_expansion + entity_graph ranks

fn multi_factor_rrf(
    dense_results: &[(Uuid, f32)],
    bm25_results: &[(Uuid, f32)],
    expansion_results: &[(Uuid, f32)],       // from query_expansion (may be empty)
    entity_results: &[(Uuid, f32)],           // NEW: from entity_graph (may be empty)
    k: f32,
    entity_weight: f32,
) -> Vec<(Uuid, f32)> {
    // For each unique UUID across all result sets:
    //   rrf_score = sum(1/(k + rank_in_perspective))  for each perspective it appears in
    //   entity perspectives are weighted by entity_weight
}
```

**RetrievalProfile integration:**

Entity graph retrieval respects the existing `RetrievalProfile`:
- `UserFacing`: entity graph results filtered through `allows()` (no internal-only nodes)
- `AgentDebug` / `FullFidelity`: full entity graph results included
- Entity graph score multiplier applied through existing `score_multiplier()` chain

**Tests to write (Phase 4):**
- `test_entity_graph_retrieve_seeds_only` — seed entities returned when no relations
- `test_entity_graph_retrieve_multi_hop` — multi-hop expansion returns indirectly linked nodes
- `test_entity_graph_retrieve_empty` — no entities → empty result, no panic
- `test_hybrid_with_entity_graph` — entity graph results appear in hybrid_retrieve output
- `test_rrf_four_perspectives` — 4-way RRF correctly fuses scores
- `test_entity_graph_weight_zero` — entity_graph_weight=0.0 → entity results excluded
- `test_entity_graph_weight_full` — entity_graph_weight=1.0 → entity results equal to others
- `test_entity_graph_user_facing_filter` — UserFacing profile filters internal-only entity nodes

**Commit:** `feat: entity graph 4th retrieval perspective with multi-factor RRF fusion`

---

### Phase 5: Evaluation & Tuning (4–6 hours)

**Goal:** Measure real impact on LongMemEval and tune parameters.

**Files:**
- **Create:** `benchmarks/entity_graph_eval.py` — evaluation harness
- **Modify:** `benchmarks/longmemeval_eval.py` — optional entity graph integration

**Benchmark design:**

```python
# benchmarks/entity_graph_eval.py

# Two-run comparison:
# Run 1: Baseline (no entity graph) → Recall@5
# Run 2: Entity graph enabled → Recall@5

# Multi-hop subset detection:
# Identify queries that require entity traversal:
# - Queries with 2+ named entities
# - Queries with relational keywords ("after", "before", "decided", "who else")

# Report:
# - Overall Recall@5 delta
# - Multi-hop subset Recall@5 delta (expected: +5-10%)
# - Latency delta (expected: +5-15ms)
# - Graph stats: entity_count, edge_count, avg_degree
```

**Tuning parameters to evaluate:**

| Parameter | Default | Tune Range | Expected Optimal |
|-----------|---------|------------|------------------|
| `entity_graph_weight` | 0.4 | 0.2–0.6 | ~0.3–0.4 |
| `max_entities` | 10,000 | 5,000–50,000 | 10,000 (memory: ~5MB) |
| `llm_confidence_threshold` | 0.70 | 0.50–0.85 | 0.70 |
| `max_expansion_depth` | 2 | 1–3 | 2 (3 rarely helps) |
| `max_seed_entities` | 10 | 5–20 | 10 |

**Tests to write (Phase 5):**
- `test_entity_graph_recall_regression` — entity graph never reduces Recall@5 vs baseline
- `test_multi_hop_improvement` — multi-hop queries measurably improve
- `test_latency_bound` — entity graph adds <50ms to p95 retrieval latency
- `test_graph_serialization_perf` — serialize/deserialize 10k entities in <500ms

**Commit:** `test: entity graph evaluation harness + LongMemEval integration`

---

## 4. API Design

### 4.1 New Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/entities` | List all entities with counts (paginated) |
| `GET` | `/entities/:name` | Get entity details + relations + source nodes |
| `GET` | `/entities/:name/graph` | Get subgraph centered on entity (BFS 1-2 hops) |
| `POST` | `/entities/query` | Find entities by name/type (search) |
| `GET` | `/entities/stats` | Count entities, edges, avg degree, top entities |

### 4.2 Enhanced Retrieval Endpoint

Existing `POST /retrieve` and `POST /retrieve/fractal` accept `HybridQuery`. Add the `entity_graph` and `entity_graph_weight` fields (both optional, backward-compatible — default disabled):

```json
{
  "query_text": "What did Nimar decide about the database?",
  "top_k": 10,
  "max_depth": 2,
  "entity_graph": true,
  "entity_graph_weight": 0.4
}
```

Response includes entity graph metadata in `ScoreDebug`:

```json
{
  "nodes": [...],
  "debug": {
    "entity_graph_seeds_found": 3,
    "entity_graph_subgraph_size": 12,
    "entity_graph_source_nodes": 8
  }
}
```

### 4.3 Response Schema

**GET /entities/:name**

```json
{
  "entity": {
    "name": "KnowWhere",
    "type": "Project",
    "source_node_count": 47,
    "first_seen": "2026-03-14T10:00:00Z",
    "last_seen": "2026-05-20T14:30:00Z",
    "confidence": 0.92
  },
  "relations": [
    {
      "source": "KnowWhere",
      "target": "Rust",
      "type": "built_with",
      "confidence": 0.95,
      "evidence": "KnowWhere is written in Rust",
      "source_node_id": "abc-123"
    },
    {
      "source": "Nimar",
      "target": "KnowWhere",
      "type": "created",
      "confidence": 0.88,
      "evidence": "Nimar created KnowWhere as a fractal memory service",
      "source_node_id": "def-456"
    }
  ],
  "graph": {
    "nodes": ["KnowWhere", "Rust", "Nimar", "Ollama", "PostgreSQL"],
    "edges": [
      ["KnowWhere", "Rust", "built_with"],
      ["KnowWhere", "Ollama", "uses"],
      ["KnowWhere", "PostgreSQL", "uses"],
      ["Nimar", "KnowWhere", "created"]
    ]
  }
}
```

---

## 5. Feature Flag Strategy

The `entity-graph` feature flag ensures:

| Build | entity-graph enabled? | Behavior |
|-------|----------------------|----------|
| `cargo build` | No | No entity extraction, no entity graph, no new retrieval perspective. `HybridQuery.entity_graph` field exists but is ignored (default `false`). |
| `cargo build --features entity-graph` | Yes | Full entity graph enabled. Extraction runs at storage time. 4-perspective RRF retrieval. |
| `cargo build --features "postgres-storage,summarizer,reranker,entity-graph"` | Yes | Production build with all features. |

**Docker builds:** The `entity-graph` feature is composable with all existing features. No mutual exclusion.

---

## 6. What NOT to Build (v1 Scope Boundaries)

These H-Mem features are explicitly deferred:

| Feature | Reason for Deferral |
|---------|-------------------|
| **Full entity profile system** | Adds complexity without clear v1 retrieval gain |
| **Automatic entity disambiguation via LLM** | Requires additional LLM calls per entity pair; merge-by-similarity threshold is sufficient for v1 |
| **Bridge queries** (missing-info detection + follow-up sub-queries) | Separate feature; Priority 4 in HMEM_PAPER_ANALYSIS.md |
| **Multi-hop query decomposition** | Separate feature; Priority 3 in HMEM_PAPER_ANALYSIS.md |
| **Decision graph visualization UI** | Out of scope for backend; consume API to build UI separately |
| **Entity graph persistence in PostgreSQL** | v1 stores in memory only (petgraph serialized to JSON blob on shutdown). PostgreSQL table for entities deferred to v2. |
| **Cross-session entity merging** | v1 graph is per-session. Cross-session merging requires entity resolution, deferred. |

---

## 7. Risk Mitigation

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| **Phase 4 retrieval latency increases >50ms** | Low | petgraph BFS is O(V+E), bounded by `max_expansion_depth=2`. Pre-compute entity embeddings. |
| **Phase 3 LLM extraction quality is poor** | Medium | Confidence threshold gating (0.70). Fallback to Tier 1 regex extraction if LLM fails. `---ENTITIES---` block is optional in parsing. |
| **Phase 1 regex false positives pollute graph** | Medium | Per-rule confidence scoring. Low-confidence entities require 2+ source nodes to be retained. Pruning removes singletons older than 30 days. |
| **Graph memory exceeds budget (target: <50MB)** | Low | `max_entities=10,000` cap enforced. Each EntityNode ~200 bytes + EntityEdge ~150 bytes. 10k entities + 20k edges ≈ 5MB. |
| **Feature interaction: entity graph scores dominate RRF** | Low | `entity_graph_weight` defaults to 0.4, tunable. RRF k=60 is forgiving of score distribution differences. |

---

## 8. Test Strategy

### Unit Tests (Phases 1–4)

Total: ~50 new unit tests across phases. Run with:

```bash
cargo test --lib --features entity-graph
```

### Integration Tests (Phase 5)

```bash
# Full integration test with entity graph
SQLX_OFFLINE=true \
OLLAMA_URL=http://localhost:11434 \
OLLAMA_MODEL=nomic-embed-text \
DATABASE_URL="postgresql://postgres@localhost:5433/kw" \
  cargo test --features "postgres-storage,summarizer,entity-graph" --test integration
```

### Benchmark Validation

```bash
python3 benchmarks/entity_graph_eval.py \
  --api-key kw_testkey_12345 \
  --endpoint http://localhost:3737 \
  --baseline  # run without entity graph first
  --compare   # run with entity graph and compare
```

---

## 9. Success Criteria

| Criterion | Target | Measurement |
|-----------|--------|-------------|
| **Recall@5 improvement (multi-hop)** | +3–8% | LongMemEval multi-hop subset |
| **Recall@5 regression (overall)** | None | Full LongMemEval 50-case run |
| **Retrieval latency p95** | <100ms (from <50ms baseline) | Benchmark harness p95 |
| **Entity extraction precision** | >80% (regex), >90% (LLM) | Manual review of 100 random extractions |
| **Graph memory** | <10MB at 10k entities | Process RSS before/after entity graph init |
| **Test coverage** | 50+ new tests, all passing | `cargo test --features entity-graph --lib` |

---

## 10. References

1. `docs/ENTITY_GRAPH_SPIKE.md` — Research spike with H-Mem findings and architecture sketch
2. `docs/HMEM_PAPER_ANALYSIS.md` — Full H-Mem paper ablation analysis
3. `docs/THEORETICAL_FOUNDATIONS.md` — Steele's 3-Tier Architecture, near-decomposability
4. `src/memory/fractal_node.rs:84-88` — `Relation` struct (unused seam)
5. `src/memory/fact_extraction.rs` — Existing regex extraction, integration point for Tier 1
6. `src/vlm/mod.rs:166-170` — `SummaryContext::Detailed` prompt (already asks for entities)
7. `src/storage/backend.rs:210-250` — `HybridQuery` (extension point for entity_graph fields)
8. `src/retrieval/hybrid.rs` — RRF fusion (extension point for 4th perspective)
9. petgraph crate: https://crates.io/crates/petgraph | https://docs.rs/petgraph/0.8/

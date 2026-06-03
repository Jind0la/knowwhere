# KnowWhere Research Briefing — 2026-05-16

Web-Recherche zu den 5 priorisierten Tasks, ergänzt um Code-Review.
Erstellt für Nimar, Deadline: 09:00 Uhr.

---

## 1. 3-Level-Cascade verdrahten (~30 Zeilen)

**Status:** Infrastruktur fertig, `retrieve_fractal()` nutzt sie NICHT.

### Was existiert:
- `ultra_coarse_search(vector, count) → Vec<Uuid>` — 64d HNSW-Suche (Z. 705-723 in_memory.rs)
- `coarse_search(vector, count) → Vec<Uuid>` — 256d HNSW-Suche (Z. 685-703)
- `ensure_ultra_coarse_index()` / `ensure_coarse_index()` — Index-Lebenszyklus
- Auto-Insert in `insert()`: Jeder neue Node wird automatisch in alle drei Indizes geschrieben

### Was fehlt:
`retrieve_fractal()` (Z. 1208-1289) benutzt NUR `usearch_index` (768d). Weder `ultra_coarse_search` noch `coarse_search` werden aufgerufen.

### Wie es aktuell läuft (Z. 1228-1269):
```rust
// Nur main index (768d):
let candidate_keys = usearch_index.search(query_vector, top_k * 2);
// → zoom_retrieve auf den Candidates → sort → take top_k
```

### Was rein muss (~30 Zeilen):
```rust
// Step 1: Ultra-Coarse (64d) — billig, filtert 95%
let ultra_candidates = self.ultra_coarse_search(&query_vector[..64], top_k * 8).await;

// Step 2: Coarse (256d) — nur innerhalb ultra_candidates
let coarse_candidates = self.coarse_search(&query_vector[..256], top_k * 4).await;
// → Filter: nur Kandidaten, die auch in ultra_candidates sind

// Step 3: Precision (768d) — nur innerhalb coarse_candidates
// → zoom_retrieve wie bisher, aber reduzierter Candidate-Set
```

**TST-Design-Rationale** (aus Paper): Die 3-Level-Cascade entspricht exakt TST's Superposition→Recovery-Phasen. 64d ist die intrinsische Dimension (~4% von 1536d, hier ~8% von 768d). TST nutzt Bag-Averaging auf Embedding-Ebene; KnowWhere nutzt Matryoshka-Trunkierung. Beide erhalten geometrische Kontinuität.

**Industrie-Parallelen:** Oracle AI Vector Search implementiert ähnliche Multi-Level-HNSW (grob→fein über die natürlichen HNSW-Layer). Keiner macht jedoch separate dimensionale Indizes pro Layer — KnowWhere's 64d/256d/768d-Ansatz ist architektonisch unique.

**Konkreter Code-Pointer:** `retrieve_fractal()` Z. 1208, Branch Z. 1228-1269. Die Cascade muss VOR dem usearch_index.search() eingebaut werden, mit fallback auf den bestehenden Code wenn ultra_coarse/coarse Index nicht verfügbar sind.

---

## 2. 64d-Index-Rebuild in load_state()

**Status:** Broken — kein Rebuild.

### Was existiert:
`load_state()` (Z. 467-535) baut NUR den main `usearch_index` aus `state.json` wieder auf:
```
Z. 475-500: dimension = dominant_dimension(&state.nodes)
           → usearch_index rebuilt ONLY
```

### Was fehlt:
- Kein Rebuild von `coarse_index` (256d)
- Kein Rebuild von `ultra_coarse_index` (64d)
- Keine Wiederherstellung der UUID↔Key-Mappings (`coarse_uuid_to_key`, `ultra_coarse_uuid_to_key`)

### Konsequenz:
Nach Server-Neustart sind coarse_index und ultra_coarse_index **leer**. `coarse_search()` und `ultra_coarse_search()` retournieren `vec![]` (Z. 688/708: `None => return vec![]`). Kein Crash, aber die Cascade wird nie getriggert — fällt lautlos auf linearen Scan zurück.

### Was zu tun ist (~40 Zeilen in load_state):
1. Nach dem main-index-Rebuild (Z. 500): Analoge Schleife für 256d und 64d
2. `coarse_uuid_to_key` / `coarse_key_to_uuid` aus `state.json` wiederherstellen — oder aus `uuid_to_key` ableiten
3. `ultra_coarse_uuid_to_key` / `ultra_coarse_key_to_uuid` — dito

**Wichtig:** Das `PersistedState` struct (Z. 123-132) persistiert coarse/ultra_coarse mappings aktuell NICHT. Optionen:
- (A) `PersistedState` erweitern um `coarse_uuid_to_key` etc. → sauber, aber Schema-Change
- (B) Rebuild on-the-fly: Bei load_state für jeden Node Trunkierung berechnen und in neuen Index einfügen → kein Schema-Change, aber langsamer bei großen States

**Empfehlung:** Option B für jetzt (kein Schema-Change, Code-Pfad identisch zu `insert()`), Option A später.

**Industrie-Pattern:** Oracle REBUILD_INDEX, pgvector REINDEX, DuckDB HNSW checkpoint — alle speichern Index-Daten separat vom State. KnowWhere's PersistedState-Ansatz (State + separater Index-Rebuild) ist ein Hybrid. Die meisten Vektor-DBs bauen Indizes komplett neu aus den Rohdaten (kein Serialisieren des Index-Graphen) — KnowWhere macht das auch für den main index, muss es nur für coarse/ultra_coarse nachziehen.

---

## 3. User-ID-Filter-Bug fixen oder dokumentieren

**Status:** Known bug, workaround existiert.

### Der Bug (Z. 212-221, hybrid_retrieve):
```rust
match &query.user_id {
    None => node_uid.is_none(),  // ← BUG: filtert ALLES mit user_id raus
    Some(uid) => node_uid.map_or(true, |v| v == uid.as_str()),
}
```

Wenn `user_id = None`: Nur Nodes **ohne** user_id-Metadaten werden zurückgegeben.
PersonaMem-Daten: **alle 2082 Nodes** haben `user_id` → `None`-Queries retournieren **0 Ergebnisse**.

Selber Bug in `retrieve_fractal()` (Z. 246-252).

### Workaround (bekannt aus Session 2026-05-15):
User-ID aus `data/state.json` extrahieren und im Request mitsenden:
```bash
jq '.nodes | to_entries[0].value.metadata.user_id' data/state.json -r
```

### Optionen für Fix:
1. **None = alle Nodes** (permissiv): `None => true` — einfach, aber verliert multi-tenancy isolation
2. **None = Error** (strikt): `None => return Err("user_id required")` — zwingt caller zum Setzen
3. **Dokumentieren + Workaround beibehalten** — Status quo, aber mit prominentem Warning in API-Docs

### Multi-Tenant Industry Patterns:
- **Pinecone:** metadata filtering mit `$eq`, `$ne` operatoren — `None` ist kein gültiger Filter, muss explizit sein
- **Weaviate:** `where` filter mit GraphQL — kein impliziter Scope
- **Qdrant:** `must` / `must_not` conditions — permissiv by default
- **Milvus:** partition-based oder scalar filtering — explizit

**Fazit:** Die meisten Produktions-Vektor-DBs machen multi-tenancy **explizit**. KnowWhere's `None → node_uid.is_none()` ist ein Design-Pattern was dokumentiert gehört — entweder als Feature („global scope queries sind eine bewusste Operation") oder als Bug („None sollte alles retournieren"). Für PersonaMem-Benchmarks reicht der Workaround. Für Produktion: Option 1 oder 2.

**Empfehlung:** Option 1 (`None => true`) + Feature-Flag für strict mode. Dann ist das Verhalten backward-compatibel für existierende User, und neue Deployments können strict mode aktivieren.

---

## 4. Distributional Scoring (~15 Zeilen)

**Status:** RRF existiert, Softmax-Normalisierung fehlt.

### Was TST sagt:
MCE Loss (Mean Cross-Entropy) = `1/s * Σ CE(z, y_i)` — die Loss-Funktion predictet eine **Verteilung** über das nächste Token-Bag, nicht ein einzelnes Token. Analog für Retrieval: Scores sollten eine Wahrscheinlichkeitsverteilung über den Candidate-Set sein, kein diskretes Top-k-Ranking.

### Was KnowWhere schon hat:
- RRF-Fusion (Z. 1170): `Self::rrf_fuse(&vector_ids, &bm25_results, 5.0)` — produziert bereits Scores die als Distribution interpretierbar sind
- `ScoredNode` struct (aus `storage/mod.rs`) — hat `score: f32`, aber KEIN `distribution_scores`-Feld

### Was fehlt (~15 Zeilen):
1. Nach RRF-Fusion (Z. 1173): Softmax über die fused scores
2. `ScoredNode` um `distribution_scores: Option<Vec<f32>>` erweitern
3. Optional: Temperature-Parameter für Softmax (kontrolliert Entropie der Distribution)

```rust
// Nach RRF-Fusion (Z. 1173):
let scores: Vec<f32> = fused.iter().map(|(_, s)| *s).collect();
let max_score = scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
let exps: Vec<f32> = scores.iter().map(|s| (s - max_score).exp()).collect();
let sum: f32 = exps.iter().sum();
let distribution: Vec<f32> = exps.iter().map(|e| e / sum).collect();
// → distribution[i] = P(candidate_i | query)
```

**TST-Connection:** MCE's mean-over-bag ist mathematisch äquivalent zu einem softmax über multi-hot targets. KnowWhere's RRF ist bereits eine Form von Fusion — softmax macht daraus eine echte Distribution. Die Temperatur τ kontrolliert den Trade-off zwischen Precision (τ→0, wie MCE mit bag_size=1) und Recall (τ→∞, wie MCE mit bag_size=s).

**Industrie:** Elasticsearch/OpenSearch nutzen `min_score` threshold; Cohere/Weaviate bieten re-ranking scores; Qdrant hat `score_threshold`. Keiner bietet native softmax-Distribution im API-Response — KnowWhere wäre hier vorne dabei.

---

## 5. ConsolidationStore implementieren (~100 Zeilen)

**Status:** Trait definiert, NULL Implementationen.

### Was existiert:
- `ConsolidationStore` trait (Z. 284-297, `src/memory/dream/consolidation.rs`):
  - `get_episodic_memories_older_than(days) → Vec<ClusteringCandidate>`
  - `get_memories_by_ids(ids) → Vec<ConsolidationMemory>`
  - `create_summary_node(content, type, topic, importance) → Uuid`
  - `set_parent(memory_id, parent_id) → ()`
  - `archive(memory_id) → ()`
- `ConsolidationEngine<C: ConsolidationStore>` — komplett generisch, wartet auf C
- `run_consolidation()` — Algorithmus existiert (Z. 87-175), ruft Trait-Methoden auf

### Was NULL ist:
**KEINE** Struct implementiert `ConsolidationStore`. Zero. Der gesamte Dream-Consolidation-Pfad ist scaffolding.

### Was implementiert werden muss (~100 Zeilen):

**`InMemoryConsolidationStore`** — ein Wrapper um `MemoryStore`, der den Trait erfüllt:

1. `get_episodic_memories_older_than()` → durch `nodes` iterieren, `MemoryType::Episodic` + `created_at` filtern
2. `get_memories_by_ids()` → `nodes.get(id)` für jeden ID
3. `create_summary_node()` → **TST-kritisch**: Muss `mean_vector()` aufrufen (aus `fractal_node.rs`), nicht neu embedden!

```rust
// TST-Regel: L1-Node-Vektoren = mean_vector(kind_vektoren)
// NIEMALS neu embedden!
let child_vectors: Vec<&[f32]> = children.iter()
    .filter_map(|m| {
        let node = self.store.get(&m.id).await?;
        Some(node.vector.as_slice())
    })
    .collect();
let l1_vector = mean_vector(&child_vectors);  // ← TST bag-of-claims averaging
```

4. `set_parent()` → Node-Metadaten updaten, `parent_id` setzen
5. `archive()` → `MemoryStatus::Consolidated` setzen

### TST-Design-Rationale:
- **Phase 1 (billig):** `mean_vector()` auf semantischen Clustern → L1 parent nodes. Das ist die TST-Superposition-Phase: billige Mittelung, erhält 91% Coverage.
- **Phase 2 (teuer):** Nur auf bereits gebildeten L1-Nodes Präzisions-Verfeinerung (LLM-Summarization, Cross-Encoder Re-Ranking). Das ist die TST-Recovery-Phase.
- **Representation Continuity:** L1-Node-Vektoren MÜSSEN mean_vector(children) sein — nie neu embedden. TST's schärfste Ablation: Re-Initialisierung zwischen Phasen zerstört ALLE Gains.

### Implementierungs-Ansatz:
```rust
pub struct InMemoryConsolidationStore {
    store: Arc<MemoryStore>,
}

#[async_trait]
impl ConsolidationStore for InMemoryConsolidationStore {
    // 5 Methoden implementieren, ~100 Zeilen total
}
```

---

## Zusammenfassung & Priorisierung (unverändert)

| # | Task | Aufwand | Risk | Abhängigkeiten |
|---|------|---------|------|----------------|
| 1 | 3-Level-Cascade verdrahten | ~30 Zeilen | Niedrig | None (ultra_coarse_search existiert bereits) |
| 2 | 64d-Index-Rebuild in load_state | ~40 Zeilen | Mittel | PersistedState-Schema-Frage |
| 3 | User-ID-Filter fixen/dokumentieren | ~5 Zeilen | Niedrig | Entscheidung: None→true oder None→error |
| 4 | Distributional Scoring | ~15 Zeilen | Niedrig | ScoredNode struct erweitern |
| 5 | ConsolidationStore implementieren | ~100 Zeilen | Hoch | mean_vector() existiert, aber Design-Entscheidungen nötig |

## Referenzen

- **TST Paper:** arXiv 2605.06546 — https://arxiv.org/pdf/2605.06546
- **Nous Blog:** https://nousresearch.com/token-superposition
- **MarkTechPost Summary:** https://www.marktechpost.com/2026/05/13/nous-research-releases-token-superposition-training-to-speed-up-llm-pre-training-by-up-to-2-5x-across-270m-to-10b-parameter-models/
- **Multi-Tenant Vector DB Survey:** arXiv 2401.07119 — https://arxiv.org/html/2401.07119v1
- **Metadata Filtering Guide:** https://www.saumilsrivastava.ai/blog/metadata-filtering-in-vector-search-a-comprehensive-guide-for-engineering-leaders
- **KnowWhere Doku:** `knowwhere/docs/TST_KNOWWHERE_IMPLEMENTATION.md`
- **Code: retrieval_fractal:** `src/storage/in_memory.rs:1208-1289`
- **Code: load_state:** `src/storage/in_memory.rs:467-535`
- **Code: ConsolidationStore trait:** `src/memory/dream/consolidation.rs:284-297`
- **Code: fractal_node helpers:** `src/memory/fractal_node.rs:36-81`

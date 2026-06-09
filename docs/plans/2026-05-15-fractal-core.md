# Fractal-Core: Matryoshka + Multi-Query Retrieval

> **Für Hermes:** Nutze `subagent-driven-development` Skill um diesen Plan task-by-task umzusetzen.

**Goal:** LLM-Summarization und TTL-basierte L0-Löschung durch Matryoshka-Resolution-Hierarchie und Multi-Query-Retrieval ersetzen. Die Fraktal-Struktur entsteht direkt aus der Embedding-Geometrie.

**Architektur:** Zwei HNSW-Indices (256d coarse + 768d fine). Retrieval via Multi-Query-Expansion (2-3 Reformulierungen pro Query) → Matryoshka-Zoom (coarse→fine) → BM25 → RRF-Fusion. K-Means-Clustering als optionale Navigationshilfe (>500 Nodes).

**Tech Stack:** Rust, USearch (HNSW), BM25, Ollama (nomic-embed-text v1.5), serde, tokio, RRF k=5

**Branch:** `fractal-core` (neu von `main`)

---

## Vorab: Aufräumen und Vorbereiten

### Task 0.1: Branch erstellen und sauberen Startpunkt schaffen

**Objective:** Frischer Branch von `main`, uncommittete Changes stashen oder committen.

```bash
cd ~/knowwhere
git stash
git checkout -b fractal-core
git stash pop  # optional: nur wenn Changes relevant
```

### Task 0.2: Aktuellen State dokumentieren

**Objective:** Baseline für Vergleich nach Umbau.

```bash
curl -s http://localhost:3737/health
# Notieren: node_count

cargo test --lib 2>&1 | tail -3
# Notieren: test count, failures
```

---

## Phase 1: Totes entfernen (LLM-Summarization, TTL)

### Task 1.1: L0-TTL aus FractalNode entfernen

**Objective:** `expires_at`-Logik aus Storage entfernen. L0-Nodes sind permanent.

**Files:**
- Modify: `src/memory/types.rs` — `suggested_refresh_days` auf `None` setzen, `expires_at`-Default entfernen
- Modify: `src/storage/audit.rs` — `evict_expired()` zu No-Op machen (Funktion behalten, leeren Body)

**Verification:**
```bash
# Store Node mit TTL-freiem FractalNode
curl -X POST http://localhost:3737/store_external \
  -H "Authorization: Bearer kw_testkey_12345" \
  -H "Content-Type: application/json" \
  -d '{"pointer":"ttl-test","content":"Dieser Node soll niemals ablaufen.","metadata":{"type":"test"}}'
# → 201, node hat kein expires_at
```

### Task 1.2: ConsolidationScheduler stilllegen

**Objective:** Dream-Pipeline deaktivieren. Kein automatisches L2→L1→L0.

**Files:**
- Modify: `src/scheduler/consolidation.rs` — `ConsolidationScheduler::run()` zu No-Op
- Modify: `src/main.rs` — Consolidation-Worker-Start auskommentieren oder hinter Feature-Flag `consolidation` legen

**Verification:**
```bash
curl -s http://localhost:3737/dream/status -H "Authorization: Bearer kw_testkey_12345"
# → active: false ODER consolidations_run bleibt 0
```

### Task 1.3: LLM-Summarizer-Pfad kappen

**Objective:** `summarizer/mod.rs` und `vlm/mod.rs` hinter Feature-Flag `summarizer` legen. Default-Build ohne.

**Files:**
- Modify: `Cargo.toml` — `summarizer` Feature optional machen
- Modify: `src/main.rs` — Summarizer-Init mit `#[cfg(feature = "summarizer")]` gaten
- Modify: `src/api/routes.rs` — `/vlm/summarize` Endpoint nur mit Feature

**Verification:**
```bash
cargo build --release 2>&1
# → Keine Compile-Fehler, summarizer-Code nicht gelinkt
```

---

## Phase 2: Embedding-Modell evaluieren und wechseln

### Task 2.1: nomic-embed-text v1.5 in Ollama pullen

**Objective:** Das nicht-trunkierende Modell bereitstellen.

```bash
ollama pull nomic-embed-text:latest
# Prüfen: context length, Matryoshka-Support
ollama show nomic-embed-text:latest | grep -i "context\|matryoshka\|dimension"
```

### Task 2.2: EmbeddingProvider auf Matryoshka-Support prüfen

**Objective:** API-Call-Test: produziert das Modell valide truncated Embeddings?

**Files:**
- Create: `scripts/test_matryoshka.py`

**Script:**
```python
import requests, json, numpy as np

# Test: full 768d vs truncated 256d similarity
texts = [
    "Redis zum Cachen von User-Sessions",
    "Redis als Message-Queue für Jobs",
    "PostgreSQL als Message-Queue nutzen",
    "Ein komplett anderes Thema über Steuererklärung",
    "Noch ein Steuerthema: Freibetrag 2026"
]

# Embed via Ollama
resp = requests.post("http://localhost:11434/api/embed",
    json={"model": "nomic-embed-text:latest", "input": texts})
embeddings = [np.array(e) for e in resp.json()["embeddings"]]

# Vergleiche full 768d cosine vs truncated 256d cosine
def cos(a, b): return np.dot(a,b)/(np.linalg.norm(a)*np.linalg.norm(b))

for i in range(len(texts)):
    for j in range(i+1, len(texts)):
        full = cos(embeddings[i][:768], embeddings[j][:768])
        trunc = cos(embeddings[i][:256], embeddings[j][:256])
        ratio = trunc/full if full != 0 else 0
        print(f"{i}↔{j}: full={full:.4f} trunc={trunc:.4f} ratio={ratio:.2f}")
```

**Erwartet:** Truncated-Similarities proportional zu Full-Similarities (ratio konsistent). Wenn ratio willkürlich → Matryoshka NICHT unterstützt → Alternative: K-Means auf 768d als Cluster-Hierarchie.

### Task 2.3: KNOWWHERE_EMBEDDING_MODEL umstellen

**Objective:** `.env` auf `nomic-embed-text` umstellen, Server damit starten.

**Files:**
- Modify: `~/knowwhere/.env` — `OLLAMA_MODEL=nomic-embed-text:latest`
- Modify: `src/embedding/provider.rs` — dimension() auf dynamisch (Modell-Abfrage) oder hart auf 768

**Verification:**
```bash
# RESTART server mit neuem .env
# Smoke-Test
curl -X POST http://localhost:3737/store_external \
  -H "Authorization: Bearer kw_testkey_12345" \
  -H "Content-Type: application/json" \
  -d '{"pointer":"test","content":"Test mit nomic-embed-text v1.5.","metadata":{"type":"test"}}'
# → 201
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer kw_testkey_12345" \
  -H "Content-Type: application/json" \
  -d '{"query_text":"Test","max_results":3}'
# → Ergebnis mit Score >0
```

---

## Phase 3: Matryoshka Dual-Index

### Task 3.1: MemoryStore um zweiten HNSW-Index erweitern

**Objective:** `SendableIndex` für 256d coarse zusätzlich zum existierenden 768d fine Index.

**Files:**
- Modify: `src/storage/in_memory.rs`

**Änderungen:**
```rust
// Bestehend:
usearch_index: Arc<Mutex<Option<SendableIndex>>>,
index_dimension: Arc<Mutex<Option<usize>>>,

// NEU:
coarse_usearch_index: Arc<Mutex<Option<SendableIndex>>>,
coarse_index_dimension: Arc<Mutex<Option<usize>>>,
const COARSE_DIM: usize = 256;
```

**Methoden:**
- `ensure_coarse_index(&self, dimension=256) -> Result<()>` — analog zu `ensure_index`
- `insert()` — beide Indices befüllen: full-768d → fine, truncated-256d → coarse

### Task 3.2: Matryoshka-Zoom in hybrid_retrieve integrieren

**Objective:** Two-Stage Retrieval: coarse (256d, k=50) → fine (768d, k=10) nur innerhalb der Coarse-Kandidaten.

**Files:**
- Modify: `src/storage/in_memory.rs`

**Neue Methode:**
```rust
pub async fn matryoshka_retrieve(
    &self,
    query_vector: &[f32], // 768d
    coarse_top_k: usize,   // 50
    fine_top_k: usize,     // 10
    max_depth: usize,
) -> Vec<(f32, FractalNode)> {
    // Stage 1: Coarse search
    let query_256 = &query_vector[..256];
    let coarse_ids = self.coarse_search(query_256, coarse_top_k).await;

    // Stage 2: Fine search (nur in coarse_ids)
    let fine_results = self.fine_search_in(query_vector, &coarse_ids, fine_top_k, max_depth).await;

    fine_results
}
```

**Integration in `hybrid_retrieve`:**
- Wenn `query_vector` vorhanden: `matryoshka_retrieve` statt `retrieve_fractal` (flat)
- BM25 parallel wie gehabt
- RRF-Fusion über Matryoshka + BM25

### Task 3.3: Matryoshka-Retrieval testen

**Objective:** Verifizieren: Grobsuche + Feinsuche findet korrekte Results.

**Test-Script:**
```bash
# 10 diverse Nodes speichern (Tools, Patterns, Domains)
# Query "Build-Problem" → sollte coarse in die richtige Region lenken, fine exakten Node finden
```

---

## Phase 4: Multi-Query-Expansion

### Task 4.1: Template-basierte Query-Expansion

**Objective:** Aus einem Query 2-3 Reformulierungen generieren — regelbasiert.

**Files:**
- Create: `src/retrieval/query_expansion.rs`
- Create: `src/retrieval/mod.rs`

**Logik:**
```rust
pub fn expand_query(query: &str) -> Vec<String> {
    let mut expanded = vec![query.to_string()];

    // 1. Keyword-Extraktion: Nomen + Verben extrahieren
    let keywords = extract_key_nouns_verbs(query);

    // 2. Broadening: "Redis als Queue" → "Message-Queue-Systeme und Tools"
    if keywords.len() >= 1 {
        expanded.push(format!("{} Systeme und Konfigurationen", keywords.join(" ")));
    }

    // 3. Narrowing: Konkreter machen
    if keywords.len() >= 2 {
        expanded.push(keywords.join(" "));
    }

    expanded.dedup();
    expanded.truncate(3);
    expanded
}
```

### Task 4.2: Multi-Query-Retrieval in retrieve_fractal integrieren

**Objective:** `POST /retrieve_fractal` führt Matryoshka-Zoom für JEDEN expandierten Query aus, fusioniert alle Results via RRF.

**Files:**
- Modify: `src/api/routes.rs` — `retrieve_fractal` Handler
- Modify: `src/storage/in_memory.rs` — neue Methode `multi_query_retrieve`

**Flow:**
```
1. expand_query(query_text) → [q1, q2, q3]
2. Für jedes qi:
     embed_query(qi) → vector_768
     matryoshka_retrieve(vector_768) → results_i
3. Alle results_i ∪ → RRF-Fusion über combined set
4. Top-K zurück
```

### Task 4.3: Multi-Query testen

**Objective:** Redis/Queue-Szenario — B erscheint in mehreren Query-Perspektiven, bekommt höheren fused score.

**Test-Script:**
```bash
# A: Redis/Cache, B: Redis/Queue, C: Postgres/Queue speichern
# Query "Message-Queue-Lösungen"
# Erwartet: B und C in Top-3, B hat höheren Score als C (weil in mehr Perspektiven)
```

---

## Phase 5: Eval-Harness

### Task 5.1: Retrieval-Quality-Harness bauen

**Objective:** Reproduzierbare Evaluation mit Ground-Truth-Erwartungen.

**Files:**
- Create: `scripts/eval_retrieval.py`

**Struktur:**
```python
TEST_CASES = [
    {
        "query": "Build-Problem M1",
        "relevant_node_content": "cargo build hängt bei 47%",
        "min_expected_rank": 3,  # sollte in Top-3 sein
    },
    # ... 20 Fälle
]

def eval_retrieval(endpoint, token):
    for case in TEST_CASES:
        results = query(case["query"])
        rank = find_rank(case["relevant_node_content"], results)
        passed = rank <= case["min_expected_rank"]
        # Log: query, rank, score, passed
    # Metrics: Recall@10, MRR, Precision@5
```

**Verification:**
```bash
python scripts/eval_retrieval.py
# → Recall@10: X%, MRR: Y.YY
```

### Task 5.2: Baseline messen (VOR Matryoshka+Multi-Query)

**Objective:** Aktuelle Retrieval-Qualität erfassen als Vergleichswert.

```bash
python scripts/eval_retrieval.py --baseline > eval_baseline.txt
```

---

## Phase 6: Integration und Härtung

### Task 6.1: Server-Konfiguration für Dual-Index

**Objective:** KNOWWHERE_COARSE_DIM env var (default 256), KNOWWHERE_COARSE_TOP_K (default 50).

**Files:**
- Modify: `src/main.rs` — Config aus env lesen
- Modify: `src/storage/in_memory.rs` — coarse_dim aus Config

### Task 6.2: Index-Rebuild nach Server-Neustart

**Objective:** Beim Laden von `state.json` beide Indices (coarse + fine) aus gespeicherten Nodes neu aufbauen.

**Files:**
- Modify: `src/storage/in_memory.rs` — `load_from_disk()` um coarse-Index-Rebuild erweitern

### Task 6.3: Full Test Suite

```bash
cargo test --lib
cargo test --test integration --features postgres-storage
# → Alle Tests grün
```

### Task 6.4: Performance-Messung

**Objective:** P50/P99 Latenz für single-query vs multi-query.

```bash
# 100 Queries, Latenz messen
python scripts/bench_latency.py
# Erwartet: P50 < 300ms für Multi-Query, P50 < 150ms für Single-Query
```

---

## Phase 7: Optional — K-Means-Clustering als Navigation

### Task 7.1: K-Means auf 256d Embeddings

**Objective:** Periodisches Clustering (alle 50 Inserts), Cluster-Labels als Navigations-Metadaten.

**Files:**
- Create: `src/retrieval/cluster_nav.rs`

**Dependencies:**
- `linfa-clustering` in Cargo.toml

**Logik:**
```rust
pub struct ClusterNavigator {
    centroids: Vec<Vec<f32>>,  // k × 256
    labels: Vec<String>,        // "Build+Tools", "Redis+Cache", ...
    k: usize,
}

impl ClusterNavigator {
    pub fn recluster(&mut self, nodes: &[FractalNode]) { /* K-Means */ }
    pub fn find_cluster(&self, query_256: &[f32]) -> usize { /* nearest centroid */ }
    pub fn cluster_members(&self, cluster_id: usize) -> Vec<Uuid> { /* members */ }
}
```

### Task 7.2: Cluster-Abfrage-Endpoint

**Objective:** `POST /retrieve_fractal` mit `?cluster_scope=true` → nur im Cluster der Query suchen.

**Files:**
- Modify: `src/api/routes.rs`

### Task 7.3: Cluster-Rebuild-Trigger

**Objective:** Nach jedem 50. Insert → `cluster_nav.recluster(all_nodes)`.

**Files:**
- Modify: `src/storage/in_memory.rs` — Counter in `insert()`

---

## Phase 8: Dokumentation und Cleanup

### Task 8.1: ARCHITECTURE.md aktualisieren

**Objective:** Neue Architektur dokumentieren: Matryoshka-Zoom, Multi-Query, Cluster-Navigation.

### Task 8.2: README.md updaten

**Objective:** Quickstart, API-Änderungen, neue env vars dokumentieren.

### Task 8.3: CHANGELOG.md

**Objective:** Was wurde entfernt, was hinzugefügt, Breaking Changes.

---

## Erfolgskriterien (alle MÜSSEN wahr sein)

1. ✅ `cargo build --release` ohne Fehler (kein summarizer, kein consolidation-code gelinkt)
2. ✅ `cargo test --lib` — alle Tests grün
3. ✅ Zwei HNSW-Indices (256d + 768d) aktiv und durchsuchbar
4. ✅ Multi-Query-Retrieval liefert Results für "Redis als Queue" mit B und C in Top-5
5. ✅ Eval-Harness zeigt verbesserte Recall@10 gegenüber Baseline
6. ✅ P50 Latenz < 300ms für Multi-Query
7. ✅ Kein LLM-Summarizer-Code im Default-Build
8. ✅ L0-Nodes permanent (kein TTL)

---

## Risiken

| Risiko | Mitigation |
|--------|-----------|
| `nomic-embed-text` kein Matryoshka-Support | Fallback: K-Means auf 768d als Cluster-Hierarchie |
| Dual-Index sprengt 8GB RAM | Coarse-Index nur bei >500 Nodes aktivieren |
| Multi-Query 3× langsamer | Parallelisierung via tokio::spawn |
| PCA-Achsen-Test (optional) liefert Rauschen | Erwartet. Dokumentieren, nicht darauf bauen |

# KnowWhere v0.5 Roadmap — Von Nische zu Marktführer

**Datum:** 2026-05-06
**Autor:** Hermes + Nimar
**Status:** In Ausführung

---

## First Principles: Was macht ein Memory-System zum Marktführer?

1. **Retrieval-Qualität** — Findet es die richtige Information, wenn sie gebraucht wird?
2. **Strukturtiefe** — Speichert es nicht nur Fakten, sondern Entscheidungen, Kausalität, Provenienz?
3. **Skalierbarkeit** — Funktioniert es bei 1.000, 100.000, 10.000.000 Nodes?
4. **Beweisbarkeit** — Kann es seine Qualität reproduzierbar und transparent nachweisen?
5. **Minimalität** — Tut es mehr mit weniger Komplexität als die Konkurrenz?

## Die vier Baustellen

| # | Baustelle | Was | Impact | Aufwand |
|---|---|---|---|---|
| 1 | **Cross-Encoder Reranking** | bge-reranker-v2-m3 ONNX statt MiniLM | +33-42% Precision | 2h |
| 2 | **Fractal Zoom Default** | PG children_tier_ids + expand_fractal + adaptive Fallback | O(log N) statt O(N), Zukunftssicherheit | 1h |
| 3 | **Entity-Layer** | Leichtgewichtige Entitäten-Relationen via PG edge-Tabelle | Neue Query-Klasse (Tools, Modelle, Personen) | 3h |
| 4 | **AMB Benchmark** | Hindsight's Agent Memory Benchmark + KnowWhere-Backend | Glaubwürdigkeit, reproduzierbare Qualität | 4h |

---

## 1. Cross-Encoder Reranking

### First Principles

**Problem:** Bi-encoder (embedding) kann semantische Ähnlichkeit messen, aber keine präzisen Relevanzurteile fällen. "OLLAMA_URL must be 127.0.0.1" ist semantisch nah an "Docker container networking" — aber das erste ist eine spezifische macOS-Falle, das zweite generisch. Ein Cross-Encoder liest Query+Document *gemeinsam* und kann diese Unterscheidung treffen.

**Warum bge-reranker-v2-m3:**
- State-of-the-Art multilingual (DE+EN)
- 568M Params — fit für M1/8GB
- ONNX-Community hat den Export bereits gemacht: `onnx-community/bge-reranker-v2-m3-ONNX`
- Integriert sich in existierenden Code (`src/retrieval/cross_encoder.rs`, 491 Zeilen, feature-gated `reranker`)

### Umsetzungsplan

**Step 1:** Modell downloaden
```bash
# Via HuggingFace CLI
huggingface-cli download onnx-community/bge-reranker-v2-m3-ONNX \
  --local-dir ./models/bge-reranker-v2-m3-onnx
```

**Step 2:** Code-Pfad aktualisieren
- `cross_encoder.rs:129`: Tokenizer-Pfad auf neues Modell
- `cross_encoder.rs:145`: ONNX-Modell-Pfad
- `cross_encoder.rs:89`: Input-Format prüfen (`[CLS] query [SEP] doc [SEP]` für bge-reranker)

**Step 3:** `/rerank` Endpoint testen
```bash
curl -X POST "http://127.0.0.1:3737/rerank" \
  -d '{"query":"event driven consolidation trigger",
       "candidates":[{"node_id":"1","content":"Event-driven triggers after store_session"},
                     {"node_id":"2","content":"Docker containers need volume mounts"}],
       "top_k":2}'
# Ziel: node_1 score >> node_2 score (~0.8 vs ~0.1)
```

**Step 4:** In Retrieval-Pipeline integrieren
- `retrieve_fractal` → `hybrid_retrieve` → `expand_fractal` → **`rerank`** → governance
- Feature-Flag `reranker` in Binary aktivieren (bereits in Cargo.toml)

**Fallback:** Falls ONNX-Tokenizer inkompatibel → Ollama `qllama/bge-reranker-v2-m3` (636 MB, API-Call).

### Qualitätsmetrik
- Vorher (bi-encoder only): 92% Queries mit Decision in Top-5
- Nachher (cross-encoder rerank): Ziel ≥95% + höhere Top-1-Präzision

---

## 2. Fractal Zoom als Default

### First Principles

**Was ist Fractal Zoom?** Ein hierarchischer Suchindex — wie ein B-Tree für Vektoren. Statt alle N Nodes zu scannen, startet die Suche bei L2-Overviews (wenige Nodes), zoomt zu L1-Kindern wenn cosine ≥ 0.7, und zu L0-Rohdaten wenn relevant. Branches unter der Schwelle werden abgeschnitten.

**Warum jetzt, nicht später?** Bei 3.280 Nodes ist flat scan (20ms) schneller als Zoom (15ms Overhead). Aber: Wenn wir Zoom erst bei 100K Nodes aktivieren, entdecken wir Bugs bei 100K Nodes — wo Debugging teuer ist. Jetzt aktivieren = Bugs bei 3K Nodes entdecken = nahezu kostenlos.

**Die PG-Lücke:** `expand_fractal()` ist in `PostgresStore` ein NO-OP (trait default). Das `children_tier_ids` Feld existiert nicht im PostgreSQL-Schema. Ohne Fix funktioniert Zoom nur im MemoryStore — also nie auf dem Production-Server.

### Umsetzungsplan

**Step 1:** PG-Schema erweitern
```sql
ALTER TABLE memories ADD COLUMN IF NOT EXISTS children_tier_ids UUID[] DEFAULT '{}';
CREATE INDEX IF NOT EXISTS idx_memories_children ON memories USING GIN (children_tier_ids);
```

**Step 2:** `memory_row_to_fractal_node` patchen
- `children_tier_ids` aus MemoryRow auslesen und in FractalNode schreiben
- Aktuell: alle Tier-Felder auf defaults gehackt → FIX

**Step 3:** `PostgresStore::expand_fractal()` implementieren
```rust
fn expand_fractal(&self, nodes: Vec<ScoredNode>, query_vector: &[f32]) -> Vec<ScoredNode> {
    // 1. Für jeden Node mit non-empty children_tier_ids:
    //    SELECT * FROM memories WHERE id = ANY($1)
    // 2. Cosine similarity gegen query_vector berechnen
    // 3. Bei ≥0.7: Child inkludieren, rekursiv expandieren
    // 4. Bridge-Expansion: parent_tier_id climb für nodes ohne Kinder
    // 5. Dedup via visited-HashSet
}
```

**Step 4:** Adaptive Fallback
```rust
fn retrieve_with_zoom(query) -> Vec<ScoredNode> {
    let zoom_results = fractal_zoom(query);
    if zoom_results.is_empty() || zoom_results[0].score < 0.15 {
        // Pruning too aggressive — fall back to guaranteed optimal
        return flat_hybrid_retrieve(query);
    }
    zoom_results
}
```

**Step 5:** Zoom als Default in `retrieve_fractal` Endpoint
- `hybrid_retrieve` → `expand_fractal` → rerank → governance
- Pruning-Threshold: 0.7 (konservativ — priorisiert Recall über Speed bei <10K Nodes)

### Qualitätsmetrik
- Recall@5 vorher/nachher: darf nicht sinken
- Latency bei 3.280 Nodes: darf nicht >50ms steigen
- Bei simulierten 100K Nodes: muss <100ms bleiben

---

## 3. Entity-Layer

### First Principles

**Problem:** Queries wie "Welche Tools nutzen wir?" oder "Welche Modelle haben wir getestet?" können nicht allein durch semantische Ähnlichkeit beantwortet werden. "qwen2.5" und "nomic-embed-text-v2-moe" sind semantisch ähnlich (beides Modelle) — aber die Query will die *Liste aller getesteten Modelle*, nicht das ähnlichste Modell.

**Warum kein voller Knowledge Graph (Neo4j, Apache TinkerPop)?**
→ Memanto-Paper (arXiv 2604.22085, April 2026): "Surpasses all evaluated hybrid graph+vector systems using only vector search + structured memory typing." Ein voller Graph ist Overhead. Wir brauchen nur Entity-Relationen für spezifische Query-Klassen.

**Ansatz:** Leichtgewichtiger Entity-Layer über PostgreSQL JSONB — keine neue Infrastruktur.

### Umsetzungsplan

**Step 1:** Schema
```sql
CREATE TABLE entity_edges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_node_id UUID NOT NULL REFERENCES memories(id),
    target_node_id UUID REFERENCES memories(id),
    entity_type TEXT NOT NULL,       -- 'tool', 'model', 'person', 'project'
    entity_name TEXT NOT NULL,       -- 'qwen2.5:3b', 'Ollama', 'PostgreSQL'
    relation_type TEXT NOT NULL,     -- 'uses', 'tested', 'chose', 'depends_on'
    confidence FLOAT DEFAULT 1.0,
    extracted_at TIMESTAMPTZ DEFAULT now(),
    metadata JSONB DEFAULT '{}'
);
CREATE INDEX idx_entity_edges_type ON entity_edges(entity_type, relation_type);
CREATE INDEX idx_entity_edges_name ON entity_edges(entity_name);
```

**Step 2:** Summarizer-Prompt erweitern
```
Zusätzlich zu Entscheidungs-Claims, extrahiere Entitäten mit Relationen:
{
  "entities": [
    {"type": "model", "name": "qwen2.5:3b", "relation": "chose", "context": "Replaced llama3.2 due to 92.1% IF score"},
    {"type": "tool", "name": "pgvector", "relation": "uses", "context": "HNSW index for vector search"}
  ]
}
```

**Step 3:** Query-Routing
- Intent-Detektor erkennt "Welche [Typ]?" → triggert Entity-Query
- Entity-Query: `SELECT * FROM entity_edges WHERE entity_type = 'model' AND relation_type = 'tested'`
- Ergebnisse mergen mit semantischer Suche

### Qualitätsmetrik
- "Welche Modelle wurden getestet?" → ≥3 Modelle in Top-5
- "Welche Tools nutzen wir?" → ≥3 Tools in Top-5

---

## 4. AMB Benchmark

### First Principles

**Problem existierender Benchmarks:**
- **LOCOMO:** 6.4% Answer-Key-Fehler, LLM-Judge akzeptiert 63% falscher Antworten → **KAPUTT**
- **LongMemEval:** Fragen passen in moderne Kontextfenster → **Context-Window-Test, kein Memory-Test**
- **LongMemEval-S:** Besser, aber immer noch Context-beeinflusst

**Lösung:** Hindsight's Agent Memory Benchmark (AMB). Open Source, misst 4 Dimensionen (Accuracy + Efficiency + Simplicity + Explainability), nicht an einen spezifischen Backend gekoppelt.

**Zusätzlich:** KnowWhere's eigene Golden Queries (12 Stück, Produktions-Queries mit Intent-Tags) als ergänzenden Datensatz publizieren. Das sind *echte* Queries aus dem täglichen Gebrauch — kein synthetischer Benchmark.

### Umsetzungsplan

**Step 1:** AMB-Harness studieren + clonen
```bash
git clone https://github.com/vectorize-io/agent-memory-benchmark
```

**Step 2:** KnowWhere-Backend-Adapter schreiben
- Implementiert AMB's MemoryBackend-Trait
- Nutzt `/store_session` und `/retrieve_fractal`
- Konfigurierbar: mit/ohne Cross-Encoder, mit/ohne Zoom

**Step 3:** Benchmarks durchführen
- LOCOMO-Datensatz (trotz Fehlern — für Vergleichbarkeit)
- LongMemEval-S-Datensatz
- KnowWhere Golden Queries (12 Stück)

**Step 4:** Ergebnisse in `BENCHMARKS.md` publizieren
- Rohdaten: pro Query, Score, Latenz
- Aggregiert: Accuracy, Recall@5, MRR, Latency p50/p95
- Vergleich: KnowWhere vs. Mem0 vs. Hindsight (soweit öffentliche Daten verfügbar)

### Qualitätsmetrik
- AMB Score > Mem0 (öffentliche Benchmarks)
- Recall@5 ≥ 0.95 auf KnowWhere Golden Queries
- P50 Latenz < 50ms

---

## Ausführungsreihenfolge

1. **Cross-Encoder** (zuerst — größter Hebel, geringster Aufwand)
2. **Fractal Zoom Default** (direkt danach — PG-Fix ist Vorbedingung für alles andere)
3. **Entity-Layer** (baut auf Zoom + Cross-Encoder auf)
4. **AMB Benchmark** (misst das fertige System)

Jeder Schritt wird getestet und committed, bevor der nächste beginnt. Kein Big-Bang-Deploy.

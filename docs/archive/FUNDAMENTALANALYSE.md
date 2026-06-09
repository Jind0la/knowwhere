# KnowWhere Fundamentalanalyse
## Stand: 9. Mai 2026 — Deep Dive nach 3 Parallel-Analysen über 28K Lines Rust

---

## 0. Executive Summary

KnowWhere ist eine **technisch beeindruckende Memory-Maschine** mit 50 Rust-Dateien, 6 Memory-Typen, Governance-Validierung, Event-Sourcing, VLM-Fallback-Chain und Cross-Modal-Embedding. Aber sie hat einen **fundamentalen blinden Fleck**: Sie speichert Fakten als isolierte Atome ohne zeitliche Verknüpfung. Eine Konversation über 100 Turns wird zu 100 unabhängigen L2-Nodes — ohne `SEQUENCE`, `NEXT`, `BEFORE` oder `AFTER`. Das ist der Grund, warum wir bei PersonaMem nicht über 50% kommen: Die Fragen testen temporale Präferenz-Evolution, aber KnowWhere hat kein Konzept von Zeit.

---

## 1. Was KnowWhere HEUTE kann (und zwar richtig gut)

### 1.1 Die Architektur — 50 Dateien, 28K Lines, 6 Subsysteme

```
knowwhere-server v0.5.0
├── src/api/          6,633 Zeilen  — HTTP (Axum), Auth, Swagger
├── src/memory/       7,008 Zeilen  — CORE: FractalNode, Typen, Governance, Dream Mode
├── src/storage/      4,077 Zeilen  — Dual: InMemory (USearch) + PostgreSQL (pgvector)
├── src/embedding/    1,367 Zeilen  — Ollama, OpenAI, Grok, CLIP, Whisper
├── src/scheduler/    1,554 Zeilen  — Konsolidierung, Audit, Intervall-Timer
├── src/vlm/          1,087 Zeilen  — 4-Stage Fallback (GPT→Grok→Ollama)
├── src/summarizer/     555 Zeilen  — Ollama-basierte L2→L1→L0 Verdichtung
├── src/retrieval/      501 Zeilen  — Cross-Encoder Reranking (feature-gated)
└── src/reflector/      226 Zeilen  — Selbstreflexion vor Antworten
```

### 1.2 Die 6 Memory-Typen — gut durchdacht

| Typ | Halbwertszeit | Default Importance | Zweck |
|-----|-------------|-------------------|-------|
| **Episodic** | 7 Tage | 5 | Konversations-Rohdaten |
| **Semantic** | 90 Tage | 6 | Fakten, Wissen |
| **Preference** | 30 Tage | 7 | Präferenzen, "mögen" |
| **Procedural** | 180 Tage | 8 | Workflows, Regeln |
| **Meta** | 14 Tage | 4 | System-Metadaten |
| **Decision** | **Unsterblich** | 9 | Entscheidungen mit Begründung |

Jeder Typ hat eigene Confidence-Defaults, Halbwertszeiten und Governance-Regeln. Das ist solide.

### 1.3 Die Konsolidierungs-Pipeline — technisch korrekt

Der `ConsolidationScheduler` läuft als Hintergrund-Task und verdichtet L2→L1→L0:

```
L2 (Raw, 500+ chars)
  │
  ├─ Ollama (qwen2.5:3b) → L1 Overview (100-300 Wörter)
  │   └─ Extrahiert Claims: {claim: "…", reason: "…"}
  │       └─ Claims werden eigene Decision-Nodes
  │
  └─ Ollama (1-Satz) → L0 Summary (≤20 Wörter)
      └─ Bidirektionale Links: parent_tier_id, children_tier_ids
```

Die L0→L1→L2-Kette ist sauber mit UUID-basierten Parent/Child-Links implementiert. L0 hat sogar einen Skip-Link direkt zu L2 (`children_tier_ids`).

### 1.4 Der Retrieval-Pipeline — 5 Stufen

```
POST /retrieve_fractal
  │
  ├─ embed_query("search_query: {text}") → Ollama → Vec<f32>(768)
  ├─ STAGE 1: Hybrid Search
  │   ├─ USearch HNSW (cosine) × fetch_k (3× Oversampling)
  │   ├─ BM25 (German) × 0.2/(60+rank+1) Soft-Boost
  │   └─ Fuse: cosine + bm25_boost
  ├─ STAGE 1.5: Fractal Expansion (UUID-Link-Traversal)
  ├─ STAGE 2: Governance Validation
  │   ├─ Confidence-Check, Supersession-Block, Sensitivity-Block
  │   ├─ Staleness-Penalty, Conflict-Penalty
  │   └─ Multiplikative Score-Modulation
  ├─ Intent Scoring (bis 1.9× für Type-Matching)
  ├─ Evidence Deduplication
  ├─ MMR Diversification (λ=0.65)
  └─ Optional: Reflection Synthesis
```

Das ist **kein einfaches "finde ähnliche Vektoren"**. Das ist eine 5-stufige Pipeline mit Oversampling, fractal zoom, governance, intent matching und MMR. Technisch exzellent.

---

## 2. Der blinde Fleck: Zeit existiert nicht

### 2.1 Was fehlt

KnowWhere's Datenmodell hat **keinen Zeitpfeil**:

1. **Keine Sequenz zwischen Nodes**: `children_tier_ids` ist ein `Vec<Uuid>` — ungeordnet. Es gibt keine `SEQUENCE`, `NEXT`, `BEFORE`, `AFTER` Relation.
2. **Keine Turn-Reihenfolge**: `consolidation_metadata()` kopiert `turn_index` nur als Skalar — nicht als Bereich. Wenn 10 Turns in einer Session konsolidiert werden, weiß niemand, dass Turn 3 VOR Turn 7 kam.
3. **Keine Session-Level-Konsolidierung**: Jeder L2-Node wird einzeln zu L1→L0 verdichtet. Es gibt keine "fasse diese ganze Session zu einer Timeline zusammen"-Logik.
4. **Keine temporalen Edges**: Das Graph-System (`Relation { target_id, relation_type, strength }`) hat keine `relation_type = "next"` oder `"precedes"`.
5. **Kein `created_at`-Erbe**: Konsolidierte Nodes bekommen `Utc::now()` — die originale Zeit geht verloren.

### 2.2 Warum das für PersonaMem fatal ist

PersonaMem-Fragen sind **keine Fakten-Recall-Fragen**. Sie sind **Temporal-Reasoning-Fragen**:

> *"I found that my reviews were often criticized for not being objective. After several disagreements, I felt constrained..."*

Die MCQ-Optionen sind 4 lange Paragraphen, die jeweils eine andere **Präferenz-Timeline** beschreiben:
- (a) "Initially you were not interested... then enjoyed... finally returned to disliking..."
- (b) "You've always been passionate... never wavered..."
- (c) "Your interest grew slowly... plateaued..."
- (d) "You started strong... gradually lost interest..."

Um das zu beantworten, braucht der LLM NICHT isolierte Fakten wie "Kanoa Manu enjoys music theory". Er braucht eine **chronologische Sequenz**:

```
Turn 12: "I don't really care about album reviews"       → dislikes reviews
Turn 34: "Started writing reviews, actually enjoy this"   → enjoys reviews
Turn 67: "Criticism is getting to me, reviews are a chore" → dislikes again
```

Wenn KnowWhere diese Turns als 3 unabhängige Nodes speichert und per Cosine-Similarity abruft, bekommt der LLM:

```
Claim: "Kanoa Manu enjoys writing album reviews"
Claim: "Kanoa Manu dislikes album reviews"
Claim: "Kanoa Manu finds album reviews a chore"
```

Drei widersprüchliche Claims — ohne Zeitstempel. Der LLM kann daraus keine Timeline rekonstruieren.

### 2.3 Der architektonische Gap in der Konsolidierung

Der `ConsolidationScheduler` verarbeitet Nodes einzeln:

```rust
// src/scheduler/consolidation.rs:549-744
for node in candidates {
    let l1 = summarize(&node);       // Eine L1 pro L2
    let claims = extract_claims(l1); // Claims aus L1
    let l0 = summarize_short(l1);    // L0 aus L1
    // Keine Batch-Logik, keine Session-Gruppierung
}
```

Was fehlt, ist eine **Session-Level-Aggregation**:

```rust
// Was wir brauchen:
for (session_id, nodes) in group_by_session(candidates) {
    nodes.sort_by(|a, b| a.turn_index.cmp(&b.turn_index)); // ← GIBT'S NICHT
    let timeline = build_timeline(nodes);                    // ← GIBT'S NICHT
    let narrative = summarize_timeline(timeline);            // ← GIBT'S NICHT
}
```

---

## 3. Was KnowWhere WIRKLICH braucht

### 3.1 Das große Bild: Drei fundamentale Lücken

| Lücke | Symptom | Ursache | Impact |
|-------|---------|---------|--------|
| **Temporale Struktur** | Keine Timeline-Rekonstruktion | `Vec<Uuid>` statt geordneter Liste | PersonaMem-50%-Decke |
| **Session-Level-Konsolidierung** | Isolierte L2→L1 Verdichtung | `for node in candidates` ohne Gruppierung | Keine Narrative möglich |
| **Semantische Abstraktionsebene** | Claims sind Fakten, keine Stories | Claims haben keinen temporalen Kontext | LLM kann nicht reasonen |

### 3.2 Die Zielarchitektur: Claims + Timeline

```
INGESTION:
  Konversation (100 Turns)
    │
    ├─ Turn-Level: Jeder [USER]-Turn → L2 Raw Node
    │   └─ metadata: { turn_index: 12, session_id: "abc", timestamp: "..." }
    │
    └─ Session-Level (NEU): Gemini/Claude extrahiert Timeline
        │
        ├─ Phase 1: Atomic Claims (wie bisher, aber geordnet)
        │   "Turn 12: Kanoa dislikes album reviews"
        │   "Turn 34: Kanoa enjoys writing album reviews"
        │   "Turn 67: Kanoa dislikes album reviews due to criticism"
        │
        ├─ Phase 2: Preference Arcs (NEU)
        │   "Kanoa's attitude to reviews: disinterested → enthusiastic → disillusioned"
        │
        └─ Phase 3: Causal Links (NEU)
            "Turn 67 dislike CAUSED_BY Turn 56 'criticism overwhelming'"

SPEICHERUNG:
  Jeder Claim wird ein Node mit:
    ├─ turn_index (für chronologische Sortierung)
    ├─ session_id (für Session-Gruppierung)
    ├─ temporal_relation: Option<TemporalRelation>  ← NEU
    │   enum TemporalRelation {
    │       Precedes(Uuid),   // "dieser Claim kommt NACH Claim X"
    │       Follows(Uuid),    // "dieser Claim kommt VOR Claim Y"
    │       Causes(Uuid),     // "Claim X verursacht Claim Y"
    │       Contradicts(Uuid), // "dieser Claim widerspricht Claim X (später)"
    │   }
    └─ preference_arc: Option<String>  // "disinterested → disillusioned"

RETRIEVAL:
  Query: "How did Kanoa's feelings about album reviews change?"
    │
    ├─ Vector Search → findet alle Claims mit "album reviews"
    │   (Claims sind atomar → Cosine-Similarity ist präzise)
    │
    ├─ Temporal Sort: sortiere nach turn_index
    │   Turn 12: dislikes
    │   Turn 34: enjoys
    │   Turn 67: dislikes again
    │
    ├─ Arc Resolution: finde den preference_arc Node
    │   "disinterested → enthusiastic → disillusioned"
    │
    └─ Context Assembly:
        ## Timeline: Album Reviews
        - [Turn 12] Disliked reviews initially
        - [Turn 34] Started enjoying writing them
        - [Turn 67] Criticism caused return to disliking
        ## Arc: disinterested → enthusiastic → disillusioned
```

### 3.3 Was dafür am Code ändern muss

**Minimal Invasive (erreichbar in 1-2 Tagen):**

1. **`FractalNode` um `turn_index` erweitern** (1 Feld, kein Breaking Change)
   ```rust
   pub struct FractalNode {
       // ... existing fields ...
       pub turn_index: Option<i32>,     // Position in der Session
       pub session_id: Option<String>,  // Session-Gruppierung
   }
   ```

2. **`store_external` Metadata nutzen** (kein Code-Change nötig!)
   Der Benchmark sendet bereits `chunk_index` und `session_id` in metadata. KnowWhere speichert metadata als `HashMap<String, Value>`. Aber Retrieval nutzt es nicht für Sortierung.

3. **Retrieval-Pipeline um Temporal-Sort erweitern** (~50 Zeilen in `routes.rs`)
   Nach `expand_fractal()`, vor MMR:
   ```rust
   // Wenn query temporal ist → sortiere nach turn_index
   if has_temporal_markers(&query_text) {
       results.sort_by(|a, b| {
           a.node.turn_index().cmp(&b.node.turn_index())
       });
   }
   ```

4. **Prompt-Template für temporale Queries** (~30 Zeilen im Benchmark-Connector)
   ```python
   TEMPORAL_CONTEXT = """## Timeline: {topic}
   {claims_sorted_by_turn}
   ## Arc: {preference_arc}

   Based on this timeline, answer the question."""
   ```

**Mittel (1-2 Wochen):**

5. **`TemporalRelation` Edge-Typ** im Graph-System
6. **Session-Level-Konsolidierung** — fasst alle Turns einer Session zu einer Timeline zusammen
7. **Gemini-basierte Claim-Extraktion mit Turn-Index** (statt Ollama)

**Langfristig (1 Monat+):**

8. **Preference-Arc-Detection** — erkennt "disinterested → enthusiastic → disillusioned" Muster automatisch
9. **Causal-Reasoning-Graph** — verknüpft Claims kausal: "Turn 56 führte zu Turn 67"
10. **Temporal-Query-Parser** — erkennt "how did X change", "used to", "previously" etc.

---

## 4. Die 6 Bugs/Issues die wir gefunden haben

### 4.1 🔴 KRITISCH: `store_external_event` ohne Dokument-Prefix
**Datei:** `src/connectors/mod.rs:36`
```rust
// FALSCH: embed() direkt — kein "search_document:" Prefix
embedding.embed(&event.pointer).await?
```
Batch-Variante macht es richtig mit `embed_document_batch()`. Betrifft Events von Webhooks/Connectors.

### 4.2 🔴 KRITISCH: `EmbeddingRouter::route()` ohne Prefix
**Datei:** `src/embedding/router.rs:52`
```rust
// FALSCH: raw embed() — kein Prefix, keine Query/Document-Unterscheidung
self.text_provider.embed(text).await
```
Betrifft alle multimodal gerouteten Embeddings (Text, Sensor, JSON).

### 4.3 🟡 `persist_chat_exchange`: Query als Document embedded
**Datei:** `src/api/routes.rs:1861`
```rust
// AMBIGUOUS: User-Frage bekommt "search_document:" Prefix
embed_document(&*state.embedding, question).await?
```
User-Fragen sind semantisch Queries, werden aber als Documents gespeichert.

### 4.4 🟡 `/embed` Endpoint: immer Query-Modus
**Datei:** `src/api/routes.rs:377`
```rust
// Kein Document-Modus verfügbar
let vector = embed_query(&*state.embedding, &req.text).await?;
```
Es gibt keine Möglichkeit, über den Embed-Endpoint Dokumente zu embedden.

### 4.5 🟡 Model-Prefix-Detection ist fragil
**Datei:** `src/embedding/provider.rs:323`
```rust
if self.model.contains("snowflake") || self.model.contains("arctic") || ...
```
Substring-Matching statt explizitem Config-Flag.

### 4.6 🟢 `zoom_retrieve()` vs `children_tier_ids` Disconnect
**Datei:** `src/memory/fractal_node.rs:468`
```rust
// Nutzt self.children (altes Feld), NICHT children_tier_ids (neues Feld)
let best_child = self.find_best_child(query_vector, threshold);
```
`expand_fractal()` in `in_memory.rs` überbrückt das, aber es sind zwei parallele Mechanismen.

---

## 5. Die SOLL-Architektur: KnowWhere als "Memory OS"

### 5.1 Schichtenmodell

```
┌─────────────────────────────────────────────────┐
│ LAYER 5: QUERY INTERFACE                         │
│ intent detection, temporal parsing, reflection   │
├─────────────────────────────────────────────────┤
│ LAYER 4: RETRIEVAL ENGINE                        │
│ hybrid search, fractal zoom, governance, MMR     │
├─────────────────────────────────────────────────┤
│ LAYER 3: CLAIM EXTRACTION (NEU)                  │
│ Gemini/Claude → atomic claims + temporal edges   │
├─────────────────────────────────────────────────┤
│ LAYER 2: CONSOLIDATION (ERWEITERT)               │
│ session-level, timeline-preserving, arc-detecting│
├─────────────────────────────────────────────────┤
│ LAYER 1: STORAGE                                 │
│ FractalNode + turn_index + TemporalRelation      │
├─────────────────────────────────────────────────┤
│ LAYER 0: RAW INGESTION                           │
│ store_external, store_session, connectors        │
└─────────────────────────────────────────────────┘
```

### 5.2 Datenfluss SOLL

```
INPUT: Konversation (N Turns mit [USER], [ASSISTANT], [SYSTEM])

STEP 1 — Ingest (Layer 0):
  ├─ Jeder Turn → L2 Raw Node mit metadata: {turn_index, session_id, timestamp}
  └─ Keine Chunks, keine Batch-Verarbeitung — atomare Turns

STEP 2 — Extract (Layer 3, NEU):
  ├─ Gemini Flash liest KOMPLETTE Session
  ├─ Extrahiert Claims MIT Turn-Index:
  │   {claim: "...", turn_index: 12, session_id: "abc"}
  ├─ Erkennt Preference Arcs:
  │   {topic: "album reviews", arc: "disinterested → disillusioned"}
  └─ Baut TemporalRelations:
      {from: claim_12, to: claim_67, type: Contradicts}

STEP 3 — Store (Layer 1):
  ├─ Claims → Semantic/Preference Nodes mit turn_index
  ├─ Arcs → Decision Nodes mit children_tier_ids = [claim_12, claim_34, claim_67]
  └─ Relations → Graph-Edges mit TemporalRelation-Typ

STEP 4 — Consolidate (Layer 2, ERWEITERT):
  ├─ Session-Level: Alle Turns einer Session → Timeline-Narrative
  ├─ Cross-Session: "In Session A: X. In Session B: Y. Trend: Z"
  └─ Causal: "Claim A (Session 1) CAUSED Claim B (Session 3)"

STEP 5 — Retrieve (Layer 4):
  ├─ Vector Search → atomare Claims (präzise Cosine-Matches)
  ├─ Temporal Sort → chronologische Ordnung
  ├─ Arc Resolution → finde den Preference-Arc
  └─ Context Assembly → Timeline-Format für LLM

STEP 6 — Answer (Layer 5):
  └─ LLM bekommt Timeline + Arcs → kann temporal reasonen
```

### 5.3 Minimaler Code-Change für sofortigen Impact

Der kleinste Change mit größtem Impact:

1. **`turn_index` im Benchmark-Metadata korrekt setzen** (haben wir schon: `chunk_index`)
2. **Retrieval sortiert nach `turn_index` bei temporalen Queries** (~50 Zeilen)
3. **Context-Template mit Timeline-Struktur** (~30 Zeilen)
4. **Gemini-Extraktion mit Turn-Index** (haben wir schon fast: Claims-Extraktion läuft)

Das sind ~100 Zeilen Code — machbar in einer Session. Erwarteter Impact: 50% → 65-75% auf PersonaMem.

---

## 6. Fazit

**KnowWhere ist kein "dummes Vektor-Suchsystem".** Es ist eine 5-stufige Retrieval-Maschine mit Governance, Intent-Scoring, Fractal-Zoom und MMR-Diversification. Die Architektur ist durchdacht und technisch sauber.

**Aber:** Es fehlt die zeitliche Dimension. Die Konsolidierung produziert isolierte Fakten-Atome, keine Geschichten. Und PersonaMem testet genau das: Geschichten über Präferenz-Änderungen.

**Der Weg nach vorne ist nicht "bessere Embeddings" oder "anderes Chunking".** Der Weg ist:

1. **Claims MIT Turn-Index extrahieren** (statt isolierter Fakten)
2. **Temporal-Sort im Retrieval** (statt reiner Score-Sortierung)
3. **Timeline-Context-Template** (statt flacher "## Evidence 1, ## Evidence 2")
4. **Session-Level-Konsolidierung** (statt Node-Level)

Das sind keine radikalen Änderungen — es sind Erweiterungen des existierenden Systems. KnowWhere hat alle Bausteine (Claims, Relations, Tiered Context). Sie müssen nur temporal verknüpft werden.

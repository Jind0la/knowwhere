# KnowWhere — Task-Liste aus Code-Review

> Erstellt: 2026-03-21 | Review: Technisches Code-Review & Verbesserungsvorschläge
> Status: 🟡 In Bearbeitung

---

## 🔴 Kritisch — Sofort

### [CRIT-001] Timing-Angriff im API-Key-Vergleich beheben

**Problem:** `t == expected` in `src/api/auth.rs` ist anfällig für Timing-Angriffe.

**Research:**
- `subtle::ct_eq()` für constant-time comparison
- Rate-Limit kommt VOR Auth auf Auth-Endpoints

**Fix:**
```rust
use subtle::ConstantTimeEq;

fn secure_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false; // Länge ist öffentlich, kein Leak
    }
    a.ct_eq(b).into()
}

// Usage in auth middleware:
if !secure_compare(t.as_bytes(), expected.as_bytes()) {
    return Err(StatusCode::UNAUTHORIZED);
}
```

**Datei:** `src/api/auth.rs`
**Aufwand:** ~30 Minuten
**Referenzen:**
- subtle crate: https://docs.rs/subtle/2.6/subtle/trait.ConstantTimeEq.html

---

### [CRIT-002] Rate-Limiting auf Auth-Endpoints

**Problem:** Keine Rate-Limits — Brute-Force auf API-Key trivial möglich.

**Research:**
- `axum-governor` + `lazy-limit` für deklaratives Rate-Limiting
- Reihenfolge: RealIpLayer → GovernorLayer → Auth
- Strengere Limits für Auth-Endpoints (3 req/s) vs. API (5 req/s)

**Fix:**
```rust
// Cargo.toml
// axum-governor = "0.1"
// lazy-limit = "1"

lazy_limit::init_rate_limiter!(
    default: RuleConfig::new(Duration::seconds(1), 5),
    routes: [
        ("/auth/login",    RuleConfig::new(Duration::seconds(1), 3)),
        ("/auth/refresh",  RuleConfig::new(Duration::seconds(1), 3)),
    ]
).await;
```

**Middleware-Reihenfolge:**
```
Request → RealIp → Governor → CORS → Auth → Handler
```

**Aufwand:** ~2 Stunden
**Referenzen:**
- axum-governor: https://docs.rs/axum-governor

---

### [CRIT-003] JSON-Persistenz → PostgreSQL (Option A)

**Problem:** `state.json` ist kein Production-Backend. Keine concurrent write safety, O(n) Save-Overhead, kein WAL.

**Research-Ergebnis:**

`postgres_store.rs` ist bereits **~80% fertig** — keine Neueimplementierung nötig:
- ✅ Volle CRUD-API (`store_session`, `get_memory`, `vector_search`, etc.)
- ✅ Event Sourcing (immutables Event Log, Layer 0)
- ✅ Schema mit pgvector HNSW-Index, Knowledge Edges, Fractal-Zoom
- ✅ Trigger für immutables Event-Log, auto-`updated_at`, `content_preview`
- ✅ `api_keys` Tabelle für Auth

**Was noch fehlt:**
- `PostgresStore` in Storage-Interface einklinken (statt JSON-Store)
- Schema-Migration auf PostgreSQL laufen lassen
- USearch + PostgreSQL dual维护 (Vektoren in beiden)

**Entscheidung:** ✅ Direkt PostgreSQL — kein SQLite-Zwischenstopp.

| Kriterium | SQLite (WAL) | PostgreSQL |
|-----------|-------------|------------|
| Concurrent Writes | 1 Writer | ✓ MVCC |
| Vektor-Index | Extern (USearch) | pg_vector |
| JSON-Support | json_extract (TEXT) | JSONB + GIN |
| Setup-Aufwand | Minimal | Mittel |
| Bestehende Arbeit | Keine | ✅ 80% fertig |
| Für Cloud/Distributed | Nein | ✓ Ja |

**Aufwand:** ~1 Tag (Integration + Tests — nicht bei Null starten!)
**Referenzen:**
- deadpool-sqlite: https://docs.rs/deadpool-sqlite
- SQLite Performance: https://www.sqlite.org/speedcheck.html

---

## 🟡 Mittelfristig

### [MED-001] Lineares → Exponentielles Decay-Modell

**Problem:** Code selbst merkt an, dass lineares Decay falsch ist. Echte Ebbinghaus-Kurve ist exponentiell.

**Research — Formel:**
```
R(t) = e^(-t/S)

t = vergangene Zeit (Sekunden)
S = Stability (je höher, desto stabiler die Erinnerung)
Halbwertszeit: t½ = S · ln(2)
```

**Typische Werte:**

| Memory-Typ | Halbwertszeit | λ (decay_rate) |
|------------|---------------|----------------|
| Fakten/Trivia | 7 Tage | 0.099/day |
| Kontext-Info | 1–3 Tage | 0.693–0.231/day |
| Langzeit-Wissen | 30+ Tage | 0.023/day |

**PostgreSQL-Implementierung:**
```sql
-- R = exp(-λ * days_elapsed)
UPDATE memories
SET energy = GREATEST(0,
    CAST(energy * EXP(-decay_rate * EXTRACT(EPOCH FROM (NOW() - last_energy_update)) / 86400.0)
    AS INT))
WHERE id = $1;
```

**Rust-Implementierung:**
```rust
use std::time::{DateTime, Utc};

fn retention(stability_secs: f64, last_access: DateTime<Utc>) -> f64 {
    let elapsed = Utc::now()
        .signed_duration_since(last_access)
        .num_seconds() as f64;
    (-elapsed / stability_secs).exp()
}

fn with_halflife(halflife_days: f64) -> f64 {
    halflife_days * 86400.0 / 2.0_f64.ln() // S = halflife / ln(2)
}
```

**Datei:** `src/memory/dream/energy_decay.rs`
**Aufwand:** ~1 Stunde
**Referenzen:**
- Ebbinghaus Curve: https://www.edubloxtutor.com/the-mathematics-of-forgetting-ebbinghauss-curve-meets-modern-ai-predictions/

---

### [MED-002] Tiered Compaction — LLM-Summarization statt Truncation

**Problem:** `generate_overview()` und `generate_summary()` sind reine Truncation-Funktionen (erste 50 Zeichen). Macht Tiered Context System de facto wirkungslos.

**Research:**
- Bereits `GrokProvider` und `OpenAIProvider` als Embedding-Provider vorhanden
- Günstige Modelle: `gpt-4o-mini` ($0.15/1M tokens) oder `claude-haiku-3`
- Batch-Embedding bereits über `embed_batch()` möglich

**Fix-Konzept:**
```rust
// src/memory/tiered.rs

pub async fn generate_summary(&self, node: &FractalNode) -> Result<String> {
    // Statt truncate(50):
    let prompt = format!(
        "Fasse den folgenden Memory-Inhalt prägnant zusammen (~50 Wörter):\n\n{}",
        node.content
    );
    
    // Nutze vorhandenen LLM-Provider
    self.llm_provider
        .complete(&prompt)
        .await
        .map(|c| c.content)
}
```

**Aufwand:** ~1 Tag
**Enabler:** LLM-Provider ist bereits vorhanden

---

### [MED-003] BM25-Corpus nach Neustart für External-Nodes

**Problem:** `bm25_corpus` fehlt in `PersistedState`. External-Nodes (ohne `content`, nur `original_pointer`) verlieren BM25-Eintrag nach Neustart.

**Research:**
- USearch + BM25 parallel pflegen
- BM25-Corpus in PersistedState aufnehmen ODER
- On-demand Re-Indexing beim Start

**Fix:**
```rust
// src/storage/in_memory.rs

struct PersistedState {
    nodes: HashMap<Uuid, FractalNode>,
    uuid_to_key: HashMap<Uuid, u64>,
    key_to_uuid: HashMap<u64, Uuid>,
    next_key: u64,
    // NEU:
    bm25_corpus: Vec<(Uuid, String)>,  // oder: Vec<(Uuid, String)> für External-Nodes
}
```

**Aufwand:** ~2 Stunden
**Priorität:** Mittel (beeinträchtigt lexikalische Suchequalität nach Neustart)

---

### [MED-004] O(n²) Conflict Detection → Vektor-Ähnlichkeit

**Problem:** `detect_confidence_conflicts()` lädt alle aktiven Erinnerungen und macht exakten String-Match. Skaliert nicht bei 100k+ Erinnerungen.

**Research:**
- USearch-HNSW bereits vorhanden für Vektor-Suche
- Semantische Konflikt-Erkennung via Cosine-Similarity > 0.95
- Statt O(n²) String-Match → O(log n) HNSW-Lookup

**Fix-Konzept:**
```rust
// detect_conflicts_via_vectors()
let threshold = 0.95_f32;
let conflicts: Vec<(Uuid, Uuid)> = Vec::new();

// Für jeden Node: semantische Nachbarn > threshold finden
for node in active_nodes {
    let neighbors = usearch_index
        .search(&node.vector, 10)  // Top-10 semantische Nachbarn
        .into_iter()
        .filter(|(id, similarity)| similarity > &threshold && *id != node.id);
    
    for neighbor in neighbors {
        // Konflikt gefunden
        conflicts.push((node.id, neighbor.id));
    }
}
```

**Aufwand:** ~4 Stunden
**Vorteil:** Genauere Konflikt-Erkennung (semantisch statt syntaktisch)

---

### [MED-005] Doppelte Governance-Logik konsolidieren

**Problem:** `GovernanceValidator::validate()` und `GovernanceCandidate::apply_governance()` implementieren ähnliche Logik an zwei Stellen.

**Fix:**
```rust
// GovernanceCandidate delegiert an GovernanceValidator
impl GovernanceCandidate {
    pub fn governance_score(&self, policy: &GovernancePolicy) -> f32 {
        // Delegiere an vollständige Implementierung
        GovernanceValidator::new(policy)
            .validate(self.clone())
            .score_multiplier()
    }
}
```

**Aufwand:** ~1 Stunde
**Dateien:** `governance.rs`

---

## 🟢 Niedrig — Später

### [LOW-001] FractalNode.children: Arena-Allocation

**Problem:** `children: Vec<FractalNode>` speichert inline. Tiefes Klonen kopiert alle Kinder + Enkel. Bei `zoom_retrieve()` werden ganze Teilbäume kopiert.

**Research:**
- Arena-Allocation: separater `NodeArena: Vec<FractalNode>` mit `NodeId: u64` Referenzen
- Alternativ: Children als `Vec<Uuid>` speichern und on-demand laden

**Aufwand:** ~3 Tage (größere Architektur-Änderung)
**Empfehlung:** Niedrig priorisieren, bis Performanz-Problem messbar

---

### [LOW-002] Embedding-Batching-Support

**Problem:** `EmbeddingProvider::embed()` unterstützt nur Einzel-Embeddings. Bulk-Import führt zu N sequenziellen HTTP-Requests.

**Research:**
- Alle modernen APIs (xAI, OpenAI) unterstützen Batch-Embedding
- Faktor 5-20x Latenz-Reduktion möglich

**Fix:**
```rust
// provider.rs
trait EmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    
    // Default: sequentiell, Provider können überschreiben
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        futures::future::join_all(texts.iter().map(|t| self.embed(t))).await
            .into_iter().collect()
    }
}
```

**Aufwand:** ~4 Stunden
**Dateien:** `provider.rs`, `bulk_import.rs`

---

### [LOW-003] CI erweitern: Clippy, Audit, Fmt

**Problem:** CI macht nur `check`, `test`, Docker-Build. Fehlt Linting, Security-Audit, Format-Check.

**Fix:**
```yaml
# .github/workflows/ci.yml
- name: cargo fmt --check
  run: cargo fmt --check

- name: cargo clippy
  run: cargo clippy -- -D warnings

- name: cargo audit
  run: cargo audit
```

**Aufwand:** ~1 Stunde
**Nutzen:** Automatische Erkennung von Security-Problemen und Code-Stil

---

### [LOW-004] RRF statt Score-Addition für Hybrid-Retrieval

**Problem:** Review schlug RRF als Verbesserung vor.

**Research-Ergebnis:** ✅ **Bereits implementiert!**

- `rrf_fuse()` existiert in `in_memory.rs`
- Nutzt `k=60` (Industry Standard)
- RRF ist korrekt — BM25-Scores werden verworfen (nur Ränge zählen)

**Fazit:** **Kein Handlungsbedarf.** Hybrid-Retrieval ist bereits state-of-the-art.

---

## 📋 Priority Matrix

| | Aufwand | Impact |
|---|---------|--------|
| **Kritisch** | | |
| CRIT-001: Timing-Angriff | Niedrig (30min) | Hoch |
| CRIT-002: Rate-Limiting | Mittel (2h) | Hoch |
| CRIT-003: JSON → PostgreSQL | Mittel (~1 Tag, 80% fertig) | Hoch |
| **Mittelfristig** | | |
| MED-001: Exp. Decay | Niedrig (1h) | Mittel |
| MED-002: LLM Compaction | Mittel (1 Tag) | Hoch |
| MED-003: BM25-Corpus | Niedrig (2h) | Mittel |
| MED-004: Vektor Conflict | Mittel (4h) | Mittel |
| MED-005: Gov. Dedupe | Niedrig (1h) | Niedrig |
| **Niedrig** | | |
| LOW-001: Arena Alloc. | Hoch (3 Tage) | Mittel |
| LOW-002: Batch Embed. | Mittel (4h) | Mittel |
| LOW-003: CI erweitern | Niedrig (1h) | Mittel |
| LOW-004: RRF | — | **bereits erledigt** ✅ |

---

## 📅 Nächste Schritte

1. **CRIT-001 + CRIT-002** sofort machen (30min + 2h = überschaubar)
2. **MED-001** als Quick-Win (1h)
3. **MED-002** Mittelfristig planen (braucht LLM-Integration)
4. **CRIT-003** PostgreSQL fertigbauen (~1 Tag, 80% vorhanden)

---

## 🔗 Research-Quellen

- Auth Security: subtle crate, axum-governor docs
- SQLite Performance: https://www.sqlite.org/speedcheck.html, Mozilla Application Services
- Decay Model: Ebbinghaus research, spaced repetition literature
- RRF: OpenSearch 2.19 RRF implementation, TopK hybrid retrieval research

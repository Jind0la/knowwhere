# KnowWhere Architecture

> Stand: April 2026 — Repository `main`, Version `0.3.0`

## 1. High-level overview

KnowWhere ist ein eigenständiger Memory-Service als Rust-Binary mit HTTP-API. Die zentrale Innovation ist die **Fractal Memory Architecture**: Informationen werden in einer 3-stufigen Hierarchie (L0 atomic → L1 overview → L2 summary) gespeichert und können über Fractal Zoom auf jeder Auflösungsebene durchsucht werden.

Die Architektur besteht aus fünf Hauptschichten:

1. **Client-Schicht** — Agenten, SDKs, OpenClaw-Plugin, React-Dashboard
2. **API- und Auth-Schicht** — Axum-Router, Bearer-Token-Middleware, Capability-Endpoint
3. **Memory- und Retrieval-Schicht** — StorageBackend, EmbeddingProvider, Hybrid Retrieval mit Fractal Zoom
4. **Compaction-Schicht** — LocalSummarizer (Ollama) + VLM-Fallback-Chain für L2→L1→L0
5. **Operations-Schicht** — Dream Scheduler, Energy Decay, Deduplication, Self-Healing, Governance

## 2. Laufzeittopologie

```text
Agent / SDK / Dashboard
        │
        ▼
Axum Router
  ├─ public routes: /health, /swagger-ui, /register, /login, /refresh
  └─ protected routes: /auth/me, /store_session, /retrieve_fractal, ...
        │
        ▼
Auth middleware → AuthContext(token_kind, allowed_retrieval_profiles)
        │
        ▼
StorageBackend + EmbeddingProvider
  ├─ MemoryStore (default, JSON-backed)
  └─ PostgresStore (postgres-storage, pgvector)
        │
        ▼
Fractal Memory Engine
  ├─ FractalNode (5 types × 4 trust tiers × 3 context tiers)
  ├─ USearch (vector) + BM25 (keyword) + RRF (fusion)
  └─ Fractal Zoom (hierarchical retrieval with pruning)
        │
        ▼
Operational workers
  ├─ ConsolidationScheduler (L2→L1→L0 via LocalSummarizer)
  ├─ AuditScheduler (energy decay, dedup, conflicts)
  ├─ VLM Worker (GPT-5-nano → GPT-4o-mini → Grok-4-fast)
  └─ Frigate Connector (NVR polling)
```

## 3. Repository structure

```text
knowwhere/
├── Cargo.toml
├── src/
│   ├── main.rs                    # Server entry, router setup, all workers
│   ├── runtime.rs                 # Store init, embedding provider selection
│   ├── api/
│   │   ├── auth.rs                # Bearer token auth, user registration
│   │   ├── docs.rs                # OpenAPI / utoipa schema
│   │   ├── routes.rs              # All REST handlers (~3268 lines)
│   │   ├── webhooks.rs            # DedupCache, webhook infrastructure
│   │   └── subconscious_qa.rs     # Question type detection for /chat/subconscious
│   ├── embedding/
│   │   ├── mod.rs                 # EmbeddingProvider trait + ProviderKind enum
│   │   └── provider.rs            # LocalOllama, OpenAI, Grok, FixedEmbedding
│   ├── memory/
│   │   ├── mod.rs                 # MemoryStore, GovernanceCandidate
│   │   ├── fractal_node.rs        # FractalNode struct, zoom_retrieve, trust_tier
│   │   ├── types.rs               # MemoryType, MemorySource, ContextTier, etc.
│   │   ├── governance.rs          # GovernancePolicy, sensitivity checks
│   │   ├── namespaces.rs          # Namespace grouping
│   │   ├── skills.rs              # Agent skill tracking
│   │   ├── self_healing.rs        # Orphan detection, link repair
│   │   ├── events.rs              # InMemoryEventStore
│   │   └── dream/                 # Dream mode (scheduled micro-dreams)
│   ├── storage/
│   │   ├── backend.rs             # StorageBackend trait, RetrievalProfile, ScoreDebug
│   │   ├── in_memory.rs           # MemoryStore implementation
│   │   └── postgres_store.rs      # PostgresStore implementation
│   ├── scheduler/
│   │   ├── mod.rs                 # SchedulerConfig
│   │   ├── consolidation.rs       # ConsolidationScheduler (periodic L2→L1→L0)
│   │   └── audit.rs               # AuditScheduler (energy decay, dedup)
│   ├── connectors/
│   │   ├── mod.rs                 # store_external_event helper
│   │   ├── frigate.rs             # FrigateConnector (NVR polling)
│   │   └── drive.rs               # Google Drive (placeholder)
│   ├── vlm/
│   │   └── mod.rs                 # VlmWorker, 4-stage fallback chain
│   ├── summarizer/
│   │   └── mod.rs                 # LocalSummarizer (Ollama HTTP API)
│   └── multimodal/
│       └── mod.rs                 # MultimodalData (Image/Audio/Sensor)
├── dashboard/                     # React/Vite operator UI
├── frontend/                      # Minimal static fallback
├── sdk/python/                    # Python SDK
├── migrations/                    # SQL migrations (001–013)
├── docs/
├── scripts/                       # Pre-commit hook, benchmark scripts
└── .github/workflows/ci.yml      # CI pipeline
```

## 4. Fractal Memory Architecture

### 4.1 FractalNode — das zentrale Datenmodell

```rust
pub struct FractalNode {
    // Identity
    pub id: Uuid,
    pub memory_type: MemoryType,         // Episodic | Semantic | Preference | Procedural | Meta
    pub source: MemorySource,            // Conversation | Document | Import | Manual | Consolidation
    pub status: MemoryStatus,            // Active | Draft | Archived | Deleted | Superseded | Stale

    // Content (Pointer-First)
    pub content: Option<String>,         // Session: Volltext. External: None
    pub original_pointer: Option<String>,// External: URI/Pfad. Session: None
    pub embedding: Vec<f32>,            // 1024-dim (snowflake-arctic-embed2)

    // Governance
    pub confidence: f64,                 // 0.0–1.0
    pub sensitivity: Sensitivity,        // Normal | Low | High | Restricted
    pub importance: i32,                 // 1–10
    pub conflict_state: ConflictState,   // None | Pending | Resolved
    pub superseded_by: Option<Uuid>,
    pub provenance: Value,

    // Fractal Hierarchy (L0/L1/L2)
    pub context_tier: ContextTier,       // Raw(L0) | Overview(L1) | Summary(L2)
    pub parent_tier_id: Option<Uuid>,
    pub children_tier_ids: Vec<Uuid>,
    pub summary_content: Option<String>, // L0: ein-Satz-Zusammenfassung
    pub overview_content: Option<String>,// L1: Paragraph-Übersicht

    // Relations
    pub children: Vec<FractalNode>,
    pub relations: Vec<Relation>,
    pub metadata: HashMap<String, Value>,

    // Stats
    pub access_count: i32,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,

    // Optional
    pub multimodal: Option<MultimodalData>,
    pub weight: f64,
}
```

### 4.2 Fractal Zoom

```rust
pub fn zoom_retrieve(&self, query_vector: &[f32], max_depth: usize, pruning_threshold: f32)
    -> Vec<(f32, &FractalNode)>
```

- Berechnet cosine similarity auf aktueller Ebene
- Wenn `sim >= pruning_threshold` (default 0.7): steigt rekursiv in Kinder ab
- Wenn `sim < pruning_threshold`: Ast wird abgeschnitten (PRUNED)
- Ergebnisse enthalten den gesamten Pfad von der Übersicht zum Detail

### 4.3 Trust Tier Auto-Detection

```rust
pub fn trust_tier(&self) -> &'static str {
    // 1. Internal nodes (meta, assistant, system) → derived
    // 2. Explicit metadata trust_tier → use it
    // 3. Imported artifacts (MEMORY.md, SOUL.md) → primary
    // 4. Documents, Manual entries → reference
    // 5. User messages, conversations → primary
    // 6. Fallback → reference
}
```

### 4.4 5-Type System

Jeder Memory-Typ hat:
- **default_confidence** — Episodic: 0.8, Semantic: 0.85, Preference: 0.75, Procedural: 0.9, Meta: 0.5
- **default_importance** — Episodic: 5, Semantic: 6, Preference: 7, Procedural: 8, Meta: 4
- **suggested_refresh_days** — Episodic: 7, Semantic: 90, Preference: 30, Procedural: 180, Meta: 14
- **consolidation_logic** — typspezifische Strategie
- **can_evolve** / **can_contradict** — Edge-Typ-Berechtigungen

## 5. API-Aufbau

### 5.1 Öffentliche Routen

- `GET /health` — Liveness + node count
- `GET /swagger-ui/*` — OpenAPI / Swagger UI
- `POST /register` — User-Registrierung (postgres-storage)
- `POST /login` — Session-Token (postgres-storage)
- `POST /refresh` — Token-Rotation (postgres-storage)

### 5.2 Geschützte Kernrouten

- `GET /auth/me` — Token-Capabilities
- `POST /embed` — Text embedden
- `POST /store_session` — Session speichern (auto-chunking)
- `POST /store_external` — Externe Referenz speichern
- `GET /retrieve/{id}` — Einzelnen Knoten abrufen
- `POST /retrieve_fractal` — Hybrid Retrieval mit Fractal Zoom
- `POST /chat/subconscious` — Retrieval-gestützte Antwort
- `GET /nodes/recent` — Letzte Knoten
- `POST /nodes/reembed_all` — Alle Knoten neu embedden
- `POST /maintenance/repair_embeddings` — Embedding-Reparatur
- `GET /dream/status` — Compaction-Status
- `GET /vlm/status` — VLM-Worker-Status
- `POST /vlm/summarize` — VLM-Summarization anstoßen
- `GET /events` — Event-Stream
- `GET` / `POST /governance/policy` — Governance-Policy
- `POST /webhooks/frigate` — Frigate Webhook

### 5.3 PostgreSQL-Routen (postgres-storage)

- Retrieval-Analytik: `/retrieval/runs`, `/retrieval/runs/{id}`, `/retrieval/runs/{id}/trajectory`
- Lifecycle: `/memories/{id}`, `/memories/{id}/compact`, `/memories/{id}/energy/boost`
- Energy: `/energy/low`, `/energy/decay`, `/energy/compress`
- Deduplication: `/deduplication/candidates`, `/deduplication/run`, `/deduplication/runs`
- Conflicts: `/conflicts`, `/conflicts/{id}/resolve`
- Self-Healing: `/memories/{id}/reindex`, `/memories/{id}/health`, `/self-healing/stats`
- Namespaces: `/namespaces`

## 6. Auth- und Capability-Modell

```rust
pub struct AuthContext {
    pub token_kind: AuthTokenKind,           // admin | user
    pub user_id: Option<Uuid>,
    pub allowed_retrieval_profiles: Vec<RetrievalProfile>,
}
```

- **Admin-Token** (`KNOWWHERE_API_KEY`): full-fidelity, agent-debug, user-facing
- **User-Token** (PostgreSQL-Auth): user-facing only
- `GET /auth/me` ist die Single Source of Truth für Client-Capabilities

## 7. Storage-Backends

### MemoryStore (Default)
- In-Memory mit JSON-Persistenz
- USearch-Index für Vector Search
- BM25-Index für Keyword Search
- Gut für Entwicklung und Single-Node

### PostgresStore (postgres-storage)
- PostgreSQL + pgvector (1024-dim)
- Alle Lifecycle-Features (Energy, Dedup, Conflicts, Self-Healing)
- Retrieval-Trajektorien und Analytik
- User-Auth mit API-Key-Management

## 8. Embedding Provider

```rust
pub enum ProviderKind {
    LocalOllama,  // snowflake-arctic-embed2 (1024-dim), multilingual
    OpenAI,       // text-embedding-3-small (1536-dim)
    Grok,         // grok-embed (dimension variable)
}
```

Auswahlreihenfolge:
1. `KNOWWHERE_EMBEDDING_PROVIDER` wenn explizit gesetzt
2. Grok wenn `GROK_API_KEY` + `grok-provider` Feature
3. OpenAI wenn `OPENAI_API_KEY` + `openai-provider` Feature
4. Local Ollama (Default)

## 9. L2→L1→L0 Compaction

### LocalSummarizer (Primär)
- Ollama HTTP API mit llama3.2 (3B, Q4_K_M)
- Deterministisch: temperature=0, seed=42
- Feature-Flag: `summarizer` (default enabled)
- UpdateOperations: SetOverviewContent, SetSummaryContent

### VLM Fallback Chain
1. GPT-5-nano → 2. GPT-4o-mini → 3. Grok-4-fast → 4. Truncation (disabled)

### ConsolidationScheduler
- Periodisch (konfigurierbar via DREAM_INTERVAL)
- Findet unconsolidierte L0-Knoten
- Gruppiert nach Parent
- Enqueued Summarization-Jobs

## 10. Energy Decay (Ebbinghaus)

- Memories starten mit energy=50.0
- Decay folgt Ebbinghaus-Vergessenskurve
- AuditScheduler wendet periodisch Decay an
- Low-Energy-Memories können komprimiert werden
- Zugriffe boosten die Energie (spacing effect)

## 11. Self-Healing

```rust
pub struct SelfHealingConfig {
    pub check_orphaned_nodes: bool,
    pub check_broken_links: bool,
    pub check_embedding_drift: bool,
    pub repair_interval: Duration,
}
```

- Orphaned Nodes: Knoten ohne gültigen Parent werden re-parented
- Broken Links: Pointer zu gelöschten Knoten werden bereinigt
- Embedding Drift: Veraltete Embeddings werden erkannt und repariert

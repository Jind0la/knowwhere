# KnowWhere Architecture

> Stand: 25. Maerz 2026 — v0.3.0

## High-Level Overview

KnowWhere is a standalone memory service for AI agents. It runs as a single Rust binary, exposes a REST API, and persists state to disk as JSON. Agents connect via HTTP; there is no direct library coupling.

```
┌──────────────────────────────────────────────────────────┐
│  Agent (OpenClaw, LangChain, custom)                     │
│                                                          │
│  message_received → POST /store_session (user msg)       │
│  before_prompt    → POST /embed + POST /retrieve_fractal │
│  llm_output       → POST /store_session (ai response)    │
└────────────────────────┬─────────────────────────────────┘
                         │ HTTP / REST
┌────────────────────────▼─────────────────────────────────┐
│  KnowWhere Server (Axum 0.8 + Tokio)                    │
│                                                          │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │  API Routes  │  │  Auth Layer  │  │  Swagger UI    │  │
│  └──────┬──────┘  └──────────────┘  └────────────────┘  │
│         │                                                │
│  ┌──────▼──────────────────────────────────────────────┐ │
│  │  MemoryStore                                        │ │
│  │  ┌────────────┐ ┌──────────┐ ┌───────────────────┐ │ │
│  │  │ USearch    │ │ BM25     │ │ HashMap<Uuid,Node>│ │ │
│  │  │ (vectors)  │ │ (tokens) │ │ (node storage)    │ │ │
│  │  └─────┬──────┘ └────┬─────┘ └─────────┬─────────┘ │ │
│  │        └──────┬───────┘               │           │ │
│  │               ▼                       │           │ │
│  │        RRF Fusion ────────────────────┘           │ │
│  └───────────────────────────────────────────────────┘ │
│                                                          │
│  ┌───────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Embedding     │  │ Dream Mode   │  │ Persistence  │  │
│  │ Provider      │  │ (clustering) │  │ (state.json) │  │
│  └───────────────┘  └──────────────┘  └──────────────┘  │
│                                                          │
│  ┌───────────────────────────────────────────────────┐   │
│  │ Connectors (optional)                             │   │
│  │  Frigate NVR → store_external (pointer only)      │   │
│  └───────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
```

## Ordnerstruktur

```
knowwhere/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, server setup, graceful shutdown
│   ├── api/
│   │   ├── mod.rs
│   │   ├── routes.rs         # All REST endpoints, ScoredNode, clean_for_embedding
│   │   ├── auth.rs           # Bearer token middleware
│   │   └── docs.rs           # OpenAPI schema (utoipa)
│   ├── memory/
│   │   ├── mod.rs
│   │   ├── fractal_node.rs   # FractalNode, NodeType, Relation, zoom_retrieve
│   │   ├── tiered.rs        # TieredCompactionWorker (VLM-based, async)
│   │   └── dream.rs         # DreamMode (micro-dream clustering)
│   ├── embedding/
│   │   ├── mod.rs            # ProviderKind, create_provider
│   │   └── provider.rs       # Grok, OpenAI, LocalOllama + task prefixes
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── backend.rs        # StorageBackend trait (backend-agnostic interface)
│   │   ├── in_memory.rs     # MemoryStore (impl StorageBackend, USearch, BM25, RRF)
│   │   └── postgres_store.rs # PostgresStore (impl StorageBackend, SQL, RRF, ts_rank)
│   ├── connectors/
│   │   ├── mod.rs
│   │   └── frigate.rs        # Frigate NVR poller (pointer-first)
│   ├── vlm/
│   │   └── mod.rs           # VlmWorker, VlmWorkerHandle, VlmClient, SummaryContext
│   └── multimodal.rs         # MultimodalData (image/audio/sensor)
├── frontend/                 # Dashboard (vanilla JS + Tailwind)
├── data/                     # Persisted state (state.json)
├── docs/
│   ├── PRD.md
│   ├── ARCHITECTURE.md       # This file
│   ├── IMPORT_GUIDE.md       # Host-system memory import playbook
│   ├── DREAM-MODE-SCHEDULER.md
│   ├── CRIT-003-postgresql-architecture.md
│   ├── FEEDBACK_INTEGRATION.md
│   └── OPENVIKING_UPGRADES_PLAN.md
├── sdk/python/
│   └── knowwhere/
│       ├── client.py
│       └── langchain.py
└── .cursor/rules/knowwhere.mdc
```

## Datenstruktur

### FractalNode

```rust
pub enum NodeType { Session, External }

pub struct FractalNode {
    pub id: Uuid,
    pub node_type: NodeType,           // Session = full content, External = pointer only
    pub vector: Vec<f32>,              // Embedding (768-dim for nomic-embed-text-v2-moe)
    pub content: Option<String>,       // Full text (Session nodes only)
    pub original_pointer: Option<String>, // URI/path (External nodes only)
    pub metadata: HashMap<String, Value>,
    pub weight: f64,
    pub multimodal: Option<MultimodalData>,
    pub children: Vec<FractalNode>,    // Fractal children for zoom-retrieval
    pub relations: Vec<Relation>,      // Named edges to other nodes
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}
```

### ScoredNode (API response)

Retrieval endpoints return `ScoredNode` instead of raw `FractalNode`. This struct **excludes the vector** to save bandwidth and includes a relevance score:

```rust
pub struct ScoredNode {
    pub score: f32,
    pub id: Uuid,
    pub node_type: NodeType,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    pub metadata: HashMap<String, Value>,
    pub created_at: DateTime<Utc>,
}
```

## Hybrid Retrieval Pipeline

1. **Vector search** (USearch): Cosine similarity over HNSW index → top `2*k` candidates
2. **BM25 search**: Keyword scoring over cached German tokenizer → top `2*k` candidates
3. **Fractal zoom**: Each vector candidate expands via `zoom_retrieve(max_depth)`
4. **RRF fusion**: `score(d) = Σ 1/(k + rank_i)` with `k=60` across both lists
5. **Return** top `k` as `ScoredNode[]` with RRF scores

The BM25 scorer is cached and only rebuilt when the corpus changes (`bm25_dirty` flag), avoiding the O(n) rebuild cost on every query.

## Embedding Pipeline

### Providers

| Provider     | Model                       | Dimensions | Task Prefixes |
|--------------|-----------------------------|------------|---------------|
| LocalOllama  | `nomic-embed-text-v2-moe`   | 768        | Yes           |
| Grok (xAI)   | Grok embedding API          | varies     | No            |
| OpenAI       | text-embedding-3-small      | varies     | No            |

### Task Prefixes (Nomic models)

Documents are prefixed with `search_document:` before embedding. Queries are prefixed with `search_query:`. This asymmetric embedding improves retrieval quality for the nomic model family.

### Content Cleaning

Before embedding, content passes through `clean_for_embedding()` which:
- Strips markdown formatting (`**`, `##`, `|`, `` ``` ``)
- Removes emoji characters
- Collapses whitespace
- Truncates to 1024 characters

## Persistence

- **Format:** Single `state.json` file (nodes + USearch key mappings)
- **Auto-save:** Debounced every 5 seconds on write operations
- **Graceful shutdown:** SIGINT/SIGTERM triggers final save before exit
- **Recovery:** On startup, state is loaded and USearch index is rebuilt from stored vectors

## Agent Integration Pattern

### OpenClaw Plugin (`knowwhere-memory`)

The plugin registers three hooks:

| Hook                | Direction | What It Does                                          |
|---------------------|-----------|-------------------------------------------------------|
| `message_received`  | Inbound   | Stores user message as Session node (`user:<name>`)   |
| `llm_output`        | Outbound  | Stores AI response as Session node (`ai:<model>`)     |
| `before_prompt_build` | Query   | Embeds prompt → hybrid retrieval → injects context    |

Additionally, an internal hook (`handler.js`) monitors server health and notifies users when KnowWhere goes offline/online.

### Generic Integration Pattern

Any agent platform can integrate via these steps:

1. **Health check** — `GET /health` before first operation
2. **Import existing memories** — Read the host system's existing memory files and session history, store them as Session nodes with `metadata.source: "import:<system>"` and `metadata.imported_from: "<original_path>"`
3. **Store messages** — `POST /store_session` with `{ content, metadata: { source } }`
4. **Retrieve context** — `POST /embed` + `POST /retrieve_fractal` with vector + query text
5. **Circuit breaker** — After N failures, pause and retry with exponential backoff
6. **Notify user** — Report when memory goes offline/online

### Integration Rules (Non-Negotiable)

When connecting KnowWhere to an existing agent system:

- **NEVER delete, overwrite, or reset** the host system's existing memories, session history, identity files, or configuration
- **ALWAYS import** existing memories into KnowWhere first — they become additional nodes
- **ALWAYS add** to host config files (e.g. append a section to SOUL.md) — never replace content
- **ALWAYS keep** the host's original memory system running alongside KnowWhere
- KnowWhere is an **additive layer**, not a replacement

## Memory Import Pipeline

When KnowWhere connects to an existing agent system, it must import the host's existing memories before activating the live memory loop. The import follows a structured pipeline:

```
┌─────────────────────────────────────────────────────┐
│  Host System (OpenClaw, LangChain, etc.)            │
│                                                     │
│  ┌──────────┐ ┌──────────┐ ┌────────────────────┐  │
│  │ Identity │ │ Memory   │ │ Agent Knowledge    │  │
│  │ Files    │ │ Files    │ │ (research, output) │  │
│  └────┬─────┘ └────┬─────┘ └────────┬───────────┘  │
└───────┼─────────────┼────────────────┼──────────────┘
        │             │                │
        ▼             ▼                ▼
┌─────────────────────────────────────────────────────┐
│  Import Pipeline                                    │
│                                                     │
│  1. Discover  → Scan paths for known systems        │
│  2. Classify  → Identity / Memory / Research / Noise│
│  3. Filter    → Skip cron, system msgs, duplicates  │
│  4. Import    → POST /store_session with metadata   │
│  5. Verify    → Test queries across all domains     │
└─────────────────────────────────────────────────────┘
```

### Import Metadata Schema

Every imported node carries structured metadata for traceability:

```json
{
  "source": "import:openclaw:business-agent:konkurrenzanalyse.md",
  "imported_from": "~/.openclaw/workspace-business-agent/research/konkurrenzanalyse.md",
  "import_type": "openclaw_agent_knowledge",
  "agent": "business-agent",
  "original_file": "konkurrenzanalyse.md"
}
```

### Known Host Systems

| System | Detection | Memory Location | Noise Sources |
|--------|-----------|-----------------|---------------|
| OpenClaw | `~/.openclaw/openclaw.json` | `workspace/MEMORY.md`, `memory/*.md`, sub-agent workspaces | Cron jobs, system messages, heartbeat |
| LangChain | `langchain` in deps | In-memory or SQLite/Redis | Intermediate chain outputs |
| LlamaIndex | `storage/` dir | `docstore.json`, `chat_store.json` | Index rebuild artifacts |
| CrewAI | `crewai` in deps | Task results, agent memory | Delegation logs |
| Cursor | `.cursor/rules/` | `agent-transcripts/*.jsonl` | Tool call metadata |

Full import documentation: `docs/IMPORT_GUIDE.md`

## Connectors

### Frigate NVR (optional)

When `FRIGATE_URL` is set, a background poller fetches camera events every 30s and stores them as **External nodes** (pointer-first — no images stored, only `frigate://` URIs with metadata like camera name, label, confidence).

## Security

- Bearer token auth on all routes except `/health` and `/swagger-ui`
- CORS enabled (configurable)
- No credentials in code or state files
- State file contains only embeddings + text + metadata (no secrets)

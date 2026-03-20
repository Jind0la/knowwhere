# OpenViking-Inspired Upgrades für KnowWhere

**Branch:** `feature/openviking-inspired-upgrades`
**Status:** Planning
**Datum:** 2026-03-20

---

## Ziel

KnowWhere um 5 Features erweitern, inspiriert durch OpenViking Research:

1. **Tiered Context Loading (L0/L1/L2)** — Automatische Kontext-Hierarchie
2. **Retrieval Trajectory Logging** — Observability für Retrieval
3. **Skills als expliziter Memory-Typ** — Agent-Capabilities managen
4. **Session Memory Iteration** — Agent lernt aktiv aus Sessions
5. **Directory Namespace** — Hierarchische Adressierung (viking://-ähnlich)

---

## Feature 1: Tiered Context Loading (L0/L1/L2)

### Was
- Memories haben jetzt einen `context_tier`: `summary` (L0), `overview` (L1), `raw` (L2)
- Automatische Generierung: Raw → Overview → Summary via VLM
- Retrieval lädt standardmäßig nur L0/L1; Raw bei Bedarf

### Warum
- 26-54% Token-Reduktion (ACon Paper)
- LLMs werden bei langen Contexten verwirrt ("Context Distraction")
- Bessere Retrieval-Qualität durch聚焦 auf höhere Abstraktionen

### Implementation

#### Schema-Änderung (PostgreSQL)

```sql
-- Neuer Enum für Context-Tier
CREATE TYPE context_tier AS ENUM ('summary', 'overview', 'raw');

-- Neue Tabelle für Tiered Memories
CREATE TABLE memories (
    -- ... existing columns ...
    context_tier context_tier NOT NULL DEFAULT 'raw',
    parent_tier_id UUID REFERENCES memories(id),  -- summary → overview → raw
    summary_content TEXT,                          -- L0: one-sentence
    overview_content TEXT,                        -- L1: paragraph summary
    -- ...
);

-- Index für schnelle Tier-Filterung
CREATE INDEX idx_memories_tier ON memories(context_tier);
CREATE INDEX idx_memories_tier_session ON memories(session_id, context_tier);
```

#### Rust-Änderungen

1. **Neuer Typ in `types.rs`:**
```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContextTier {
    Summary,   // L0: one-sentence
    Overview,  // L1: paragraph  
    Raw,       // L2: full content
}
```

2. **FractalNode erweitern:**
```rust
pub struct FractalNode {
    // ... existing fields ...
    pub context_tier: ContextTier,
    pub parent_tier_id: Option<Uuid>,
    pub summary_content: Option<String>,
    pub overview_content: Option<String>,
}
```

3. **Background Worker für Auto-Compaction:**
```rust
// In memory/tiered.rs
pub struct TieredCompactionWorker {
    pool: PgPool,
    embedding: Arc<dyn EmbeddingProvider>,
}

impl TieredCompactionWorker {
    /// Kompaktiert Memories wenn Session zu groß wird
    pub async fn compact_session(&self, session_id: Uuid) -> Result<()> {
        // 1. Sammle alle L2 (raw) Memories der Session
        // 2. Generiere L1 (overview) via VLM
        // 3. Generiere L0 (summary) via VLM
        // 4. Speichere mit parent_id Verknüpfung
        // 5. Markiere originale L2 als "compacted"
    }
}
```

4. **API-Änderungen:**
- Neuer Endpoint: `POST /consolidate/{session_id}` — triggert Tiered Compaction
- `GET /memories/{id}?tier=raw` — lädt Raw-Content bei Bedarf
- `RetrieveFractalRequest` bekommt `max_tier` Filter (default: overview)

### Reihenfolge
1. Schema-Migration schreiben
2. `ContextTier` Enum + FractalNode-Erweiterung
3. Tiered Retrieval in `hybrid_retrieve`
4. Background Worker für Auto-Compaction
5. API-Endpunkte

---

## Feature 2: Retrieval Trajectory Logging

### Was
- Jeder Retrieval-Vorgang wird mit Metadaten geloggt
- Trajectory zeigt: Welche Steps, welche Scores, welche Entscheidungen

### Warum
- **Observability:** Man sieht WIE Kontext gefunden wurde
- **Debugging:** Retrieval-Fehler werden nachvollziehbar
- **Optimierung:** RAGAs-Metriken (Contextual Precision/Recall)

### Implementation

#### Schema

```sql
CREATE TABLE retrieval_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query_text TEXT NOT NULL,
    query_embedding vector(768),
    run_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Stats
    total_candidates INT,
    retrieved_count INT,
    execution_time_ms INT,
    max_depth_used INT,
    
    metadata JSONB DEFAULT '{}'
);

CREATE TABLE retrieval_trajectory (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID REFERENCES retrieval_runs(id) ON DELETE CASCADE,
    
    step_index INT NOT NULL,
    step_type VARCHAR(30),  -- 'initial_search', 'fractal_zoom', 'rerank', 'governance_filter'
    
    memory_id UUID REFERENCES memories(id),
    score_before FLOAT,
    score_after FLOAT,
    rank INT,
    
    -- Reasoning/Explainability
    decision TEXT,           -- "Directory 'skills' had 3/5 relevant children"
    filter_reason TEXT,      -- "Excluded: superseded memory"
    
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index
CREATE INDEX idx_trajectory_run ON retrieval_trajectory(run_id);
CREATE INDEX idx_runs_at ON retrieval_runs(run_at DESC);
```

#### Rust

```rust
// In storage/trajectory.rs

#[derive(Debug, Clone)]
pub struct RetrievalStep {
    pub step_index: usize,
    pub step_type: String,
    pub memory_id: Option<Uuid>,
    pub score_before: Option<f32>,
    pub score_after: Option<f32>,
    pub rank: Option<usize>,
    pub decision: String,
    pub filter_reason: Option<String>,
}

pub struct RetrievalTrajectory {
    pub run_id: Uuid,
    pub query_text: String,
    pub query_embedding: Vec<f32>,
    pub steps: Vec<RetrievalStep>,
    pub total_candidates: usize,
    pub execution_time_ms: u64,
}

impl PostgresStore {
    pub async fn log_retrieval(&self, trajectory: &RetrievalTrajectory) -> Result<Uuid> {
        // Insert retrieval_runs row
        // Insert all retrieval_trajectory steps
    }
}
```

#### API

- `GET /retrieval/runs` — Liste vergangener Retrieval-Runs
- `GET /retrieval/runs/{id}` — Einzelner Run mit Trajectory
- `GET /retrieval/runs/{id}/trajectory` — Nur die Trajectory

---

## Feature 3: Skills als expliziter Memory-Typ

### Was
- Neuer Sub-Typ oder Erweiterung für "Agent kann X tun"
- Verknüpfung: Skill → Memory (wo gelernt), Skill → Tool (wie ausgeführt)

### Warum
- Agents müssen wissen was sie **können**, nicht nur was sie **wissen**
- OpenViking hat `viking://agent/skills/` — das brauchen wir auch

### Implementation

#### Schema

```sql
-- Skills sind eine View/Extension auf memories
-- oder eine separate Tabelle für mehr Struktur

CREATE TABLE agent_skills (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_name TEXT NOT NULL,
    category VARCHAR(50),         -- 'language', 'tool', 'domain', 'framework'
    proficiency INT DEFAULT 5,    -- 1-10
    
    -- Nutzung
    last_used TIMESTAMPTZ,
    success_rate FLOAT,           -- % erfolgreicher Einsätze
    
    -- Komponenten (was gehört zur Skill?)
    components TEXT[],             -- ['tokio', 'axum', 'sqlx']
    prerequisites TEXT[],         -- ['rust', 'async']
    
    -- Verknüpfung zu Memories wo gelernt
    learned_from_memory_id UUID REFERENCES memories(id),
    
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Skill ← → Memory (viele-zu-viele)
CREATE TABLE skill_memories (
    skill_id UUID REFERENCES agent_skills(id) ON DELETE CASCADE,
    memory_id UUID REFERENCES memories(id) ON DELETE CASCADE,
    relation_type VARCHAR(30),    -- 'improved_by', 'failed_using', 'referenced_in'
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (skill_id, memory_id)
);

CREATE INDEX idx_skills_category ON agent_skills(category);
CREATE INDEX idx_skills_proficiency ON agent_skills(proficiency DESC);
```

#### Rust

```rust
// In memory/skills.rs

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentSkill {
    pub id: Uuid,
    pub skill_name: String,
    pub category: String,
    pub proficiency: i32,
    pub last_used: Option<DateTime<Utc>>,
    pub success_rate: Option<f64>,
    pub components: Vec<String>,
    pub prerequisites: Vec<String>,
    pub learned_from_memory_id: Option<Uuid>,
}

pub struct SkillsStore {
    pool: PgPool,
}
```

#### API

- `POST /skills` — Neue Skill erstellen
- `GET /skills` — Alle Skills (mit Filter: category, proficiency)
- `GET /skills/{id}` — Einzelne Skill
- `PUT /skills/{id}` — Skill aktualisieren (z.B. after use)
- `DELETE /skills/{id}` — Skill löschen
- `POST /skills/{id}/use` — Mark Skill als benutzt (update last_used, success_rate)
- `GET /skills/match?task=...` — Finde Skills die zu einem Task passen

---

## Feature 4: Session Memory Iteration (Active Learning)

### Was
- Nach jeder Session: automatische Extraktion von "was hat funktioniert"
- Agent lernt aus Erfahrung, nicht nur speichern

### Warum
- OpenViking: "Automatically extracts user preferences and agent operational experience"
- Die meisten Systeme speichern nur. Active Learning = echte Intelligenz.

### Implementation

#### Konzept

```
Session-Transcript → Extraction-Prompt → 
  1. User-Präferenzen (neu oder aktualisiert)
  2. Tool/Ansatz-Erfolge ("Tool X mit Pattern Y hat gut funktioniert")
  3. Tool/Ansatz-Fails ("Tool Z ist fehlgeschlagen weil...")
  4. Neue Erkenntnisse über den User/Task
```

#### Schema (existiert teilweise)

```sql
-- consolidation_history erweitern
ALTER TABLE consolidation_history 
ADD COLUMN IF NOT EXISTS preferences_extracted INT DEFAULT 0,
ADD COLUMN IF NOT EXISTS experiences_extracted INT DEFAULT 0,
ADD COLUMN IF NOT EXISTS extraction_prompt_tokens INT DEFAULT 0;
```

#### Rust

```rust
// In memory/iteration.rs

pub struct SessionExtractor {
    pool: PgPool,
    embedding: Arc<dyn EmbeddingProvider>,
    llm_client: Arc<dyn LLMClient>,  // Für Extraction via VLM
}

impl SessionExtractor {
    /// Extrahiert Knowledge aus einer Session
    pub async fn extract_session_knowledge(
        &self, 
        session_id: Uuid,
        transcript: &str
    ) -> Result<ExtractionResult> {
        
        // 1. Build extraction prompt
        let prompt = format!(
            "Analyze this session transcript and extract:\n\
             1. User preferences discovered\n\
             2. Successful tool/approach patterns\n\
             3. Failed approaches and why\n\
             4. Key insights or decisions\n\n\
             Transcript:\n{}", 
            transcript
        );
        
        // 2. Call VLM (cheap model like GPT-4o-mini)
        let response = self.llm_client.complete(&prompt).await?;
        
        // 3. Parse response und erstelle Memories
        let result = self.parse_extraction(&response)?;
        
        // 4. Speichere als 'experiential' oder 'preference' Memories
        // mit source='consolidation' und provenance.extra

        Ok(result)
    }
}

pub struct ExtractionResult {
    pub preferences: Vec<ExtractedPreference>,
    pub experiences: Vec<ExtractedExperience>,
    pub tokens_used: i32,
}
```

#### API

- `POST /sessions/{id}/extract` — Trigger Extraction für Session
- `GET /consolidation/history` — Zeigt auch extraction_stats
- Auto-Trigger: nach `store_session` wenn Session beendet

---

## Feature 5: Directory Namespace

### Was
- Hierarchische Adressierung von Memories nach Art
- Ähnlich viking:// aber als Extension von KnowWhere

### Warum
- Strukturierte Navigation: "zeig mir alle Skills"
- Kombination mit Graph Edges = sehr mächtig

### Implementation

#### Schema

```sql
-- Namespace-Tabelle
CREATE TABLE memory_namespaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    path TEXT NOT NULL UNIQUE,  -- 'user/preferences', 'agent/skills', 'resources/docs'
    depth INT NOT NULL,
    parent_id UUID REFERENCES memory_namespaces(id),
    
    description TEXT,
    memory_type_hint VARCHAR(20),  -- 'preference', 'procedural', etc.
    
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Verknüpfung: Memory → Namespace
ALTER TABLE memories ADD COLUMN namespace_id UUID REFERENCES memory_namespaces(id);

-- Pre-seeded namespaces
INSERT INTO memory_namespaces (path, depth, description) VALUES
    ('user/preferences', 2, 'User preferences and settings'),
    ('user/profile', 2, 'User profile information'),
    ('agent/skills', 2, 'Agent capabilities and skills'),
    ('agent/experience', 2, 'Learned experiences and patterns'),
    ('agent/procedures', 2, 'Agent workflows and procedures'),
    ('resources/docs', 2, 'External document references'),
    ('resources/cameras', 2, 'Camera/IoT device references'),
    ('session/history', 2, 'Session transcripts and events'),
    ('memory/meta', 2, 'Meta-information about memory system');
```

#### Rust

```rust
// In memory/namespaces.rs

#[derive(Debug, Clone)]
pub struct MemoryNamespace {
    pub id: Uuid,
    pub path: String,
    pub depth: i32,
    pub parent_id: Option<Uuid>,
    pub description: Option<String>,
}

pub struct NamespaceStore {
    pool: PgPool,
}

impl NamespaceStore {
    /// Find namespace by path
    pub async fn find_by_path(&self, path: &str) -> Result<Option<MemoryNamespace>> {
        // ...
    }
    
    /// Get all children of a namespace
    pub async fn children(&self, parent_id: Uuid) -> Result<Vec<MemoryNamespace>> {
        // ...
    }
}
```

#### Retrieval mit Namespace

```rust
// Namespace-aware retrieval
pub async fn retrieve_with_namespace(
    &self,
    namespace_path: &str,
    query_vector: &[f32],
    top_k: usize,
) -> Result<Vec<ScoredNode>> {
    // 1. Find namespace by path
    // 2. Get all memories in namespace
    // 3. Vector search within namespace
    // 4. Return with namespace context
}
```

#### API

- `GET /namespaces` — Alle Namespaces
- `GET /namespaces/{path}` — Namespace mit Memories
- `POST /namespaces` — Neuer Namespace
- `GET /namespaces/{path}/search?q=...` — Suche innerhalb Namespace

---

## Umsetzungs-Reihenfolge (AKTUALISIERT 2026-03-20)

### Phase 1: ✅ ABGESCHLOSSEN
1. **Retrieval Trajectory Logging** ✅
2. **Tiered Context (L0/L1/L2)** ✅

### Phase 2: 🔄 EXTERNES FEEDBACK — P0 PRIORITÄT
Basierend auf externem Review (Feedback 2026-03-20):

1. **Hierarchical Pruning (Threshold 0.7)** — Performance Critical
2. **Conflict Resolution im Dream Mode** — Governance Critical

### Phase 3: Feedback P1
3. **Energy / Memory Decay** (Ebbinghaus)
4. **Deduplikations-Worker**

### Phase 4: Feedback P2
5. **Content Hashing + Self-Healing**
6. **Cluster-Zentroiden-Cache**
7. **SIMD-Optimierung**

### On Hold (war Phase 2-3):
- Directory Namespace
- Skills Management
- Session Memory Iteration

---

## Datei-Änderungen

### Neue Dateien
- `src/memory/tiered.rs` — Tiered Context Logic
- `src/memory/iteration.rs` — Session Memory Extraction
- `src/memory/skills.rs` — Skills Management
- `src/memory/namespaces.rs` — Namespace Management
- `src/storage/trajectory.rs` — Retrieval Trajectory Storage
- `migrations/003_add_tiered_context.sql` — Tiered Context Schema
- `migrations/004_add_retrieval_trajectory.sql` — Trajectory Schema
- `migrations/005_add_skills.sql` — Skills Schema
- `migrations/006_add_namespaces.sql` — Namespaces Schema

### Geänderte Dateien
- `src/memory/types.rs` — ContextTier Enum hinzufügen
- `src/memory/fractal_node.rs` — Tier-Felder hinzufügen
- `src/storage/postgres_store.rs` — Neue Queries für alle Features
- `src/api/routes.rs` — Neue Endpoints
- `src/api/mod.rs` — Neue Route-Module
- `src/main.rs` — Worker-Registrierung

---

## Test-Anforderungen

Für jedes Feature:
1. Unit Tests für Core Logic
2. Integration Tests für API-Endpoints
3. PostgreSQL Migration Tests

---

## Priorisierung

| Feature | Priority | Aufwand | Impact | Notes |
|---------|----------|---------|--------|-------|
| Retrieval Trajectory | P0 | Gering | Hoch | Debugging + Observability |
| Tiered Context | P0 | Mittel | Hoch | Token-Reduktion |
| Directory Namespace | P1 | Mittel | Mittel | Struktur |
| Skills | P1 | Mittel | Mittel | Capability Mgmt |
| Session Iteration | P2 | Hoch | Hoch | Active Learning |

**Starten mit:** Retrieval Trajectory + Tiered Context

# Turn-Level Storage Schema & Embedding Strategy

## 1. Problem Statement

### Current Architecture (Session-Level Aggregation)

`POST /store_session` accepts a monolithic `content` field containing a full session transcript. The server then:

1. Calls `chunk_into_rounds()` which splits the text on role-prefix boundaries (`user:`, `assistant:`, etc.) or falls back to `TextChunker` for long unstructured text
2. Each chunk becomes a `FractalNode` in the `memories` table
3. `session_id` and `turn_index` are stored as JSONB metadata, not as first-class columns
4. Embeddings are computed per-chunk, not per-turn

**Problems this causes:**

- No `turn_id` — turns have no stable identity. They're indistinguishable from arbitrary text chunks.
- Session reconstruction requires scanning all nodes for a given `session_id` metadata key, then sorting by `turn_index` — no SQL index on either field.
- Retrieval cannot target specific turns: "what did the user say in turn 3" requires full-scan.
- Turn adjacency is invisible to the vector index. Temporal proximity between turns is lost.
- Cross-session queries ("find ALL user turns about deployment") are impossible.
- `session_id` as metadata prevents foreign-key integrity and JOIN-based queries.

### Target Architecture (Turn-Level)

Each conversational turn is an independent, first-class record with its own embedding. Sessions are proper entities. Retrieval operates at turn granularity, with session-level aggregation reconstructed on-the-fly.

---

## 2. Concrete Data Model

### 2.1 New Table: `conversation_sessions`

```sql
CREATE TABLE conversation_sessions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    external_id     VARCHAR(255) UNIQUE,       -- Hermes/OpenClaw session ID
    title           TEXT,                       -- Auto-generated or user-provided
    participant_count INTEGER DEFAULT 2,        -- user + assistant
    turn_count      INTEGER NOT NULL DEFAULT 0, -- Denormalized counter
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at        TIMESTAMPTZ,
    metadata        JSONB DEFAULT '{}',         -- Platform, model, etc.
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes
CREATE INDEX idx_sessions_external ON conversation_sessions(external_id);
CREATE INDEX idx_sessions_started ON conversation_sessions(started_at DESC);
CREATE INDEX idx_sessions_turn_count ON conversation_sessions(turn_count DESC);
```

### 2.2 New Table: `conversation_turns`

```sql
CREATE TABLE conversation_turns (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id      UUID NOT NULL
                    REFERENCES conversation_sessions(id) ON DELETE CASCADE,
    turn_index      INTEGER NOT NULL,           -- 0-based, sequential within session
    speaker_role    VARCHAR(20) NOT NULL         -- 'user', 'assistant', 'system', 'tool'
                    CHECK (speaker_role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    content_preview VARCHAR(500) GENERATED ALWAYS AS (LEFT(content, 500)) STORED,
    embedding       vector(1024),                -- Matryoshka: truncate to 512/256/128
    token_count     INTEGER,
    metadata        JSONB DEFAULT '{}',          -- model, latency, tool_calls, etc.
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Every turn in a session must have a unique index
    CONSTRAINT unique_turn UNIQUE (session_id, turn_index)
);

-- Vector index (HNSW for fast k-NN)
CREATE INDEX idx_turns_embedding_hnsw ON conversation_turns
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);

-- Turn ordering within a session
CREATE INDEX idx_turns_session_order ON conversation_turns(session_id, turn_index);

-- Filter by speaker
CREATE INDEX idx_turns_speaker ON conversation_turns(speaker_role);

-- Temporal queries
CREATE INDEX idx_turns_created ON conversation_turns(created_at DESC);

-- Full-text search on turn content
CREATE INDEX idx_turns_fts ON conversation_turns
    USING gin(to_tsvector('english', content));
```

### 2.3 Migration Path: `memories` Extension (Backward Compatible)

Existing `memories` rows with `session_id` metadata remain unchanged. Add a nullable FK column for future-proofing:

```sql
-- Add nullable FK to conversation_turns for linking legacy memory nodes
ALTER TABLE memories ADD COLUMN turn_id UUID
    REFERENCES conversation_turns(id) ON DELETE SET NULL;

-- Index for turn-scoped memory queries
CREATE INDEX idx_memories_turn ON memories(turn_id) WHERE turn_id IS NOT NULL;
```

This preserves the FractalNode model while enabling turn-level joins when data is migrated.

### 2.4 StoreSessionRequest Evolution

Current:
```json
{
    "content": "user: hello\nassistant: hi there",
    "session_id": "abc123",
    "turn_index": 0,
    "metadata": {}
}
```

Proposed — add explicit fields:
```json
{
    "content": "hello",
    "session_id": "abc123",
    "turn_index": 0,
    "speaker_role": "user",
    "metadata": {"model": "deepseek-v4-pro"}
}
```

New field: `speaker_role` (required for turns, optional for backward compat). Backward compatible: if `speaker_role` absent and `session_id` present, infer from content prefix patterns (existing `chunk_into_rounds` logic).

---

## 3. Embedding Strategy

### 3.1 Per-Turn Embedding

Each turn is embedded independently using the configured embedding provider (Ollama `snowflake-arctic-embed2`, 1024-dim).

**API flow:**

```
POST /store_turn → embed_document(turn.content) → INSERT INTO conversation_turns
```

For batch store (multi-turn sessions), embed all turns in one batch call:

```
POST /store_turns → embed_document_batch([turn1, turn2, ...]) → INSERT INTO conversation_turns (multiple)
```

### 3.2 Session-Level Composite Embedding

For coarse "which session is relevant?" queries, compute a session embedding as the mean of its turn embeddings. This is stored in the `conversation_sessions` table:

```sql
ALTER TABLE conversation_sessions ADD COLUMN embedding vector(1024);
```

Computation: `mean_vector([turn_embedding_i for i in session.turns])` — average of all turn embeddings in the session.

**Alternatively (more accurate):** Use the centroid of the first-k and last-k turns to capture session opening and closing semantics, weighted by recency.

### 3.3 Matryoshka Representation Learning (MRL)

Store the full 1024-dim embedding but support retrieval at reduced dimensions:

| MRL Dimension | Storage | Quality vs 1024 | Use Case |
|---|---|---|---|
| 1024 | Full | 100% | Primary retrieval |
| 512 | Half | ~99.5% | Fast candidate scanning |
| 256 | Quarter | ~97% | Coarse filtering |
| 128 | Eighth | ~92% | Session-level clustering |

Implementation: PostgreSQL does not natively support MRL truncation in index scans. Workaround: store a truncated copy in a separate column for the fast-scan tier, or do application-side truncation + re-ranking.

Recommended: store full 1024-dim, use `truncate_vector(embedding, 512)` in application code for candidate generation, then re-rank top-N with full 1024-dim.

### 3.4 Embedding Pipeline (Store Side)

```
StoreTurnRequest {
    session_id, turn_index, speaker_role, content
}

1. Validate turn (non-empty content, valid session)
2. Clean content: clean_for_embedding()
3. Embed: embed_document(cleaned_content) → Vec<f32>
4. INSERT conversation_turns (id, session_id, turn_index, speaker_role, content, embedding)
5. UPDATE conversation_sessions SET turn_count = turn_count + 1, updated_at = NOW()
6. Optionally: trigger consolidation check
```

---

## 4. Retrieval Patterns

### 4.1 Pattern A: Pure Turn-Level Search

"Find turns about deployment"

```sql
SELECT
    ct.id, ct.session_id, ct.turn_index, ct.speaker_role,
    ct.content, ct.created_at,
    (1 - (ct.embedding <=> $query_vector))::FLOAT AS similarity
FROM conversation_turns ct
WHERE ct.embedding IS NOT NULL
ORDER BY ct.embedding <=> $query_vector
LIMIT $top_k;
```

**Pseudocode:**
```
retrieve_turns(query_text, top_k):
    query_vec = embed_query(query_text)
    turns = db.query("SELECT ... ORDER BY embedding <=> $1 LIMIT $2", query_vec, top_k)
    return turns
```

### 4.2 Pattern B: Session-Scoped Turn Retrieval

"Find relevant turns WITHIN session X"

```sql
SELECT * FROM conversation_turns
WHERE session_id = $session_id
  AND embedding <=> $query_vector < $threshold
ORDER BY embedding <=> $query_vector
LIMIT $k;
```

### 4.3 Pattern C: Two-Phase Coarse-to-Fine

Phase 1: Find relevant sessions (coarse)
Phase 2: Zoom into turns within those sessions (fine)

```
retrieve_hierarchical(query_text, top_k):
    query_vec = embed_query(query_text)

    # Phase 1: Session-level (coarse)
    sessions = db.query("""
        SELECT id, (1 - (embedding <=> $1)) AS sim
        FROM conversation_sessions
        WHERE embedding IS NOT NULL
        ORDER BY embedding <=> $1
        LIMIT $2
    """, query_vec, max(5, top_k / 2))

    # Phase 2: Turn-level within top sessions (fine)
    session_ids = [s.id for s in sessions]
    turns = db.query("""
        SELECT ct.*, (1 - (ct.embedding <=> $1)) AS similarity,
               cs.external_id AS session_external_id
        FROM conversation_turns ct
        JOIN conversation_sessions cs ON ct.session_id = cs.id
        WHERE ct.session_id = ANY($2)
          AND ct.embedding IS NOT NULL
        ORDER BY ct.embedding <=> $1
        LIMIT $3
    """, query_vec, session_ids, top_k)

    return turns
```

### 4.4 Pattern D: Turn-Ordered Session Reconstruction

"Show me all turns in session X, in order"

```sql
SELECT * FROM conversation_turns
WHERE session_id = $session_id
ORDER BY turn_index;
```

### 4.5 Pattern E: Cross-Session Speaker Filtering

"Find all USER turns about deployment across ALL sessions"

```sql
SELECT ct.*, cs.external_id AS session_external_id,
       (1 - (ct.embedding <=> $query_vector)) AS similarity
FROM conversation_turns ct
JOIN conversation_sessions cs ON ct.session_id = cs.id
WHERE ct.speaker_role = 'user'
  AND ct.embedding IS NOT NULL
ORDER BY ct.embedding <=> $query_vector
LIMIT $top_k;
```

### 4.6 Pattern F: Adjacent Turn Expansion (Context Window)

"Get ±N turns around a matched turn for context"

```sql
SELECT * FROM conversation_turns
WHERE session_id = $session_id
  AND turn_index BETWEEN ($turn_index - $window) AND ($turn_index + $window)
ORDER BY turn_index;
```

### 4.7 Existing FractalNode Compatibility

Existing `memories` entries with `session_id` metadata are wrapped:

```
retrieve_compat(query_vector, top_k):
    # New turn-level results
    turns = retrieve_turns(...)

    # Legacy session-chunk results
    legacy = store.hybrid_retrieve(HybridQuery {
        query_vector, top_k,
        session_id: None  # don't filter
    })

    # Merge + RRF-fuse
    return rrf_fuse(turns, legacy)
```

---

## 5. Migration Path

### 5.1 Phase 0: Schema (Zero-Downtime)

```sql
-- Create new tables (no impact on existing functionality)
CREATE TABLE conversation_sessions (...);
CREATE TABLE conversation_turns (...);

-- Add nullable FK to memories
ALTER TABLE memories ADD COLUMN turn_id UUID REFERENCES conversation_turns(id);
```

### 5.2 Phase 1: Dual-Write

- New `/store_turn` endpoint alongside existing `/store_session`
- Both write to their respective tables
- `/store_session` continues to work as before (backward compat)
- Hermes Agent updated to call `/store_turn` instead of `/store_session`

### 5.3 Phase 2: Backfill Existing Data

For each row in `memories` where `metadata->>'session_id'` is not null:

```python
for memory in db.query("SELECT * FROM memories WHERE metadata->>'session_id' IS NOT NULL"):
    sid = memory["metadata"]["session_id"]
    turn_idx = memory["metadata"].get("turn_index", 0)

    # Ensure session exists
    session = upsert_session(sid)

    # Create turn record
    turn = insert_turn(
        session_id=session.id,
        turn_index=turn_idx,
        speaker_role=infer_role(memory["content"]),
        content=memory["content"],
        embedding=memory["embedding"]
    )

    # Link memory → turn
    db.execute("UPDATE memories SET turn_id = $1 WHERE id = $2", turn.id, memory["id"])
```

### 5.4 Phase 3: Deprecate Session-Level Chunking

- `/store_session` marked deprecated (returns warning header)
- New sessions stored exclusively as turns
- Monitoring: track ratio of turn-level vs session-level stores

### 5.5 Phase 4: Cleanup (Optional)

- Drop chunk-related columns from `memories` if all data migrated
- Remove `chunk_into_rounds` from store path

---

## 6. API Specification

### 6.1 POST /store_turn

```json
{
    "session_id": "abc123",           // required: session identifier
    "turn_index": 5,                  // required: 0-based index
    "speaker_role": "user",           // required: user|assistant|system|tool
    "content": "How do I deploy?",
    "metadata": {
        "model": "deepseek-v4-pro",
        "latency_ms": 1200
    }
}
```

Response:
```json
{
    "turn_id": "uuid-here",
    "session_id": "uuid-here",
    "turn_index": 5,
    "message": "turn stored"
}
```

### 6.2 POST /store_turns (batch)

```json
{
    "session_id": "abc123",
    "turns": [
        {"turn_index": 0, "speaker_role": "user", "content": "hello"},
        {"turn_index": 1, "speaker_role": "assistant", "content": "hi there"},
        {"turn_index": 2, "speaker_role": "user", "content": "deploy?"}
    ]
}
```

Batch-embeds all turns in one `embed_document_batch` call. Atomic insert.

### 6.3 GET /sessions/{session_id}/turns

Returns all turns for a session in order.

### 6.4 POST /retrieve/turns

```json
{
    "query_text": "deployment",
    "top_k": 10,
    "speaker_filter": "user",       // optional
    "session_id": "abc123",         // optional: scope to one session
    "context_window": 2             // optional: include ±N adjacent turns
}
```

Response: array of turn objects with similarity scores, session context, and adjacent turns if requested.

---

## 7. Performance Characteristics

| Operation | Current (Session) | Proposed (Turn) | Delta |
|---|---|---|---|
| Store 10-turn session | 1 embed batch (10 chunks) + 10 inserts | 1 embed batch (10 turns) + 10 inserts | ~same |
| Retrieve top-10 turns | N/A | 1 embed query + 1 vector scan | New capability |
| Retrieve session order | Full scan metadata JSON | Indexed ORDER BY turn_index | 100x faster |
| Cross-session user turns | Impossible | Indexed speaker_role + vector scan | New capability |
| Average turn size | ~500-2000 chars (chunked) | ~200-2000 chars (natural) | ~same embedding cost |
| Storage per turn | ~4KB vector + node | ~4KB vector + ~1KB row | ~same |

---

## 8. Edge Cases & Design Decisions

### 8.1 Missing Turns (Crash Recovery)

If the agent crashes mid-session, turns are stored independently. The `turn_index` UNIQUE constraint prevents duplicates. A gap in `turn_index` signals a lost turn. The agent client can resubmit the missing turn.

### 8.2 Session Embedding Updates

When a new turn is added, the session embedding should be updated incrementally:

```
new_session_embedding = ((old_embedding * N) + new_turn_embedding) / (N + 1)
```

No need to recompute from scratch. This is O(dim), not O(N*dim).

### 8.3 Very Long Turns

Turns exceeding 8K characters should be truncated before embedding, with the full content stored. The embedding covers the first 8K chars (semantic core), while the full text is available for context expansion.

### 8.4 Tool Call / System Turns

Tool outputs and system messages get `speaker_role = 'tool'` or `'system'`. These are stored but marked as `retrieval_visibility: 'internal'` in metadata so they don't surface in user-facing queries.

---

## 9. Implementation Order (Recommended)

1. **Schema migration** (SQL file, reversible)
2. **Backend: turn storage** (new endpoints, reuse existing `embed_document` pipeline)
3. **Backend: turn retrieval** (new query patterns, HNSW index)
4. **Dual-write period**: Agent writes to both old and new endpoints
5. **Backfill script** (Python, can run offline)
6. **Agent migration**: Switch to turn-level API
7. **Deprecation**: mark old endpoints, monitor
8. **Cleanup**: optional, low priority

---

## 10. Summary

This design replaces implicit session-level chunking with explicit turn-level storage. Each turn becomes a first-class entity with its own vector embedding, enabling precise retrieval, session reconstruction, and cross-session analysis. The migration is phased and backward-compatible: existing `/store_session` continues to work while new `/store_turn` endpoints are introduced.

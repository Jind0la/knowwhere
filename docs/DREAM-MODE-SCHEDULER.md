# Dream Mode Scheduler

> **Status:** ✅ Implemented  
> **Commits:** `a210e76` (VLM Worker), `279265c` (TieredCompactionWorker VLM integration), `7bf6f01` (truncation fallback)

## Context

KnowWhere has a VLM Worker that handles summarization asynchronously.
Dream Mode consists of two processes (Consolidation + Audit) that need to run periodically.
Both currently have no automated trigger — they require manual HTTP calls.

**Goal:** Build an internal scheduler that runs entirely within the KnowWhere binary,
using the same tokio async runtime as the VLM Worker. No external dependencies.

## Architecture

```
KnowWhere Binary (tokio runtime)
├── VlmWorkerHandle ──────────→ VLM Summarization (L2→L1→L0)
├── DreamScheduler ────────────→ Consolidation Worker
│   ├── interval: 1h
│   ├── calls: VlmWorkerHandle.enqueue() + store.compact_memory()
│   └── config: DREAM_CONSOLIDATION_INTERVAL_MS env var
├── DreamAuditScheduler ──────→ Audit Worker
│   ├── interval: 24h
│   ├── calls: energy_decay, deduplication, conflict_detection
│   └── config: DREAM_AUDIT_INTERVAL_MS env var
└── Background task handles all of the above
```

## Two Separate Schedulers

### 1. Consolidation Scheduler
**Trigger:** Every `DREAM_CONSOLIDATION_INTERVAL_MS` (default: 1 hour)
**What it does:**
1. Query memories with `context_tier = Raw` that haven't been consolidated yet
2. Group them by namespace/time
3. Enqueue VLM jobs for summarization via `VlmWorkerHandle`
4. Mark them as consolidation-in-progress

**Config env vars:**
- `DREAM_CONSOLIDATION_INTERVAL_MS` — default 3_600_000 (1h)
- `DREAM_CONSOLIDATION_BATCH_SIZE` — how many nodes per consolidation run, default 50
- `DREAM_ENABLED` — set to "false" to disable entirely

### 2. Audit Scheduler
**Trigger:** Every `DREAM_AUDIT_INTERVAL_MS` (default: 24h)
**What it does:**
1. **Energy Decay** — call `apply_energy_decay()` on all active memories
2. **Deduplication** — run `run_deduplication()` to find + merge duplicates
3. **Conflict Detection** — run `list_conflicts()` and auto-resolve if confidence is high

**Config env vars:**
- `DREAM_AUDIT_INTERVAL_MS` — default 86_400_000 (24h)
- `DREAM_DECAY_ENABLED` — default true
- `DREAM_DEDUP_ENABLED` — default true
- `DREAM_CONFLICT_AUTO_RESOLVE_THRESHOLD` — default 0.8 (auto-resolve if confidence > 80%)

## Implementation

### New Module: `src/scheduler/`

```
src/scheduler/
├── mod.rs          — SchedulerConfig, SchedulerHandle, trait SchedulerRun
├── consolidation.rs — ConsolidationScheduler
├── audit.rs        — AuditScheduler
└── timers.rs       — Shared interval/wheel timer utilities
```

### `SchedulerConfig`
```rust
pub struct SchedulerConfig {
    // Consolidation
    pub consolidation_interval_ms: u64,
    pub consolidation_batch_size: usize,
    pub enabled: bool,
    // Audit
    pub audit_interval_ms: u64,
    pub decay_enabled: bool,
    pub dedup_enabled: bool,
    pub conflict_auto_resolve_threshold: f64,
}

impl SchedulerConfig {
    pub fn from_env() -> Self { ... }
    pub fn is_enabled(&self) -> bool { ... }
}
```

### Integration Points

**`src/main.rs`:**
```rust
let scheduler_config = SchedulerConfig::from_env();
if scheduler_config.is_enabled() {
    let scheduler = ConsolidationScheduler::new(store.clone(), vlm_worker.clone(), scheduler_config.clone());
    let handle = scheduler.spawn();
    tracing::info!("Dream Mode scheduler started");
}
```

**VLM Worker Integration:**
- `VlmWorkerHandle::enqueue()` already exists — consolidation scheduler calls it directly
- No HTTP, no network — direct in-memory call within the same tokio runtime

**Storage Integration:**
- `store.compact_memory()` for tiered compaction
- `store.list_low_energy_memories()` for energy audit
- `store.run_deduplication()` for dedup
- These are direct calls, no API overhead

### Startup/Shutdown
- Scheduler starts automatically when `DREAM_ENABLED=true` (default: true)
- Graceful shutdown on SIGTERM/SIGINT (tokio handles this)
- Logs each run: "Dream consolidation complete: X nodes, Y summaries enqueued"
- Logs errors with full context

### Docker Integration
- All config via env vars — no code changes needed for Docker
- Default values are sensible — just works with `docker compose up`
- Health check endpoint `GET /dream/status` already exists and returns scheduler state

## Implementation Order

1. **`SchedulerConfig`** + basic skeleton in `src/scheduler/mod.rs`
2. **`ConsolidationScheduler`** — interval + VLM enqueue
3. **`AuditScheduler`** — interval + decay/dedup/conflict calls
4. **Integration in `main.rs`** + graceful shutdown
5. **Tests:** smoke test that scheduler starts and logs correctly

## Boundaries

- No external HTTP calls — everything is in-memory
- No new database tables — uses existing storage APIs
- No feature flag — always compiled in (can be disabled via DREAM_ENABLED=false)
- All existing postgres-storage features remain optional

## Commit Strategy

Single commit: `feat: internal Dream Mode scheduler for consolidation and audit`

The VLM worker commit `a210e76` already has the infrastructure (VlmWorkerHandle, async background tasks).
This scheduler uses the exact same pattern — `tokio::spawn` + channel-based job dispatch.

## Verification

After building:
1. `DREAM_ENABLED=true docker compose up` — scheduler starts, logs every interval
2. `curl localhost:3000/dream/status` — shows last run timestamps + next scheduled runs
3. `DREAM_ENABLED=false docker compose up` — scheduler stays silent

## Related Docs

- VLM Worker: `src/vlm/mod.rs` (Commit `a210e76`)
- Dream Mode definition: `src/memory/dream/mod.rs`
- Storage API: `src/storage/in_memory.rs`

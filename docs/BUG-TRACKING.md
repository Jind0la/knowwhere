# Bug Tracking

**Last Updated:** 2026-03-25

---

## Resolved Bugs

### BUG-003: Governance Filtering New Nodes ⚠️ RESOLVED

**Date Reported:** 2026-03-21
**Date Resolved:** 2026-03-25
**Status:** Resolved - False Alarm

**Description:**
Initial concern was that `governance_enabled=true` (default) would filter out all new nodes because they have low confidence/importance scores.

**Root Cause Analysis:**
The concern was **unfounded**. Unit tests confirm that governance works correctly:

| Node Condition | Governance Action |
|----------------|-------------------|
| confidence >= 0.5 (default) | ✅ PASS |
| confidence < 0.5 | ⚠️ Soft penalty (score *= 0.5) |
| sensitivity = Normal (default) | ✅ PASS |
| sensitivity = Restricted | 🚫 HARD BLOCK |
| status = Active (default) | ✅ PASS |
| status = Archived | 🚫 HARD BLOCK |
| superseded_by = None (default) | ✅ PASS |
| superseded_by = Some(id) | 🚫 HARD BLOCK |

**Governance Default Policy:**
```rust
Self {
    min_confidence: 0.5,
    max_age_days: None,
    blocked_sensitivities: vec![Sensitivity::Restricted],  // NOT Normal!
    supersession_enabled: true,
    conflict_check_enabled: true,
    recency_boost_enabled: true,
    recency_penalty_after_days: 90,
}
```

**New nodes created with defaults will ALWAYS pass governance.**

**Verification:**
```bash
cargo test governance::tests --lib
# All 5 tests pass
```

---

## Known Limitations (Not Bugs)

### Rate Limiting in Docker

**Issue:** `RATE_LIMIT=1` requires X-Forwarded-For headers from a reverse proxy.

**Status:** By design — rate limiting is optional and disabled by default.

**Workaround:** Use behind nginx/Cloudflare or disable with `RATE_LIMIT=1`.

---

## Clippy Warnings Note

The codebase has ~14 clippy warnings in the default build. These are **false positives** caused by `#[cfg(feature = "postgres-storage")]` feature gates:

- Clippy analyzes only the **default** build
- Many "unused" imports/variables are actually used inside `#[cfg(feature = "postgres-storage")]` blocks
- CI runs both default AND postgres-storage builds successfully

**Example:** `PathBuf`, `anyhow::Result`, `put` imports appear "unused" but are used inside feature-gated code.

These warnings can be safely ignored or suppressed with `#[allow(...)]` if desired, but they do NOT affect compilation, tests, or runtime behavior.

---

## Historical Bugs (Pre-v0.3.0)

### BUG-001: postgres-storage compile failure

**Status:** ✅ Resolved (2026-03-21)

The `postgres-storage` feature required additional dependencies that weren't installed in the CI environment.

**Fix:** Updated CI pipeline to install required dependencies.

---

### BUG-002: Rate Limiter not working in Docker

**Status:** ✅ Resolved (2026-03-21)

Rate limiter was enabled by default but failed without a reverse proxy setting X-Forwarded-For headers.

**Fix:** Made rate limiting optional via `RATE_LIMIT` env var, disabled by default for dev.

---

## References

- [Governance Policy Implementation](../src/memory/governance.rs)
- [Governance Tests (BUG-003 verification)](../src/memory/governance.rs) (lines ~437-595)
- [AVX-512 Compatibility](./ISSUE-AVX512-COMPATIBILITY.md)

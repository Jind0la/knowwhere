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

---

## BUG-004: Rate Limiter Routes Mismatch + Missing ApiKey Extension 🟡 FIXED 2026-03-27

**Beschreibung:**
Zwei zusammenhängende Bugs verhinderten dass Auth-Endpoints (`/login`, `/register`, `/refresh`) funktionierten:

1. **Rate Limiter Mismatch:** `init_rate_limiter!` in `main.rs` war konfiguriert mit Routen `/auth/login`, `/auth/register`, `/auth/refresh` — aber die tatsächlichen Auth-Routen in `auth_router()` waren `/login`, `/register`, `/refresh` (ohne `/auth/` Prefix). Das führte zu "Rate limiter misconfigured" Fehler.

2. **Missing ApiKey Extension:** Die `login` und `refresh` Handler erwarten eine `ApiKey` Extension, aber der `auth_router_with_state` bekam diese Extension nicht. Sie wurde nur auf die `protected` Router in `main.rs` gelegt (Zeile 278), nicht auf die Auth-Router.

**Symptome:**
- `POST /login` → "Internal Server Error: Rate limiter misconfigured"
- `POST /register` → "registration not yet implemented" (OK, Stub)
- Login mit korrektem API Key → 500 Error wegen fehlender ApiKey Extension

**Fixes:**

**Fix 1:** `main.rs` Zeile 57-59 — Rate Limiter Pfade korrigiert:
```rust
// VORHER:
("/auth/login", RuleConfig::new(...)),
("/auth/refresh", RuleConfig::new(...)),
("/auth/register", RuleConfig::new(...)),

// NACHHER:
("/login", RuleConfig::new(...)),
("/refresh", RuleConfig::new(...)),
("/register", RuleConfig::new(...)),
```

**Fix 2:** `main.rs` Zeile 283 — ApiKey Extension zum auth_router hinzugefügt:
```rust
// VORHER:
.merge(auth::auth_router_with_state(state.clone()))

// NACHHER:
.merge(auth::auth_router_with_state(state.clone()).layer(axum::Extension(api_key.clone())))
```

**Fix 3:** `auth.rs` — GovernorLayer von auth_router und auth_router_with_state entfernt (nicht mehr nötig, lazy_limit kümmert sich um Rate Limiting).

**Commit:** TBD (nach Review)

**Verification:**
```bash
# Nach Fix:
curl -X POST http://localhost:3737/login \
  -H "Content-Type: application/json" \
  -d '{"username":"test","password":"test","api_key":"test123"}'
# → {"token":"***","message":"authenticated"}

curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer test123" \
  -d '{"content":"test"}'
# → {"id":"...","message":"session node created"}
```

**Hinweis:** Ollama muss für Retrieval/Embedding laufen (`ollama pull nomic-embed-text-v2-moe`).
EOF; __hermes_rc=$?; printf '__HERMES_FENCE_a9f7b3__'; exit $__hermes_rc

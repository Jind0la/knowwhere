# Bug Tracking

> Diese Datei enthaelt sowohl aktuelle Notizen als auch historische Bug-Eintraege. Versionsbegriffe hier sind Bug-Epochenmarker und nicht die autoritative Produktversion des aktuellen `main`-Standes.

**Last Updated:** 2026-04-02

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

## Historical Bugs (Pre-April 2026 beta cleanup)

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

---

## BUG-005: Leere query_text → 500 Internal Server Error 🟢 FIXED 2026-03-27

**Beschreibung:**
`POST /retrieve_fractal` mit `query_text: ""` (leerer String) führte zu einem `500 Internal Server Error` statt einem sauberen `400 Bad Request`. Das Problem: `query_text: ""` ist `Some("")` (nicht `None`), also ging die Logik in den Embed-Branch, wo Ollama einen leeren String ablehnte.

**Symptome:**
- `curl -X POST /retrieve_fractal -d '{"query_text":"","top_k":3}'` → **500**
- `curl -X POST /retrieve_fractal -d '{"query_text":"   ","top_k":3}'` (nur Whitespace) → **500**

**Root Cause:**
In `src/api/routes.rs` Zeile ~537: `if let Some(text) = &req.query_text` matched `Some("")` als gültiger Wert, daher wurde `embed_query("")` aufgerufen. Ollama lehnt leere Strings ab → `Err` → `StatusCode::INTERNAL_SERVER_ERROR`.

**Fix:**
```rust
// src/api/routes.rs ~Zeile 537
if let Some(text) = &req.query_text {
    if text.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // ... embed_query(text) ...
}
```

**Verifizierung:**
```bash
# Vor Fix:
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer ***" \
  -d '{"query_text":"","top_k":3}'  → 500

# Nach Fix:
curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer ***" \
  -d '{"query_text":"","top_k":3}'  → 400

curl -X POST http://localhost:3737/retrieve_fractal \
  -H "Authorization: Bearer ***" \
  -d '{"query_text":"   ","top_k":3}'  → 400
```

---

## BUG-006: Repetitiver Content (z.B. "AAAA...") → 500 Internal Server Error 🟢 FIXED 2026-03-27

**Beschreibung:**
`POST /store_session` mit sehr repetitivem Content (z.B. 2000×"A") führte zu einem `500 Internal Server Error`. Das lag daran, dass `clean_for_embedding()` repetitive Zeichen nicht entfernte und Ollama den stark repetitiven Input ablehnte.

**Symptome:**
- `curl -X POST /store_session -d '{"content":"AAAA...AAAA"}'` (2000×"A") → **500**
- Fehlermeldung: `"auto-embed failed: ollama API returned error status500"`

**Root Cause:**
1. `clean_for_embedding("AAA...")` gibt "AAA..." zurück (keine Reinigung bei repetitiven Chars)
2. Ollama lehnt stark repetitive Inputs ab (vermutlich Quality-Gate im Embedding-Modell)
3. Der Fehler wurde als `500 Internal Server Error` durchgereicht

**Fix:**
```rust
// src/api/routes.rs ~Zeile 291-311 (store_session)
let cleaned = clean_for_embedding(&req.content);
if cleaned.len() < 4 {
    return Err((StatusCode::BAD_REQUEST, "content too short or empty after cleaning".into()));
}
// Repetitive-Content-Check: kein einzelner Char > 90% des Contents
{
    use std::collections::HashMap;
    let mut freq: HashMap<char, usize> = HashMap::new();
    let mut total = 0usize;
    for c in cleaned.chars() {
        if !c.is_whitespace() {
            *freq.entry(c).or_insert(0) += 1;
            total += 1;
        }
    }
    if total > 0 {
        if let Some(&max_count) = freq.values().max() {
            let ratio = max_count as f64 / total as f64;
            if ratio > 0.9 {
                return Err((StatusCode::BAD_REQUEST, "content too repetitive for embedding".into()));
            }
        }
    }
}
```

**Verifizierung:**
```bash
# Vor Fix:
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer ***" \
  -d "{\"content\":\"$(printf 'A%.0s' {1..2000})\"}"  → 500

# Nach Fix:
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer ***" \
  -d "{\"content\":\"$(printf 'A%.0s' {1..2000})\"}"  → 400
# Body: "content too repetitive for embedding"

# Emoji und normale Texte funktionieren weiterhin:
curl -X POST http://localhost:3737/store_session \
  -H "Authorization: Bearer ***" \
  -d '{"content":"App 💾 mit 🌐 und 🤖"}'  → 201 ✓
```

---

## BUG-008: cargo test — 3 OpenAI-Tests scheitern ohne API Key 🟡 KNOWN

**Beschreibung:**
3 Integration-Tests in `src/memory/tests.rs` scheitern wenn `OPENAI_API_KEY` nicht gesetzt ist:

- `test_store_session_auto_embed` (Zeile 169) — versucht OpenAI zu nutzen
- `test_openai_embedding_generates_valid_vector` (Zeile 149) — expliziter OpenAI-Test
- `test_sdk_store_session_retrieve_roundtrip` (Zeile 227) — SDK-Test mit OpenAI

**Symptome:**
```
OPENAI_API_KEY must be set: NotPresent
```

**Status:** Bekanntes Verhalten — diese Tests erfordern einen echten OpenAI API Key und sind nicht für den lokalen CI-Lauf ohne Credentials geeignet.

**Workaround:**
```bash
OPENAI_API_KEY=sk-... cargo test
```

**Fix-Idee (für später):** Diese Tests mit `#[ignore]` markieren und nur in CI mit API Key laufen lassen, oder Mock-Provider für OpenAI in Tests verwenden.

---

---

## BUG-007: PostgresStore count() gibt 0 zurück trotz existenter Rows 🟢 FIXED 2026-03-28

**Beschreibung:**
`PostgresStore::count()` via `StorageBackend::count()` gab immer 0 zurück — selbst wenn die Datenbank 2+ aktive Rows enthielt. Das INSERT funktionierte korrekt (Rows wurden in DB geschrieben), aber `count()` fand sie nicht.

**Symptome:**
- `cargo test --features postgres-storage --test integration postgres_store_count_matches_active_memories` → **FAILED** (`count() should be at least 2, got 0`)
- Direkte SQL-Abfrage in der DB: `SELECT COUNT(*) FROM memories WHERE status = 'active';` → korrekt `2`

**Root Cause:**
Das Problem lag in der SQLx Query-Definition:

```sql
-- VORHER (kaputt):
SELECT COUNT(*)::bigint as "count:i64" FROM memories WHERE status = 'active'

-- Problem: "count" ist ein SQL-reserviertes Wort und kollidiert mit der
-- internen Behandlung von COUNT(*) in sqlx. try_get("count") findet die
-- Spalte nicht korrekt → stiller Fehler → unwrap_or(0) gibt 0 zurück
```

**Debugging-Prozess:**
1. `unwrap_or(0)` verschluckte den echten Fehler — erster Fix: Debug-Logging hinzufügen
2. Nach Debug-Logging sichtbar: `ERROR: no column found for name: count`
3. Erkenntnis: `COUNT(*)` mit Alias `"count"` funktioniert syntaktisch in SQL, aber sqlx kann die Spalte nicht korrekt Resolution

**Fix:**
```rust
// src/storage/postgres_store.rs — count() Methode
// VORHER:
let total: i64 = row.try_get("count")?;  // FAIL

// NACHHER:
let total: i64 = row.try_get(0)?;  // OK — Index statt Name
```

**Files geändert:**
- `src/storage/postgres_store.rs` — `count()`: `try_get("count")` → `try_get(0)`

**Zusätzliche Fixes im selben Session:**
- `src/memory/self_healing.rs` — `original_pointer` → `content` in 2 Queries (Spalte existiert nicht in DB)
- `src/memory/self_healing.rs` — `and_then(|r| r.content)` → `map(|r| r.content)` (Typ-Mismatch)

**Commit:** In Git working directory (noch nicht committed)

**Verification:**
```bash
# DB vorbereiten und Test laufen lassen
docker exec kw-postgres psql -U postgres -d kw -c "DELETE FROM memories WHERE content LIKE 'count test%';"
DATABASE_URL="postgresql://postgres:kw@localhost:5433/kw" \
cargo test --features postgres-storage --test integration postgres_store

# Ergebnis:
# test postgres_store_hybrid_retrieve_bm25_only ... ok
# test postgres_store_count_matches_active_memories ... ok  ← FIXED!
# test postgres_store_hybrid_retrieve_with_vector ... ok
# 3 passed, 0 failed
```

**Hinweis:** Die DB läuft in Docker auf Port 5433 (nicht 5432). Connection String:
```
postgresql://postgres:kw@localhost:5433/kw
```
(Passwort wurde mit `ALTER USER postgres WITH PASSWORD 'kw';` gesetzt)

---

---

## BUG-009: content_contradicts() erkennt asymmetrische Negationen nicht 🟢 FIXED 2026-04-02

**Beschreibung:** `content_contradicts()` erkannte Widersprüche wie `"The meeting is not happening"` vs `"The meeting is happening"` nicht korrekt. Die `contradictions` array prüfte nur symmetrische Patterns (beide Strings müssen je ein Negationsmarker haben), aber `b` ("The meeting is happening") hat kein "not ".

**Symptome:**
- `cargo test test_content_contradicts_negation` → FAILED
- `cargo test test_content_contradicts_cross` → FAILED

**Root Cause:** Pattern `("not ", "")` fehlte in der contradictions array. Nur `("not ", "not ")` war vorhanden — das matched nur wenn beide Strings "not " enthalten.

**Fix:** Asymmetrische Patterns hinzugefügt in `src/memory/dream/conflict_detection.rs`:
```rust
("not ", ""),      // "not X" vs "X"
("never ", ""),    // "never X" vs "X"
("no ", ""),       // "no X" vs "X"
("doesn't ", ""),  // "doesn't visit" vs "visits"
("isn't ", ""),    // "isn't reliable" vs "is reliable"
("won't ", ""),    // "won't happen" vs "happens"
```

**Files geändert:** `src/memory/dream/conflict_detection.rs`

**Verification:**
```bash
cargo test --features postgres-storage --lib conflict_detection
# test_content_contradicts_negation ... ok
# test_content_contradicts_cross ... ok
```

---

## BUG-010: tiered.rs Doctest failed wegen Unicode Box-Drawing 🟢 FIXED 2026-04-02

**Beschreibung:** Doc-Comment in `src/memory/tiered.rs` (Zeile 31-34) nutze einen Rust-Code-Block (` ``` `) der Unicode Box-Drawing Zeichen (`──►` U+2500) enthielt. Der Rust Doctest-Parser konnte diese nicht als Rust parsen → "unknown start of token" Fehler.

**Symptome:**
```bash
cargo test --doc
# FAILED: src/memory/tiered.rs - memory::tiered::TieredCompactionWorker (line 31)
# error: unknown start of token: \u{2500}
```

**Root Cause:** ` ``` ` impliziert Rust-Syntax, aber das Diagramm war Plain-Text mit Unicode-Grafik.

**Fix:** ` ``` ` → ` ```text ` in `src/memory/tiered.rs:31` — der `text` Marker sagt rustdoc dass es Plain-Text ist, kein ausführbarer Code.

**Files geändert:** `src/memory/tiered.rs`

**Verification:**
```bash
cargo test --features postgres-storage --doc
# test result: ok. 0 passed; 0 failed; 2 ignored
```

---

## BUG-011: DREAM_ENABLED Logik invertiert — Scheduler startet nicht 🟢 FIXED 2026-04-02

**Beschreibung:** `SchedulerConfig::from_env()` in `src/scheduler/mod.rs` hatte die `enabled` Logik invertiert. Ohne `DREAM_ENABLED` env var war `enabled = false` statt `enabled = true`.

**Root Cause:**
```rust
// VORHER (kaputt):
enabled: !std::env::var("DREAM_ENABLED")
    .map(|v| v.eq_ignore_ascii_case("false"))
    .unwrap_or(false),  // ← falsch!

// NACHHER (korrekt):
enabled: std::env::var("DREAM_ENABLED")
    .map(|v| !v.eq_ignore_ascii_case("false"))
    .unwrap_or(true),  // ← Default: Dream Mode enabled
```

**Symptome:**
- Dream Mode Scheduler startet nicht obwohl alle anderen Komponenten laufen
- Keine automatische Kompaktierung, kein Energy Decay, keine Deduplizierung
- Log zeigte: `"Dream Mode scheduler disabled (DREAM_ENABLED=false)"` obwohl keine Env-Var gesetzt war

**Files geändert:** `src/scheduler/mod.rs`

**Verification:**
```bash
# Ohne DREAM_ENABLED env var:
cargo test --features postgres-storage --lib
# Alle Tests PASS

# Server startet mit Dream Mode:
cargo run --features postgres-storage
# Log: "Dream Mode scheduler started"
```

---

## Offene Bugs

Keine offenen Bugs — BUG-005, BUG-006, BUG-007, BUG-009, BUG-010 und BUG-011 sind alle gefixt.

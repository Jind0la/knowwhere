# KnowWhere OSS Forensics Report

**Datum: 3. Juni 2026 | Auditor: Hermes Operator (oss-forensics skill)**
**Target: v0.6.0, 457 Dependencies, 65 Rust Source-Files**

---

## Executive Summary

| Kategorie | Grade | Kritische Issues |
|---|---|---|
| Dependency Security | **B** | 1 Medium CVE (rsa — irrelevant via sqlx-mysql), 4 rustls-webpki advisories per Google-Drive-Feature |
| Secrets Management | **A−** | 1 Schwachstelle: `.env.benchmark` hat 644 Perms. `.env` und `.env.native` korrekt 600. |
| Code Safety (unsafe) | **A** | 1 `unsafe impl Send` mit begründetem Mutex-Schutz |
| Panic Surface | **C+** | 182 `unwrap()` calls — viele in Tests, einige in API-Routen |
| SQL Injection | **A+** | sqlx-Macros sind compile-time-safe. Keine dynamischen SQL-Strings gefunden. |
| License Compliance | **A** | MIT. Sauber. Keine Copyleft-Dependencies. |
| Supply Chain | **B** | Kein cargo-deny, kein SBOM. cargo-audit aktiv, 5 Warnungen (unmaintained crates). |
| Network Security | **A−** | Rate Limiting per GovernorLayer. Auth via API-Key (subtle constant-time). |
| **GESAMT** | **B+** | Produktionsreif mit Remediation. Keine kritischen Showstopper. |

---

## 1. Dependency Vulnerabilities

### Aktive CVEs

| Crate | Severity | Betrifft KnowWhere? | Action |
|---|---|---|---|
| `rsa 0.9.10` (Marvin Attack) | Medium 5.9 | ❌ **Nein.** Nur via `sqlx-mysql` — KnowWhere nutzt PostgreSQL. | Ignorieren / Suppressen |
| `rustls-webpki 0.103.9` (4 advisories) | Medium | ⚠️ Nur aktiv wenn `google-drive` Feature enabled ist. Google Drive Connector ist optional. | `cargo update -p rustls-webpki` — oder Feature deaktivieren |
| `rand 0.8.5` (unsound) | Warning | Nur mit custom logger — nicht relevant | Ignorieren |
| `fxhash`, `google-apis-common`, `number_prefix`, `paste` | Unmaintained | Transitive Deps, kein Runtime-Risiko | Dokumentieren |

**Fazit:** Kein einziges CVE betrifft den Core-Pfad (PostgreSQL + Embedding + Retrieval). Alle 5 advisories sind transitiv oder feature-gated.

### Fix-Commands

```bash
# rustls-webpki updaten (nur nötig falls google-drive Feature aktiv)
cargo update -p rustls-webpki

# rsa advisory supprimieren (via sqlx-mysql, nicht genutzt)
# In deny.toml: ignore = ["RUSTSEC-2023-0071"]
```

---

## 2. Secrets Detection

| Check | Status |
|---|---|
| `.env` in git? | ✅ Nur `.env.example` getrackt. `.env` und `.env.native` in `.gitignore` |
| `.env` Permissions | ✅ `.env` = 600, `.env.native` = 600 |
| `.env.benchmark` Permissions | ⚠️ **644** — world-readable! `chmod 600` empfohlen |
| Hardcoded Credentials | ✅ Keine gefunden. Drive-Connector nutzt Service-Account-JSON (korrekt) |
| `.gitignore` Coverage | ✅ `.env.*` mit Ausnahme `!.env.example` |

**P1 Fix:**
```bash
chmod 600 /Users/nimarfranklinmac/knowwhere/.env.benchmark
```

---

## 3. Unsafe Code

**1 Block gefunden:**
```rust
// src/storage/in_memory.rs:136
unsafe impl Send for SendableIndex {}
```

**Bewertung:** `SendableIndex` wrappt `usearch::Index`, das nicht `Send` ist (C-Bibliothek). Wird hinter `Arc<Mutex<>>` verwendet. **Akzeptabel**, aber eine explizite `// SAFETY:`-Begründung fehlt.

**Empfehlung:** Safety-Kommentar ergänzen (Low-Prio, Code-Style).

---

## 4. Panic Surface

**182 `unwrap()` Calls** in `src/`. Davon schätzungsweise ~40% in Test-Code (`_test.rs`, `#[cfg(test)]`).

**Produktionsrelevante (nicht in Tests):**

| Ort | Risiko |
|---|---|
| `memory/conversation.rs:392` — `turn.embedding.unwrap()` | Mittel — Turn ohne Embedding crasht |
| `memory/control_room.rs:660-703` — `query_scoped().await.unwrap()` | Mittel — Async-Fehler crasht Request |
| `connectors/drive.rs:113` — `page_token.as_ref().expect()` | Niedrig — direkt nach set |

**Empfehlung:** Top-10 unwrap() in API-Routen durch `?` oder graceful error handling ersetzen. Nicht kritisch, aber technische Schuld.

---

## 5. SQL Injection

**Keine Injection-Vektoren gefunden.**

Alle SQL-Queries nutzen sqlx-Macros (`sqlx::query!()`, `sqlx::query_as!()`) mit compile-time validierten Statements. Parameter werden über `.bind()` übergeben.

**Das ist vorbildlich.** Eines der sichersten Rust-SQL-Patterns überhaupt.

---

## 6. License & Supply Chain

- **License:** MIT — kommerziell nutzbar, keine Copyleft-Restriktionen
- **Dependencies:** 457 Crates im Lockfile
- **cargo-deny:** Nicht installiert (empfohlen für CI)
- **SBOM:** Kein CycloneDX-SBOM vorhanden

**Empfehlung:**
```bash
cargo install cargo-deny
cargo deny check  # Braucht deny.toml Config
```

---

## 7. Network & Runtime Security

| Check | Status |
|---|---|
| Port Binding | Nicht hartgecodet — via `KNOWWHERE_PORT` env. Default vermutlich 3738. |
| Rate Limiting | ✅ `GovernorLayer` + `lazy_limit` in `main.rs`. Config via `RATE_LIMIT` env. |
| Auth | ✅ API-Key via `AuthContext` Middleware. `subtle` crate für constant-time Vergleich. |
| TLS | ⚠️ Kein Built-in-TLS. Erwartet Reverse-Proxy (nginx/caddy). Für Production OK. |

---

## 8. Priorisierte Remediation

| Prio | Kategorie | Action | Aufwand |
|---|---|---|---|
| **P1** | Secrets | `chmod 600 .env.benchmark` | 1 Minute |
| **P2** | Dependencies | `cargo update -p rustls-webpki` (nur mit google-drive Feature) | 2 Minuten |
| **P3** | Supply Chain | `cargo install cargo-deny` + CI-Integration | 30 Minuten |
| **P3** | Code Safety | Safety-Kommentar für `SendableIndex` ergänzen | 5 Minuten |
| **P4** | Panic Surface | Top-10 `unwrap()` in API-Routen ersetzen | 2 Stunden |
| **P4** | SBOM | `cargo cyclonedx --format json` | 5 Minuten |

---

## Gesamtverdict

**B+ — Produktionsreif mit minimaler Remediation.**

KnowWhere hat die Sicherheits-Hygiene eines ernsthaften Rust-Projekts: sqlx-Makros verhindern Injection, subtle crate für timing-sichere Auth, Governor für Rate-Limiting, `.env`-Files korrekt von git ausgeschlossen. Die 5 cargo-audit-Findings sind alle transitiv oder feature-gated — kein einziges betrifft den Core-Pfad.

Die größte technische Schuld ist nicht Sicherheit, sondern Code-Organisation (`api/routes.rs` mit 5884 LOC) und die fehlende Consolidation.

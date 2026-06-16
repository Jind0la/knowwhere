# Implementation Plan: Plugin-System & Core-Bereinigung

**Erstellt:** 16. Juni 2026
**Review:** gStack Full Review Finding K2
**Ziel:** KnowWhere auf Memory-Service-Core reduzieren. Nicht-Core-Features als Plugins auslagern.
**Prinzipien:** Boring by default · Incremental over revolutionary · One module at a time

---

## 1. Ist-Analyse

### Was ist Core (Memory-Service)?

| Modul | LOC | Bleibt | Begründung |
|-------|-----|--------|------------|
| `api/` (14 Module) | ~8.000 | ✅ Core | REST API für Store/Retrieve/Health |
| `memory/` (15 Module) | ~9.500 | ✅ Core | FractalNode, Governance, Dream, Facts |
| `retrieval/` (5 Module) | ~4.500 | ✅ Core | Hybrid BM25+Dense, Cross-Encoder, Scoring |
| `storage/` (5 Module) | ~7.000 | ✅ Core | PostgreSQL + InMemory Backends |
| `embedding/` (5 Module) | ~2.000 | ✅ Core | Ollama/OpenAI/Grok Provider |
| `summarizer/` | 560 | ✅ Core | L0-L2 Summarization |
| `scheduler/` | 1.400 | ✅ Core | Consolidation + Audit Cron |

### Was ist Nicht-Core (→ Plugin)?

| Modul | LOC | Status | Plugin-Strategie |
|-------|-----|--------|-----------------|
| `connectors/frigate.rs` | 80 | Hart verdrahtet in `main.rs` | Feature-Gate `frigate-connector` |
| `connectors/drive.rs` | 234 | Bereits `#[cfg(feature = "google-drive")]` | ✅ Nichts tun |
| `api/webhooks.rs` + `routes/webhooks.rs` | 516 | Hart verdrahtet in `main.rs` | Feature-Gate `webhooks` |
| `vlm/mod.rs` | 1.086 | In `lib.rs` gelistet, kein Feature-Gate | In `embedding/` integrieren ODER Feature-Gate |
| `embedding/audio.rs` | ~200 | Wird von niemandem aktiv genutzt | Feature-Gate `audio-embedding` |
| `embedding/clip.rs` | ~200 | CLIP Vision Embedding | Feature-Gate `vision-embedding` |

---

## 2. Strangler-Fig-Roadmap

**Reihenfolge:** Einfachstes zuerst. Pro Task maximal 200 Zeilen Diff.

```
Phase 1: Frigate (80 LOC, kein Feature-Gate) → 30 min
Phase 2: Webhooks (516 LOC, hart verdrahtet) → 60 min
Phase 3: VLM (1.086 LOC, größe Baustelle) → 90 min
Phase 4: Embedding-Bereinigung (Audio/CLIP) → 30 min
```

---

## 3. Phase 1: Frigate Connector (80 LOC)

**Datei:** `src/connectors/frigate.rs` · `src/main.rs` · `Cargo.toml`

### Task 1.1: Feature-Gate in Cargo.toml

```toml
# Cargo.toml — zu [features] hinzufügen:
frigate-connector = []
```

### Task 1.2: Feature-Gate im Connector

```rust
// src/connectors/frigate.rs — ganz oben einfügen:
//! Frigate NVR event connector — requires `frigate-connector` feature.

// Keine Änderung am bestehenden Code nötig.
// Der Connector ist bereits standalone (keine Abhängigkeiten zu anderen Modulen).
```

### Task 1.3: Feature-Gate in main.rs

```rust
// src/main.rs — Zeile 128-154: Frigate-Startup-Block wrappen:

#[cfg(feature = "frigate-connector")]
{
    if let Ok(frigate_url) = std::env::var("FRIGATE_URL") {
        // ... bestehender Frigate-Startup-Code ...
    }
}
```

### Verification

```bash
# Ohne Feature: Frigate-Code sollte nicht kompilieren
cargo check 2>&1 | grep -i frigate  # → keine Fehler

# Mit Feature: Frigate-Code kompiliert
cargo check --features frigate-connector 2>&1 | grep -i frigate  # → keine Fehler
```

**Geschätzte Diff-Größe:** ~15 Zeilen
**Risiko:** Null. Reiner Feature-Gate-Wrap.

---

## 4. Phase 2: Webhooks (516 LOC)

**Dateien:** `src/api/webhooks.rs` (99 LOC) · `src/api/routes/webhooks.rs` (417 LOC) · `src/main.rs` · `Cargo.toml`

### Task 2.1: Feature-Gate in Cargo.toml

```toml
# Cargo.toml
webhooks = []
```

### Task 2.2: Feature-Gate in webhooks.rs

```rust
// src/api/webhooks.rs — oben einfügen:
//! Webhook endpoints — requires `webhooks` feature.
//! Currently supports: Frigate NVR, HomeAssistant.
```

### Task 2.3: Feature-Gate in routes/webhooks.rs

```rust
// src/api/routes/webhooks.rs — Kopf:
#[cfg(feature = "webhooks")]
// ... gesamter bestehender Code bleibt ...
```

### Task 2.4: Feature-Gate in main.rs

In `main.rs` gibt es drei Stellen die Webhooks referenzieren:

```rust
// Zeile 20: DedupCache Import
#[cfg(feature = "webhooks")]
use knowwhere_server::api::webhooks::DedupCache;

// Zeile 216-219: DedupCache + Secrets im State
#[cfg(feature = "webhooks")]
{
    frigate_dedup: DedupCache::new(),
    frigate_webhook_secret: std::env::var("FRIGATE_WEBHOOK_SECRET").ok(),
    homeassistant_webhook_secret: std::env::var("HASS_WEBHOOK_SECRET").ok(),
}

// Zeile 269-273: Webhook-Routen
#[cfg(feature = "webhooks")]
{
    .route("/webhooks/frigate", post(routes::webhook_frigate))
    .route("/webhooks/homeassistant", post(routes::webhook_homeassistant))
}
```

⚠️ **Achtung:** `DedupCache` ist im AppState. Wenn `webhooks` deaktiviert ist, muss der State ohne diese Felder gebaut werden. Das erfordert ein `#[cfg(feature = "webhooks")]` im `AppState` struct selbst.

### Task 2.5: AppState conditional fields

```rust
// src/api/types.rs — im AppState struct:

pub struct AppState {
    pub store: Arc<dyn StorageBackend>,
    pub embedding: Arc<dyn EmbeddingProvider>,
    pub governance: Arc<RwLock<GovernancePolicy>>,
    // ... andere Felder ...

    #[cfg(feature = "webhooks")]
    pub frigate_dedup: DedupCache,
    #[cfg(feature = "webhooks")]
    pub frigate_webhook_secret: Option<String>,
    #[cfg(feature = "webhooks")]
    pub homeassistant_webhook_secret: Option<String>,
}
```

### Verification

```bash
cargo check                    # Ohne webhooks
cargo check --features webhooks # Mit webhooks
cargo test --lib               # Alle Tests
```

**Geschätzte Diff-Größe:** ~80 Zeilen (meist `#[cfg]` annotations)
**Risiko:** Mittel. State-Struktur-Änderung könnte andere Module treffen.

---

## 5. Phase 3: VLM-Modul (1.086 LOC)

**Datei:** `src/vlm/mod.rs` · `src/lib.rs` · `Cargo.toml`

Das VLM-Modul ist mit 1.086 LOC überdimensioniert. Es enthält:
- Ollama-basierte Vision-Modelle für Embedding
- Fallback-Logik
- Prompt-Templates

**Zwei Optionen:**

### Option A: Feature-Gate (einfacher, schneller)

```toml
# Cargo.toml
vlm = []
```

```rust
// src/lib.rs
#[cfg(feature = "vlm")]
pub mod vlm;
```

Alle Imports von `vlm` in anderen Modulen (main.rs, summarizer, etc.) werden via `#[cfg(feature = "vlm")]` konditional.

### Option B: In embedding/ integrieren (sauberer, mehr Arbeit)

`vlm/mod.rs` → `embedding/vision.rs` (reduziert auf <400 LOC durch Entfernen von Prompt-Templates und Fallback-Logik die nie genutzt werden)

**Empfehlung: Option A jetzt, Option B als Follow-up.** VLM ist nicht kritisch für den Core-Loop.

### VLM-Referenzen finden

```bash
grep -rn "crate::vlm\|use.*vlm" src/ --include="*.rs"
```

### Verification

```bash
cargo check
cargo check --features vlm
cargo test --lib
```

**Geschätzte Diff-Größe:** ~30 Zeilen (Feature-Gate) oder ~400 Zeilen (Integration)
**Risiko:** Niedrig bei Option A, Mittel bei Option B.

---

## 6. Phase 4: Embedding-Bereinigung

### Task 4.1: Audio Embedding feature-gaten

```toml
# Cargo.toml
audio-embedding = []
```

```rust
// src/embedding/mod.rs
#[cfg(feature = "audio-embedding")]
pub mod audio;
```

```rust
// src/embedding/router.rs — alle Audio-Referenzen wrappen
```

### Task 4.2: CLIP/Vision Embedding feature-gaten

```toml
# Cargo.toml
vision-embedding = []
```

```rust
// src/embedding/mod.rs
#[cfg(feature = "vision-embedding")]
pub mod clip;
```

### Verification

```bash
cargo check --no-default-features  # Nur Core
cargo test --lib
```

**Geschätzte Diff-Größe:** ~40 Zeilen
**Risiko:** Niedrig.

---

## 7. Neue Standard-Feature-Flags

```toml
# Cargo.toml — [features]
default = ["reranker"]

# Core (immer an)
# — postgres-storage (opt-in)
# — reranker (default)

# Plugins (alle opt-in)
frigate-connector = []
google-drive = ["dep:google-drive3", "dep:yup-oauth2", "dep:hyper", "dep:hyper-rustls", "dep:hyper-util"]
webhooks = []
vlm = []
audio-embedding = []
vision-embedding = []
openai-provider = []
grok-provider = []
```

---

## 8. Task-Reihenfolge & Abhängigkeiten

```
Task 1.1 → 1.2 → 1.3  (Frigate, keine Abhängigkeiten)
    ↓
Task 2.1 → 2.2 → 2.3 → 2.4 → 2.5  (Webhooks, hängt von keinem ab)
    ↓
Task 3.1 → 3.2  (VLM Feature-Gate, hängt von keinem ab)
    ↓
Task 4.1 → 4.2  (Embedding, hängt von keinem ab)
```

Alle vier Phasen sind **unabhängig voneinander** und können in beliebiger Reihenfolge ausgeführt werden. Die vorgeschlagene Reihenfolge geht von einfach nach komplex.

---

## 9. Kanban-Tasks

| ID | Task | Phase | Geschätzte Zeit | Cursor-fähig? |
|----|------|-------|-----------------|---------------|
| K2-1 | Feature-Gate `frigate-connector` in Cargo.toml + main.rs | 1 | 30 min | ✅ |
| K2-2 | Feature-Gate `webhooks` — Cargo.toml + routes + main.rs + AppState | 2 | 60 min | ✅ |
| K2-3 | Feature-Gate `vlm` — lib.rs + main.rs + alle Referenzen | 3 | 60 min | ✅ |
| K2-4 | Feature-Gate `audio-embedding` + `vision-embedding` | 4 | 30 min | ✅ |
| K2-5 | Integration-Test: `cargo check --no-default-features` muss kompilieren | Alle | 15 min | ✅ |
| K2-6 | `cargo check --all-features` muss kompilieren | Alle | 15 min | ✅ |

---

## 10. Erfolgskriterien

- [ ] `cargo check` (default features) kompiliert → **Core Memory-Service**
- [ ] `cargo check --no-default-features` kompiliert → **Nur Core, keine Plugins**
- [ ] `cargo check --all-features` kompiliert → **Alle Plugins aktiv**
- [ ] `cargo test --lib` (default) → **Alle Tests grün**
- [ ] `main.rs` enthält keine hardgecodeten Frigate/Webhook/VLM-Imports mehr
- [ ] Kein bestehender API-Endpoint verschwunden (Health, Store, Retrieve)

---

## 11. Was NICHT Teil dieses Plans ist

- **Consolidation-Reaktivierung** (K1) — separater Plan nach K2
- **Reranker in Pipeline** (H1) — separater Task
- **Dual-Backend-Dedup** (H2) — separater Task
- **API-Versioning** (H4) — separater Task
- **VLM-Code-Kürzung** — Follow-up nach Feature-Gate

---

## 12. Rollback-Strategie

Jeder Task ist ein einzelner Commit. Falls etwas bricht:

```bash
git revert <commit>   # Rückgängig pro Task, nicht alles auf einmal
```

Feature-Gates sind **additiv** — sie entfernen keinen Code, sie wrappen ihn nur in `#[cfg]`. Rollback ist trivial.

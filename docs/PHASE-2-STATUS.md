# Phase 2 — Connector Webhooks — Status Report

**Erstellt:** 2026-03-27
**Status:** ⚠️ INKOMPLETT — Plan != Implementierung

---

## Update: 2026-03-27 — Integration Test durchgeführt

Die OpenClaw-Integration (Phase 1.5) wurde am 2026-03-27 einem Integrationstest unterzogen.

**Ergebnis:**
- KnowWhere Core-API: ✅ FUNKTIONIERT (90-100% Precision@3 in 30-Node MilaOS-Test)
- OpenClaw `knowwhere-memory` Hook: ❌ EXISTIERT NICHT — kein JavaScript/TypeScript Code gefunden
- OpenClaw Workspace: ❌ EXISTIERT NICHT — `~/.openclaw/workspace/` fehlt
- OpenClaw Plugin Config: ❌ EXISTIERT NICHT — keine `openclaw.json`

**Zwei Bugs gefunden und gefixt (BUG-005, BUG-006):**
1. Leere `query_text` → 500 statt 400 (OLLAMA lehnt leeren String ab)
2. Repetitiver Content ("AAAA...") → 500 statt 400 (OLLAMA lehnt repetitive Inputs ab)

**Fazit:** Phase 1.5 ist in der PRD als "abgeschlossen" markiert, aber der Code existiert nicht. Die OpenClaw-Integration muss noch gebaut werden.

---

## tl;dr

Der ursprüngliche Plan `.cursor/plans/phase_2_connectors_optimiert.plan.md` wurde nach Architektur-Diskussion erstellt, aber die Implementierung wurde **nicht abgeschlossen**. Die TODO-Checkboxen im Plan sind auf "completed" gesetzt, aber der tatsächliche Code fehlt weitgehend.

**Original-Plan verschoben nach:** `docs/archive/phase_2_connectors_plan-ORIGINAL-2026-03-27.md`

---

## Was existiert vs. was der Plan verspricht

| Plan-Punkt | Geplant | Implementiert? | Anmerkung |
|------------|---------|----------------|-----------|
| **Webhook-Infrastruktur** (DedupCache, Secret-Check) | ✅ | ✅ JA | `src/api/webhooks.rs` existiert |
| **POST /webhooks/frigate** | ✅ | ❌ NEIN | Nicht in `routes.rs` |
| **OpenAPI + Integration-Tests** | ✅ | ❌ NEIN | Keine Tests vorhanden |
| **POST /dream/full** | ✅ | ❌ NEIN | Nicht in `routes.rs` |
| **Google Drive Connector** | ✅ | ⚠️ PLACEHOLDER | `drive.rs` gibt nur Dummy-Daten zurück |
| **Drive Deduplizierung** | ✅ | ❌ NEIN | Nicht implementiert |
| **POST /webhooks/homeassistant** | ✅ | ❌ NEIN | Nicht in `routes.rs` |
| **Cross-Modal optional** | ✅ | ⚠️ PLACEHOLDER | Code-Placeholder vorhanden, nicht funktional |
| **Docs: ARCHITECTURE + README** | ✅ | ❌ NEIN | Nicht aktualisiert |

---

## Was wirklich existiert

### ✅ Bereits vorhanden (Phase 1 / Core)

```
src/
├── api/
│   ├── webhooks.rs      # DedupCache + check_webhook_secret() INFRASTRUKTUR NUR
│   ├── routes.rs        # Core API Endpoints (store_session, retrieve_fractal, etc.)
│   ├── auth.rs
│   └── docs.rs
├── connectors/
│   ├── frigate.rs       # FrigateConnector.poll_events() — Polling-Modus funktioniert
│   ├── drive.rs         # PLACEHOLDER — nur Dummy-Daten
│   └── mod.rs           # store_external_event() Helper
├── memory/
│   ├── dream/           # Dream Mode (Consolidation, Audit, Conflict Detection)
│   ├── governance.rs     # Governance Policy Layer
│   └── types.rs         # MemoryType System (5 Typen)
├── storage/
│   ├── in_memory.rs     # USearch + BM25 + RRF
│   └── postgres_store.rs # PostgreSQL Backend
└── main.rs              # Server Setup, Frigate Poller
```

### ❌ Fehlt komplett

- `POST /webhooks/frigate` Endpoint in `routes.rs`
- `POST /webhooks/homeassistant` Endpoint in `routes.rs`
- `POST /dream/full` Admin Endpoint
- `src/api/mod.rs` importiert `webhooks.rs` nicht mal

### ⚠️ Placeholder / Nicht funktional

- `drive.rs` — `poll_changes()` gibt Dummy-Events zurück, keine echte Google Drive API
- Cross-Modal Embedding — Placeholder-Code, kein echter Embedder

---

## Nächste Schritte (falls benötigt)

### Option A: Phase 2 wirklich implementieren

Falls externe Connector-Integration (Frigate Webhook, Home Assistant, Google Drive) gewünscht ist:

1. `POST /webhooks/frigate` in `routes.rs` implementieren
2. `POST /webhooks/homeassistant` in `routes.rs` implementieren
3. `POST /dream/full` Admin-Endpoint implementieren
4. `src/api/mod.rs` um `webhooks` erweitern
5. `drive.rs` mit echter Google Drive API verbinden
6. OpenAPI + Integration-Tests schreiben

### Option B: Erst Core-API verifizieren (aktuell priorisiert)

OpenClaw Integration funktioniert über Core-API:
- `POST /store_session`
- `POST /retrieve_fractal`
- `POST /embed`
- `POST /auth/login`

**Empfohlen:** Erst verifizieren dass Core-API + OpenClaw funktioniert, dann entscheiden ob Phase 2 gebraucht wird.

---

## Historie

| Datum | Event |
|-------|-------|
| 2026-03-21 | Plan erstellt mit 3 Meilensteinen |
| 2026-03-25 | v0.3.0 Released (Core Features complete) |
| 2026-03-27 | Discovery: Plan ≠ Implementierung. Plan archiviert, dieses Dokument erstellt. |
| 2026-03-28 | BUG-007 (count=0) + self_healing.rs Fixes. PostgreSQL-Integration jetzt fully functional (3/3 tests passing). |

---

## Referenzen

- Original Plan (archiviert): `docs/archive/phase_2_connectors_plan-ORIGINAL-2026-03-27.md`
- Core API Docs: `http://localhost:3737/swagger-ui/`
- Bug Tracking: `docs/BUG-TRACKING.md`
- Task Status: `TASKS-code-review-2026-03-21.md`

# Phase 2 — Connector Webhooks — Status Report

**Erstellt:** 2026-03-27
**Letztes Update:** 2026-03-29
**Status:** ✅ OpenClaw-Integration funktioniert — README/STATUS waren stale

---

## Update: 2026-03-29 — OpenClaw Plugin E2E Test

Das OpenClaw-Plugin (`~/.openclaw/extensions/knowwhere/index.js`) wurde heute einem vollständigen E2E-Test unterzogen.

### Testergebnis

| Komponente | Status | Details |
|-----------|--------|---------|
| **Plugin geladen** | ✅ | Alle 6 Hooks registriert |
| **`before_prompt_build`** | ✅ FUNKTIONIERT | Memories werden abgerufen + als `prependContext` injiziert |
| **`message_received`** | ✅ FUNKTIONIERT | Für Gateway-Modus (Telegram, Discord, etc.) |
| **`gateway_start`** | ✅ FUNKTIONIERT | Importiert 5 Sessions beim Gateway-Start (+bestätigt) |
| **`before_reset`** | ✅ FUNKTIONIERT | Speichert Session beim `/reset` |
| **`session_end`** | ✅ FUNKTIONIERT | Für Session-Transitions (Gateway-Modus) |
| **`agent_end`** | ⚠️ Limited | Feuert nur in Gateway-Modus, nicht in embedded `--local` |
| **`session_start` (mapping)** | ✅ FUNKTIONIERT | Mappt sessionId → sessionFile für `session_end` |

### Verifikation: `before_prompt_build`

```
$ openclaw agent --local --session-id test-e2e-final \
  --message "Was weiss du über MilaOS? Wer arbeitet daran?"

[knowwhere] retrieved 5 memories for: "Was weiss du über MilaOS?..."
```

→ Memories werden korrekt abgerufen und dem Modell als Kontext vorgeschaltet.

### Verifikation: `gateway_start` Import

```
Gateway neu gestartet → 5 Sessions aus den letzten 7 Tagen importiert
Node Count: 144 → 150 (+5 Nodes)
```

### Bekannte Limitationen

**Embedded Mode (`openclaw agent --local`)**:
- `agent_end` und `session_end` feuern NICERT zwischen Commands
- Speicher-Lifecycle: Nur via `gateway_start` Import + `before_reset` bei manuellem `/reset`
- Das ist korrektes OpenClaw-Verhalten — kein Bug

**Lösung dafür**: Der Gateway läuft als Daemon. Beim Start werden alle Sessions der letzten 7 Tage importiert. Beim nächsten `openclaw agent --local` sind alle vergangenen Konversationen als Memories verfügbar.

---

## Update: 2026-03-27 — Discovery

Der ursprüngliche TEST-REPORT und PHASE-2-STATUS waren **teilweise falsch/inconsistent**:

- ❌ TEST-REPORT behauptete "Hook existiert nicht" → **falsch**: Das Plugin existierte bereits
- ❌ PHASE-2-STATUS behauptete "Plan ≠ Implementierung" für OpenClaw → **falsch für das Plugin**
- ✅ Das Plugin war aber veraltet (falsche Events, Recall-Loop-Bug)

**Was heute gefixt wurde:**
1. Plugin komplett neu geschrieben mit korrekten OpenClaw Events
2. Recall-Loop Bug behoben (`## Relevant Memories` wird jetzt aus User-Messages gestrippt)
3. `gateway_start` Import-Pipeline hinzugefügt
4. Dokumentation korrigiert

---

## Phase 1.5 — OpenClaw Integration: Status ✅ FERTIG

Das Plugin befindet sich in:
```
~/.openclaw/extensions/knowwhere/
├── index.js          # Plugin Code (neu geschrieben 2026-03-29)
├── index.d.ts        # TypeScript Definitions
├── openclaw.plugin.json  # Plugin Manifest
├── package.json
└── README.md
```

**Konfiguration in `~/.openclaw/openclaw.json`:**
```json
{
  "plugins": {
    "allow": ["knowwhere"],
    "slots": { "memory": "knowwhere" },
    "entries": {
      "knowwhere": {
        "enabled": true,
        "config": {
          "endpoint": "http://127.0.0.1:3737",
          "apiKey": "***",
          "autoRecall": true,
          "autoCapture": true,
          "topK": 5,
          "importLookbackDays": 7
        }
      }
    }
  }
}
```

---

## Phase 2 — Connector Webhooks: Status ⚠️ INKOMPLETT

### Was existiert vs. Plan

| Plan-Punkt | Status | Anmerkung |
|-----------|--------|-----------|
| **Webhook-Infrastruktur** (DedupCache, Secret-Check) | ✅ | `src/api/webhooks.rs` existiert |
| **POST /webhooks/frigate** | ❌ FEHLT | Nicht in `routes.rs` |
| **POST /webhooks/homeassistant** | ❌ FEHLT | Nicht in `routes.rs` |
| **POST /dream/full** Admin-Endpoint | ❌ FEHLT | Nicht in `routes.rs` |
| **Google Drive Connector** | ⚠️ PLACEHOLDER | `drive.rs` gibt nur Dummy-Daten |
| **Cross-Modal Embedding** | ⚠️ PLACEHOLDER | Code-Placeholder, nicht funktional |
| **OpenAPI + Integration-Tests** | ❌ FEHLT | — |

### Was wirklich existiert (Phase 1 + Core)

```
src/
├── api/
│   ├── webhooks.rs      # DedupCache + check_webhook_secret() INFRASTRUKTUR
│   ├── routes.rs        # Core API: store_session, retrieve_fractal, embed, health
│   ├── auth.rs
│   └── docs.rs
├── connectors/
│   ├── frigate.rs       # FrigateConnector.poll_events() — Polling-Modus funktioniert
│   ├── drive.rs         # PLACEHOLDER
│   └── mod.rs
├── memory/
│   ├── dream/           # Dream Mode (Consolidation, Audit, Conflict Detection)
│   ├── governance.rs
│   └── types.rs
├── storage/
│   ├── in_memory.rs      # USearch + BM25 + RRF
│   └── postgres_store.rs  # PostgreSQL Backend
└── main.rs               # Server Setup, Frigate Poller
```

---

## Nächste Schritte

### Phase 2 Connector Webhooks (optional)

Falls externe Connector-Integration gewünscht ist:

1. `POST /webhooks/frigate` in `routes.rs` implementieren
2. `POST /webhooks/homeassistant` in `routes.rs` implementieren
3. `POST /dream/full` Admin-Endpoint implementieren
4. `drive.rs` mit echter Google Drive API verbinden
5. OpenAPI + Integration-Tests schreiben

### Retrieval Quality Test (empfohlen)

Der Testplan `docs/TESTPLAN-RETRIEVAL-QUALITY.md` existiert, wurde aber noch nicht durchgeführt. Die North Star Metric (30-Day Context Fidelity > 92%) ist damit messbar.

---

## Historie

| Datum | Event |
|-------|-------|
| 2026-03-21 | Plan erstellt mit 3 Meilensteinen |
| 2026-03-25 | v0.3.0 Released (Core Features complete) |
| 2026-03-27 | Discovery: OpenClaw-Plugin existiert bereits, aber veraltet |
| 2026-03-28 | BUG-007 + PostgreSQL-Integration gefixt |
| 2026-03-29 | OpenClaw Plugin komplett neu geschrieben + E2E verifiziert ✅ |

---

## Referenzen

- OpenClaw Plugin: `~/.openclaw/extensions/knowwhere/`
- OpenClaw Docs: https://docs.openclaw.ai/
- Core API: `http://localhost:3737/swagger-ui/`
- Bug Tracking: `docs/BUG-TRACKING.md`
- Retrieval Testplan: `docs/TESTPLAN-RETRIEVAL-QUALITY.md`

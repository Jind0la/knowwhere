# KnowWhere Import Guide

> Stand: 4. Maerz 2026 — Lessons Learned aus der OpenClaw-Integration

## Grundprinzip

Wenn KnowWhere in ein bestehendes Agent-System integriert wird, ist der **Import bestehender Memories der allererste Schritt** — noch bevor der Live-Memory-Loop aktiviert wird. KnowWhere ist additiv: es ersetzt kein bestehendes Memory-System, es ergaenzt es.

## Was importiert werden muss

Jedes Agent-System hat eigene Memory-Strukturen. Hier ist eine Checkliste aller typischen Datenquellen:

### 1. Identitaets- und Konfigurationsdateien

| Dateityp | Beispiele | Prioritaet |
|----------|-----------|------------|
| Agent-Identitaet | `IDENTITY.md`, `persona.json`, `agent.yaml` | Hoch |
| User-Profil | `USER.md`, `user_preferences.json` | Hoch |
| Systemanweisungen | `SOUL.md`, `system_prompt.txt`, `AGENTS.md` | Hoch |
| Tool-Konfiguration | `TOOLS.md`, `tool_config.json` | Mittel |
| Bootstrap/Onboarding | `BOOTSTRAP.md`, `setup.md` | Mittel |

**Regel:** Diese Dateien NIEMALS loeschen oder ueberschreiben. Nur lesen und als Session-Nodes importieren.

### 2. Langzeit-Memory-Dateien

| Dateityp | Beispiele | Prioritaet |
|----------|-----------|------------|
| Haupt-Memory | `MEMORY.md`, `long_term_memory.json` | Kritisch |
| Daily Logs | `memory/YYYY-MM-DD.md`, `daily/*.md` | Hoch |
| Conversation Summaries | `memory/summaries/*.md` | Hoch |
| Task Queues | `tasks/QUEUE.md`, `todos.md` | Mittel |

### 3. Agent-Wissen (Sub-Agents, Spezialisierungen)

| Dateityp | Beispiele | Prioritaet |
|----------|-----------|------------|
| Agent-Profile | `agent-profile.md` | Hoch |
| Research-Ergebnisse | `research/*.md` | Hoch |
| Gelernte Lektionen | `memory/agent-lessons/*.md` | Hoch |
| Feedback-Historie | `memory/agent-feedback/*.md` | Mittel |
| Arbeitsergebnisse | Blog-Artikel, Analysen, Design-Konzepte | Hoch |

### 4. Session-Historien

| Dateityp | Beispiele | Prioritaet |
|----------|-----------|------------|
| Aktive Sessions | `sessions/*.jsonl` | Selektiv |
| Archivierte Sessions | `sessions/*.deleted.*`, `*.reset.*` | Selektiv |
| Cron-Logs | `cron/runs/*.jsonl` | Niedrig |

**Achtung bei Sessions:** Nicht blind alles importieren! Sessions enthalten oft:
- Cron-System-Messages (Agent Monitor, Heartbeat etc.) → **ueberspringen**
- Automatisierte Reports → **ueberspringen**
- Echte User-Konversationen → **importieren**
- Sub-Agent-Ergebnisse → **importieren wenn wertvoll**

## Metadata-Schema fuer Imports

Jeder importierte Node bekommt strukturierte Metadaten:

```json
{
  "source": "import:<system>:<agent>:<filename>",
  "imported_from": "<original_path>",
  "import_type": "<category>",
  "agent": "<agent_name>",
  "original_file": "<filename>"
}
```

### Import-Types

| import_type | Beschreibung |
|-------------|--------------|
| `openclaw_workspace` | Haupt-Workspace-Dateien (IDENTITY, USER, SOUL, MEMORY) |
| `openclaw_agent_knowledge` | Sub-Agent-Arbeitsergebnisse und Research |
| `openclaw_project_context` | Projekt-Kontext-Dokumente |
| `openclaw_session` | Importierte Konversationen |
| `langchain_memory` | LangChain ConversationBufferMemory etc. |
| `llamaindex_storage` | LlamaIndex Document Store |
| `custom_import` | Manuell importierte Dateien |

## Ablauf: OpenClaw-Integration (Referenz)

Dies ist der dokumentierte Ablauf aus unserer ersten Integration:

### Phase 1: Discovery (Was existiert?)

```
~/.openclaw/
├── workspace/                    # Haupt-Agent-Workspace
│   ├── IDENTITY.md              # Agent-Identitaet
│   ├── USER.md                  # User-Profil
│   ├── SOUL.md                  # Systemanweisungen
│   ├── TOOLS.md                 # Tool-Konfiguration
│   ├── MEMORY.md                # Langzeit-Memory (KRITISCH!)
│   ├── AGENTS.md                # Multi-Agent-Konfiguration
│   └── memory/                  # Daily Logs + Task Queue
│       ├── YYYY-MM-DD.md
│       └── tasks/QUEUE.md
├── workspace-<agent>/           # Sub-Agent-Workspaces (×6)
│   ├── agent-profile.md
│   ├── research/*.md
│   └── memory/*.md
└── agents/main/sessions/        # Session-Historien
    ├── *.jsonl                  # Aktive Sessions
    ├── *.jsonl.deleted.*        # Archivierte Sessions
    └── *.jsonl.reset.*          # Reset-Summaries
```

### Phase 2: Import-Reihenfolge

1. **MEMORY.md** zuerst — das ist das Herzstuck des bestehenden Gedaechtnisses
2. **IDENTITY.md + USER.md** — Agent- und User-Identitaet
3. **SOUL.md + TOOLS.md + AGENTS.md** — Systemkonfiguration
4. **Daily Memory-Dateien** — `memory/*.md`
5. **Sub-Agent-Profile und Research** — `workspace-*/agent-profile.md`, `workspace-*/research/*.md`
6. **Sub-Agent-Arbeitsergebnisse** — Blog-Artikel, Analysen, Design-Konzepte
7. **Projekt-Kontext-Dokumente** — aus Sessions extrahierte Kontextdokumente

### Phase 3: Konfiguration anpassen

Nach dem Import wird das Host-System so konfiguriert, dass KnowWhere als zusaetzliche Memory-Schicht arbeitet:

1. **SOUL.md** — Abschnitt anhaengen (nicht ersetzen!) der KnowWhere als zusaetzliche Erinnerungsquelle beschreibt
2. **TOOLS.md** — Abschnitt anhaengen mit KnowWhere-Hinweisen
3. **Plugin installieren** — Live-Memory-Loop aktivieren (store + retrieve + inject)

### Phase 4: Verifizierung

Test-Queries ueber alle importierten Domaenen:
- Persoenliche Infos ("Wer ist der User?")
- Business-Wissen ("Welche Konkurrenten?")
- Technisches Wissen ("Wie funktioniert X?")
- Historische Entscheidungen ("Warum haben wir Y gewaehlt?")

## Ergebnis der OpenClaw-Integration

| Kategorie | Dateien | Nodes | Inhalt |
|-----------|---------|-------|--------|
| Workspace-Dateien | 6 | 6 | MEMORY, IDENTITY, USER, SOUL, AGENTS, MilaOS Context |
| Daily Memory | 9 | 9 | Tageslogst, Konversationen, Reports |
| Business Agent | 10 | 10 | Blog-Artikel, GDPR, Outreach, Research |
| Research Agent | 10 | 10 | Konkurrenz, Markt, Keywords, Patent, Positionierung |
| Designer Agent | 5 | 5 | Logo, Design-Konzept, Fractal Viz |
| Dev Agent | 6 | 6 | Tests, Demo-URL, Supabase, Project Context |
| Marketing Agent | 3 | 3 | SEO-Checklist, Product Positioning |
| User-Konversationen | ~25 | 25 | Echte Telegram-Gespraeche |
| **Gesamt** | **~74** | **100** | **Vollstaendiges Organisationsgedaechtnis** |

## Bekannte Host-Systeme und ihre Memory-Strukturen

### OpenClaw

- **Workspace:** `~/.openclaw/workspace/`
- **Memory-Dateien:** `MEMORY.md`, `memory/*.md`, `IDENTITY.md`, `USER.md`, `SOUL.md`
- **Sessions:** `~/.openclaw/agents/main/sessions/*.jsonl`
- **Sub-Agents:** `~/.openclaw/workspace-<name>/`
- **Besonderheit:** Cron-Jobs erzeugen viel Noise in Sessions — filtern!
- **Warnung:** `openclaw configure` und `openclaw doctor` koennen Workspace-Dateien stillschweigend ueberschreiben

### LangChain

- **ConversationBufferMemory:** In-memory, muss vor Shutdown exportiert werden
- **ConversationSummaryMemory:** Summary-Strings → direkt als Session-Nodes importieren
- **VectorStoreRetrieverMemory:** Bestehende Vektoren koennen als Nodes mit Pre-computed Embeddings importiert werden
- **Persistierte Memories:** Oft in SQLite, Redis oder Files — Pfade variieren

### LlamaIndex

- **Document Store:** `storage/docstore.json` — Dokumente mit Metadaten
- **Index Store:** `storage/index_store.json` — Index-Konfigurationen
- **Vector Store:** Verschiedene Backends (Chroma, Pinecone, etc.)
- **Chat Store:** `storage/chat_store.json` — Konversationshistorie

### CrewAI / AutoGen

- **Agent Memory:** Oft in-memory, manchmal persistiert in JSON/YAML
- **Task Results:** Ergebnisse von abgeschlossenen Tasks
- **Shared Knowledge:** Geteiltes Wissen zwischen Agents

### Cursor / IDE-basierte Agents

- **Agent Transcripts:** `agent-transcripts/*.jsonl`
- **Cursor Rules:** `.cursor/rules/*.mdc`
- **Conversation Context:** Implizit in Session-State

## Zukuenftige Vision: Auto-Discovery

> Phase 2.5 in der Roadmap

KnowWhere soll Host-Systeme automatisch erkennen und importieren koennen:

```
POST /import/discover
{
  "scan_paths": ["~/.openclaw", "~/.langchain", "~/projects"]
}

Response:
{
  "systems_found": [
    {
      "type": "openclaw",
      "path": "~/.openclaw",
      "workspaces": 6,
      "memory_files": 42,
      "sessions": 159,
      "estimated_nodes": 120,
      "recommended_imports": [
        { "path": "workspace/MEMORY.md", "priority": "critical", "reason": "Main agent memory" },
        { "path": "workspace/USER.md", "priority": "high", "reason": "User identity" }
      ]
    }
  ]
}

POST /import/execute
{
  "system": "openclaw",
  "path": "~/.openclaw",
  "filter": {
    "skip_cron": true,
    "skip_system_messages": true,
    "min_content_length": 50
  }
}
```

### Erkennungs-Heuristiken

| System | Erkennungsmerkmal |
|--------|-------------------|
| OpenClaw | `~/.openclaw/openclaw.json` existiert |
| LangChain | `langchain` in requirements.txt/pyproject.toml |
| LlamaIndex | `llama_index` in requirements.txt, `storage/` Ordner |
| CrewAI | `crewai` in requirements.txt, `agents.yaml` |
| Cursor | `.cursor/rules/` Ordner, `agent-transcripts/` |
| Custom | Manueller Scan nach `*.md`, `*.json`, `*.jsonl` mit Memory-Patterns |

### Inhalts-Klassifizierung

KnowWhere soll importierte Inhalte automatisch klassifizieren:

| Pattern | Klassifikation | Aktion |
|---------|---------------|--------|
| `MEMORY.md`, `memory`, `long_term` | Langzeit-Memory | Import als Session-Node |
| `IDENTITY`, `persona`, `agent_profile` | Identitaet | Import + als Referenz markieren |
| `USER`, `user_pref`, `human` | User-Profil | Import + als Referenz markieren |
| `research/`, `analysis`, `report` | Wissens-Artefakt | Import mit Agent-Attribution |
| `*.jsonl`, `sessions/` | Konversation | Selektiv: nur User-Messages importieren |
| `heartbeat`, `monitor`, `cron` | System-Noise | Ueberspringen |
| `backup`, `.bak`, `.old` | Backup | Nur importieren wenn Original fehlt |

# KnowWhere — Integration Test Plan: OpenClaw Memory Loop

> **Zweck:** Verifizieren, dass KnowWhere den vollständigen Memory-Loop korrekt implementiert — von Nachricht speichern über Retrieval bis zur Kontext-Injektion — bevor eine echte OpenClaw-Integration gebaut wird.
>
> **Hypothese:** KnowWhere Core-API funktioniert korrekt (store + retrieve + embed). Die OpenClaw-Integration ist geplant aber nie als Code implementiert worden. Dieser Test validiert den Core, damit wir wissen was funktioniert wenn wir OpenClaw drankleben.

---

## Test-Szenario: "MilaOS Design Journey"

Wir simulieren einen User, der über 3 Monate (fiktiv) an einem Projekt namens **"MilaOS"** gearbeitet hat — ein anonymer Smart-Home-Assistent. Das Szenario ist realistisch: erste Idee, Design-Entscheidungen, Technologie-Wahl, Rückschläge, Budget-Änderungen, Team-Entscheidungen.

### Warum MilaOS?
- Anonym: kein Login → passt zu KnowWhere's Vision
- Smart-Home-Kontext → realistisch für Nimar's Use-Case
- 3-Monate-Timeline → zeigt "Retroactive Retrieval"

---

## Phase 1: Datenspeisung (30 Nodes)

### Topic-Buckets

| Bucket | Thema | Nodes | Beispiel-Nachrichten |
|--------|-------|-------|---------------------|
| **A** | Projekt-Start, Vision | 5 | "Ich will einen anonymen Smart-Home-Assistenten", "Kein Login, keine Cloud" |
| **B** | Design-Entscheidungen | 8 | "Farbschema: Dunkelgrau + Amber", "Minimalistisch, keine Charts", "Wireframes für iOS" |
| **C** | Technologie-Stack | 6 | "Rust Backend mit Axum", "nomic-embed-text-v2-moe für Embeddings", "PostgreSQL für Persistenz" |
| **D** | Rückschläge | 5 | "Frigate Integration hat 3 Wochen gedauert", "Ollama Embedding Qualität mäßig", "Budget für Server: 50€/Monat max" |
| **E** | Team-Entscheidungen | 4 | "Nimar macht Design allein", "Agent: 'Max' als Projektleiter", "Wöchentliche Reviews" |
| **F** | Aktuelle Gedanken | 2 | "Launch Ende April", "Erste Beta-Tester gesucht" |

### Gespeicherte Timestamps (fiktiv über 3 Monate verteilt)

```
2026-01-15 — Bucket A (Projektstart)
2026-01-22 — Bucket A + B (erste Design-Ideen)
2026-02-01 — Bucket B (Wireframes)
2026-02-08 — Bucket C (Tech-Stack-Entscheidung)
2026-02-15 — Bucket D (Frigate-Rückschlag)
2026-02-22 — Bucket C (Embedding-Modell gewechselt)
2026-03-01 — Bucket E (Team-Entscheidung)
2026-03-08 — Bucket D (Budget-Diskussion)
2026-03-15 — Bucket B (Design-Revision)
2026-03-22 — Bucket F (Launch-Planung)
```

Jede Node bekommt Metadata:
```json
{
  "source": "user:Nimar",
  "session_id": "milaos-design-journey",
  "topic_bucket": "A|B|C|D|E|F",
  "fictional_date": "YYYY-MM-DD"
}
```

---

## Phase 2: Retrieval Queries (10 Tests)

### Retrieval-Test-Matrix

| # | Query | Erwartete Topic-Buckets | Warum |
|---|-------|--------------------------|-------|
| 1 | "Was war unsere erste Idee für das Projekt?" | A | Frühester Zeitpunkt, Vision |
| 2 | "Welche Farben haben wir für das Design gewählt?" | B | Direkte Design-Entscheidung |
| 3 | "Warum haben wir uns gegen Cloud-Login entschieden?" | A + D | Anonymität + Datenschutz-Diskussion |
| 4 | "Was war das größte technische Problem?" | D | Frigate-Rückschlag |
| 5 | "Welches Embedding-Modell nutzen wir?" | C | Technologie-Entscheidung |
| 6 | "Wie ist das Budget verteilt?" | D + E | Serverkosten + Team |
| 7 | "Wer ist 'Max' im Team?" | E | Team-Struktur |
| 8 | "Wann ist der Launch geplant?" | F | Aktuelle Planung |
| 9 | "Erzähl mir von den Wireframes" | B | Design-Artefakte |
| 10 | "Was waren die wichtigsten Entscheidungen?" | A + B + C + E | Übergreifend |

### Erfolgskriterien pro Query

- **Precision@3:** Top-3 Results müssen mindestens 1 Node aus den erwarteten Buckets enthalten
- **Score-Schwelle:** Top-1 Score sollte > 0.01 (nicht triviale Ähnlichkeit)
- **Keine Halluzinationen:** Results müssen echte, gespeicherte Inhalte sein
- **Fractal-Zoom:** Bei Buckets mit Kind-Nodes sollte Zoom funktionieren

---

## Phase 3: Fractal-Zoom Tests

### Test: "Deep Dive Design"

1. Speichere eine Parent-Node: "MilaOS Design Guide v1.0"
2. Speichere 3 Child-Nodes: "Farbschema", "Typografie", "Layout"
3. Retrieval-Query: "Was steht im Design Guide?"
4. Erwartung: Parent-Node kommt zurück + Children werden reingedichtet (zoom_retrieve)

### Test: "Thread Retrieval"

1. Speichere 5 aufeinanderfolgende Nachrichten als Thread
2. Retrieval: "Was war der letzte Stand zum Design?"
3. Erwartung: Neueste Node im Thread hat highest score

---

## Phase 4: Edge Cases

| Test | Input | Erwartung |
|------|-------|-----------|
| **Leere Query** | `""` | Graceful handling, keine Panic |
| **Kein Match** | "Atomkernfusion im Wohnzimmer" | Leerer Array oder sehr niedrige Scores |
| **Duplikat-Speicherung** | Dieselbe Nachricht 2x speichern | 2 Nodes oder Dedup-Warnung |
| **Sehr lange Nachricht** | 2000 Zeichen | Embedding + Storage funktioniert |
| **Sonderzeichen** | "App 💾 mit 🌐 und 🤖!" | Embedding ignoriert Emoji korrekt |
| **Cross-Topic Query** | "Budget und Design zusammen" | Nodes aus 2 Buckets in Top-3 |

---

## Phase 5: System-Integrität

### Health & Metriken

| Check | Methode | Erwartung |
|-------|---------|-----------|
| Server läuft | `GET /health` | `{"status": "ok", "nodes": N, ...}` |
| Auth funktioniert | `POST /auth/login` | Token wird zurückgegeben |
| Node-Count nach Speisung | `GET /nodes/recent?limit=30` | Genau 30 Nodes |
| Embedding-Dimension | Embedding-Vektor Länge | 768 (nomic-embed-text-v2-moe) |

---

## Kennzahlen ( KPIs )

| Metrik | Ziel | Messmethode |
|--------|------|-------------|
| **Recall@5** | ≥ 80% | Bei jeder Query: erwartete Buckets in Top-5? |
| **Precision@3** | ≥ 70% | Top-3 contain korrekte Bucket-Nodes? |
| **Embedding-Time** | < 500ms pro Node | Zeit von POST /store_session bis 200 OK |
| **Retrieval-Time** | < 1s für 30 Nodes | Zeit von POST /retrieve_fractal bis Response |
| **Score-Distribution** | Max-Score sinnvoll | Top-1 > 0.01, nicht alle gleich |
| **Zero-Results Rate** | < 10% | Bei sinnvollen Queries |

---

## Test-Ablauf (Chronologisch)

```
[Tue 11:00] Phase 0: Prerequisites checken
             - KnowWhere Server starten (Port 3737)
             - Ollama prüfen + Modell verfügbar
             - Auth-Token holen

[Tue 11:10] Phase 1: Datenspeisung (30 Nodes)
             - 30 store_session Calls mit MilaOS-Content
             - Timestamps über 3 Monate fiktiv verteilt
             - Nach jedem Call: Bestätigung + node_id

[Tue 11:25] Phase 2: Retrieval Queries (10 Tests)
             - 10 retrieve_fractal Calls
             - Pro Query: Precision@3 manuell prüfen
             - Scores und Latenzen loggen

[Tue 11:40] Phase 3: Fractal-Zoom
             - Parent-Child Nodes speichern
             - Zoom-Query absetzen
             - Children im Response prüfen

[Tue 11:50] Phase 4: Edge Cases
             - 6 Edge-Case Tests
             - System-Integrität

[Tue 12:00] Phase 5: Ergebnis-Dokumentation
             - KPI-Tabelle ausfüllen
             - Screenshots der wichtigsten Responses
             - Fazit: "OpenClaw-Integration: ready / nicht-ready"
```

---

## Erfolgskriterien für "Go/No-Go" OpenClaw-Integration

**GO** wenn:
- Recall@5 ≥ 80%
- Precision@3 ≥ 70%
- Embedding-Time < 500ms
- Retrieval-Time < 1s
- Keine Panics, keine 5xx Errors

**NO-GO** wenn:
- Recall@5 < 60% ODER
- Retrieval häufig leer bei sinnvollen Queries ODER
- Server instabil

**CONDITIONAL** wenn:
- Retrieval funktioniert aber langsam
- Precision gut aber Recall schlecht → Retrieval-Parameter tune

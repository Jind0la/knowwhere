# GOAL-Challenge: Warum mein erstes Goal scheitern würde

## Das Goal-System verstehen

Hermes `/goal` funktioniert als **Ralph-Loop**: Nach jedem Turn prüft ein Judge-Model,
ob das Goal erreicht ist. Wenn nicht → Auto-Continue. Wenn ja → Done. Max 20 Turns.

## Warum "PersonaMem 70% Accuracy" als Goal FAILED

### 1. Nicht selbst-verifizierbar in einem Turn
Der Benchmark läuft 15+ Minuten als Hintergrundprozess. Nach Turn 1 (Code schreiben)
kann der Judge nicht prüfen ob Accuracy 70% ist — der Benchmark läuft noch nicht mal.
Nach Turn 2 (Benchmark starten) ebenso. Der Judge wird entweder:
- Zu früh "done" sagen (Code ist committed, aber nicht getestet)
- Das Budget verbrennen mit "continue" während der Benchmark noch läuft

### 2. Outcome hängt von externen Nicht-Determinismen ab
Gemini Flash (Claims), Gemini Pro (Answer), nomic-embed-text (Embedding) —
alle drei liefern nicht-deterministische Ergebnisse. 70% könnte beim ersten Run
klappen, beim zweiten nicht. Das Goal-System kann nicht zwischen "Code ist falsch"
und "LLM hatte einen schlechten Tag" unterscheiden.

### 3. Mischt Implementierung und Validierung
Gutes Goal: "Fix every lint error, verify ruff check passes" → `ruff check` = exit code 0 = done.
Mein Goal: "Baue Feature UND führe 15-Minuten-Benchmark durch UND erreiche magische Zahl" —
drei Schritte in einem Goal, wovon zwei async sind.

### 4. Falsches Pattern für /goal
Die Docs sagen: "Tasks where you'd otherwise have to say 'keep going' three times".
Mein Goal ist ein Ein-Turn-Task (Code schreiben) + Async-Warten. Das /goal-System
ist für iterative "schreiben → testen → fixen → testen"-Loops optimiert, nicht für
"starte Langzeit-Benchmark und warte auf Ergebnis".

## Was STATTDESSEN sinnvoll ist: Goal-Kaskade

### Goal 1: Turn-Index in der Claim-Extraktion
```
/goal Add turn_index to claim extraction in knowwhere.py. Every claim stored
via store_external must have turn_index in metadata reflecting its position
in the conversation. Write a Python test that ingests a sample conversation,
retrieves claims via the KnowWhere API, and verifies every claim has
metadata.turn_index with the correct sequential value. Run with:
uv run python -m pytest test_turn_index.py -v. Fix until green.
```

**Selbst-verifizierbar:** ✓ `pytest` exit code 0  
**Bounded:** ✓ Nur knowwhere.py + ein Test-File  
**Prozess-orientiert:** ✓ Der Agent kann `pytest` selbst ausführen  
**Judge-kompatibel:** ✓ Nach jedem Turn: "Ist der Test grün?" → klar messbar

### Goal 2: Timeline-Context-Template
```
/goal Add a timeline-structured context template to knowwhere.py retrieval.
When the query contains temporal markers ("how did X change", "used to",
"previously", "evolution", "shifted"), sort retrieved claims by turn_index
and format them as "## Timeline: {topic}" with each claim on its own line
prefixed by its turn number. Write a test that verifies the context output
for a temporal query contains "## Timeline" header. Run with:
uv run python -m pytest test_timeline_template.py -v. Fix until green.
```

### Goal 3: Validierungs-Benchmark
```
/goal Run the PersonaMem benchmark with timeline claims enabled:
uv run omb run --dataset personamem --split 32k --memory knowwhere
--query-limit 20 --name knowwhere-timeline-v1
Wait for completion. Read the output JSON. If accuracy < 65%,
analyze the failed queries and iterate on the context template or
claim extraction parameters. If accuracy >= 65%, report success.
```

### Goal 4: Full-Scale Run (nur wenn Goal 3 ≥65%)
```
/goal Run the full PersonaMem 589-query benchmark with timeline claims:
uv run omb run --dataset personamem --split 32k --memory knowwhere
--name knowwhere-timeline-full
Wait for completion (~25 min). Report final accuracy, retrieval time avg,
and context token stats from the output JSON.
```

## Lernpunkt

`/goal` ist kein "setze eine Vision und der Agent macht alles". Es ist ein
**iterativer Build-Test-Fix-Loop-Automat**. Jedes Goal muss nach EINEM Turn
verifizierbar sein — durch einen Test, einen CLI-Befehl, einen Exit-Code.
Alles andere (Langzeit-Benchmarks, Accuracy-Zahlen, externe APIs) gehört
in die MANUELLE Validierung nachdem die Goals durch sind.

# KnowWhere v0.6 — Temporale Golden Queries (Phase 0)

**Ziel:** Saubere, reproduzierbare Testfälle für Preference-Evolution, Timeline und Before/After Queries.

**Prinzip:** Queries müssen explizit temporal sein und auf echte Preference- oder Decision-Entwicklung abzielen.

## 1. Preference-Evolution Queries (Kern für PersonaMem Lift)

1. "Wie hat sich meine Einstellung zu Musik über die Zeit verändert?"
2. "Zeige mir die Entwicklung meiner Präferenzen bei [Thema] von den ersten Turns bis jetzt."
3. "Was habe ich früher über [Thema] gesagt und wie hat sich meine Meinung später geändert?"
4. "Preference Shift: Meine Haltung zu [Projekt/Entscheidung] früher vs. heute."
5. "Welche Präferenzen habe ich in den letzten Sessions revidiert oder aufgegeben?"

## 2. Timeline & Sequence Queries

6. "Erstelle eine Timeline meiner wichtigsten Entscheidungen der letzten Sessions."
7. "Was war die Reihenfolge meiner Claims zu [Thema]?"
8. "Zeige mir Before/After zu meiner Meinung über [X]."
9. "Welche Claims kamen zuerst und welche später bei [Thema]?"
10. "Chronologische Entwicklung meiner Gedanken zu [Projekt]."

## 3. Change & Revision Detection

11. "Wo habe ich meine Meinung zu [Thema] revidiert oder korrigiert?"
12. "Was habe ich früher positiv gesehen und später kritisch betrachtet?"
13. "Preference Evolution: Von [früherer Zustand] zu [aktueller Zustand] bei [Thema]."
14. "Welche Turns zeigen eine klare Änderung meiner Präferenz?"

## 4. Kontroll-Queries (für Baseline-Vergleich)

15. "Was ist mein aktueller Stand zu [Thema]?" (nicht temporal)
16. "Finde alle Claims zu [Thema]." (semantisch, nicht zeitlich)

## Verwendung

Diese Queries sollen:
- In AMB-ähnlichen Benchmarks verwendet werden
- Für manuelle Verification von retrieve_fractal mit temporalem Boost dienen
- Als Grundlage für die PersonaMem 20q Erweiterung dienen

**Nächster Schritt:** Diese Queries in die Baseline-Runner integrieren und mit aktuellem Stand (bge-m3, 2405 Nodes) messen.
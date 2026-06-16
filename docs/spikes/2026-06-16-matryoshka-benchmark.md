# Spike: Matryoshka-Dimensions-Benchmark

**Erstellt:** 16. Juni 2026
**Typ:** Benchmark / Spike
**Finding:** gStack Review D4 — Matryoshka 64d vs 128d vs 256d Precision-Verlust ungemessen

---

## Ziel

Messen ob `expand_fractal()` Depth-2 mit 64d-Trunkierung messbaren Mehrwert gegenüber Depth-1 (256d) allein bringt. Und: ist 128d der bessere Sweet Spot?

## aktueller Stand

```rust
// src/storage/postgres_store.rs:2084-2111
// Depth 1: 256d → 10 Nachbarn → verify mit 768d cosine
// Depth 2: 64d  →  5 Cluster → verify mit 768d cosine (threshold ×0.8)
```

Die Funktion `matryoshka_continuity()` existiert bereits in `src/memory/fractal_node.rs:72` — misst (full_sim, truncated_sim) für beliebige Dimensionen. Sie wird aktuell **in keinem Benchmark genutzt**.

---

## Aufgaben

### Task D4-1: Benchmark-Skript schreiben

**Datei:** `scripts/bench_matryoshka_dimensions.sh` oder `.rs`

1. Nimm 100 zufällige Node-Paare aus der DB
2. Für jedes Paar: `matryoshka_continuity(a, b, dim)` für dim ∈ {64, 128, 256, 512}
3. Berechne: Mean Cosine-Drift (|full_sim - trunc_sim|) pro Dimension
4. Berechne: Korrelation (Pearson) zwischen full_sim und trunc_sim
5. Ausgabe: Tabelle

```
dim | mean_drift | correlation | pairs_above_0.7
64  | 0.12       | 0.78        | 82%
128 | 0.06       | 0.91        | 91%
256 | 0.02       | 0.97        | 97%
512 | 0.01       | 0.99        | 99%
```

### Task D4-2: Depth-1-only vs Depth-1+Depth-2 A/B-Test

1. Modifiziere `expand_fractal()` temporär: `max_depth=1` only
2. Führe LongMemEval 42-case Benchmark → miss Recall@5
3. Modifiziere zurück: `max_depth=2` (Status Quo)
4. Führe Benchmark erneut → vergleiche
5. Ergebnis: Δ Recall@5 durch Depth-2

### Task D4-3 (conditional): 64d vs 128d Vergleich

Nur wenn D4-2 zeigt dass Depth-2 positiven Beitrag leistet:

1. Ändere Depth-2 von 64d auf 128d
2. Führe Benchmark → vergleiche mit 64d
3. Entscheide: 64d behalten oder auf 128d upgraden

---

## Erfolgskriterien

- [ ] `matryoshka_continuity()` ist in einem Benchmark gehookt
- [ ] Mean Drift pro Dimension ist gemessen
- [ ] Recall@5 Impact von Depth-2 ist quantifiziert
- [ ] Entscheidung 64d/128d ist datengestützt

## Geschätzte Zeit

| Task | Zeit |
|------|------|
| D4-1 (Skript) | 45 min |
| D4-2 (A/B-Test) | 30 min |
| D4-3 (64d vs 128d) | 30 min |

**Gesamt:** ~2h (Cursor-fähig, alle Tasks sind Rust/Python-Scripting)

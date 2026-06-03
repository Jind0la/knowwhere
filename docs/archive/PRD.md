# KnowWhere — Product Requirements Document

> Stand: Mai 2026 — Repository `main`, Version `0.5.0`

## 1. Produktname und One-Sentence Pitch

**KnowWhere**

"Ein lossless fractal memory substrate für KI-Agenten — jede Information hat eine Adresse in der Hierarchie, nichts geht verloren."

KnowWhere ist kein Faktenspeicher der Konversationen zu isolierten Aussagen reduziert. Es ist eine Wissens-Infrastruktur die jede Information in einer durchsuchbaren, durchzoom-baren Fraktal-Struktur ablegt. Sessions bleiben als vollständige Einheiten erhalten. Externe Daten werden per Pointer referenziert. Retrieval funktioniert auf jeder Auflösungsebene — von der Übersicht bis zum atomaren Fakt.

## 2. Das Problem

Heutige KI-Memory-Systeme haben drei fundamentale Schwächen:

1. **Informationsverlust durch Extraktion.** Systeme wie Hindsight extrahieren "Fakten" aus Konversationen und verwerfen den Rest. Nuancen, Kontext, Subtext — alles was der Extraktor nicht als Fakt erkennt, ist für Retrieval unsichtbar.
2. **Keine Hierarchie.** Flache Vektor-Datenbanken (Pinecone, Weaviate) kennen nur eine Ebene. Es gibt keine Möglichkeit, von einer Übersicht in die Details zu zoomen.
3. **Keine Provenance.** Woher kommt eine Information? Wie vertrauenswürdig ist sie? Wurde sie vom Nutzer gesagt oder vom System synthetisiert? Bestehende Systeme beantworten diese Fragen nicht.

## 3. Produktprinzipien

1. **Lossless.** Keine Information wird durch Extraktion oder Consolidation verloren. Originaldaten bleiben immer über Fractal Zoom erreichbar.
2. **Pointer-first.** Externe Quellen werden als Pointer plus Metadaten gespeichert, nie als Rohdaten-Kopien.
3. **Fractal Hierarchy.** L0 (atomic) → L1 (summary) → L2 (overview). Suche auf jeder Ebene, zoome in Details.
4. **Typed Memory.** 6 Typen mit typspezifischer Consolidation-Logik: Episodic, Semantic, Preference, Procedural, Decision, Meta.
5. **Trust-aware.** Jeder Knoten hat einen auto-detektierten Trust Tier (primary/reference/derived/volatile) der das Retrieval-Ranking beeinflusst.
6. **Additiv, niemals destruktiv.** Host-Systeme werden ergänzt, nicht ersetzt.

## 4. Zielbild

Ein Nutzer oder Agent-Betreiber soll:

- Kontext über Monate verlustfrei wiederfinden
- Von Übersichten (L2) zu atomaren Fakten (L0) zoomen können
- Den gesamten Entscheidungspfad nachvollziehen — nicht nur das Endergebnis
- Wissen mit voller Provenance speichern und abrufen

## 5. Aktueller Produktumfang (v0.5.0)

### 5.1 Kernfunktionalität

| Bereich | Beschreibung | Status |
|---------|-------------|--------|
| `store_session` | Session als vollwertige Memory mit auto-chunking speichern (inkl. memory_type parsing) | ✅ |
| `store_external` | Externe Referenz pointer-first speichern | ✅ |
| `retrieve_fractal` | Hybrid Retrieval mit Fractal Zoom, Profilen und memory_type_filter | ✅ |
| **Decision Scoring** | Decision-Nodes: PRIMARY trust tier (1.18×) + 1.5× memory_type_multiplier = 2× boost | ✅ |
| `POST /rerank` | Cross-Encoder Reranking (bge-reranker-v2-m3 via ONNX, feature: reranker) | ✅ |
| `chat/subconscious` | Retrieval-gestützte Antwort mit Quellenangaben | ✅ |
| `dream/status` | Compaction Scheduler Status mit Space-Amplification Trigger | ✅ |
| `governance/policy` | Governance Policy lesen/setzen | ✅ |
| HomeAssistant Webhook | POST /webhooks/homeassistant, Dedup + Secret | ✅ |
| Cross-Modal Embedding | EmbeddingRouter: CLIP/Whisper/Sensor → 768-dim vector space | ✅ |
| Reflect Mode | Query-Time Memory Synthesis via Ollama | ✅ |
| Claims Extraction | Structured claim parsing: Summary→Claims→Decision Nodes | ✅ |
| Event-Driven Consolidation | Write-triggered compaction (ersetzt Timer-Polling) | ✅ |
| POST /consolidation/force | Admin-triggered full re-consolidation | ✅ |
| Transient Error Resilience | DNS/Ollama failures don't mark nodes as processed | ✅ |
| PostgreSQL Tier Persistence | Full roundtrip for fractal tier fields through PostgreSQL | ✅ |
| Hermes MemoryProvider | Per-turn crash-safe storage + dual retrieval (episodic + decision) | ✅ |

### 5.2 6-Type Memory System

| Typ | Beschreibung | Consolidation-Logik | Halbwertszeit |
|-----|-------------|--------------------|--------------|
| **Episodic** | Ereignisse, Session-Fakten | Hohe temporale Sensitivität, wird zu Semantic summarized | 7 Tage |
| **Semantic** | Stabilisiertes Wissen, Fakten | Konflikt- und Supersession-fähig | 90 Tage |
| **Preference** | Persönliche Präferenzen | Version-sensitive, alte Versionen archiviert | 30 Tage |
| **Procedural** | Regeln, Workflows, How-to | Governance-kritisch, Änderungen nur mit Override | 180 Tage |
| **Decision** | Architektur-/Design-Entscheidungen | Immutable & Traceable | Unendlich / Immutable |
| **Meta** | Metakognitives Wissen über das System | Audit-kritisch | 14 Tage |

### 5.3 Trust Tiers & Scoring

| Tier | Beschreibung | Tier-Multiplier | Beispiel-Typen |
|------|-------------|----------------|---------------|
| **primary** | Direkte Nutzereingaben, Decision-Nodes, importierte Kernartefakte | 1.18× | Decision, Episodic(user), Import(MEMORY.md) |
| **reference** | Dokumente, manuelle Einträge | 1.0× | Semantic, Procedural(Manual) |
| **derived** | Assistant-Outputs, System-Zusammenfassungen, Consolidation | 0.88× | Episodic(assistant), Semantic(Consolidation) |
| **volatile** | Unsichere oder temporäre Daten | 0.72× | Meta(temp) |

**Memory-Type-Multipliers (zusätzlich zum Tier-Multiplier):**

| Typ | Multiplier | Begründung |
|-----|-----------|-----------|
| Decision | 1.5× | Strukturierte Entscheidungen sind die höchstwertigen Fakten |
| Procedural | 1.2× | How-to-Wissen ist hochwertig |
| Episodic | 0.85× | Konversations-Chatter ist weniger wertvoll |
| Andere | 1.0× | Neutral |

**Gesamtformel:** `final_score = base_score × tier_multiplier × memory_type_multiplier`

Beispiel Decision-Node: `base × 1.18 × 1.5 = base × 1.77` (vorher: `base × 0.88 × 1.0 = base × 0.88`, weil Decision als DERIVED und ohne Type-Boost). Effektiver Boost: +101%.

Tiers werden automatisch aus Metadaten (role, derivation, source) erkannt.

### 5.4 L2→L1→L0 Fractal Compaction

| Ebene | Inhalt | Generierung |
|-------|--------|-------------|
| **L0 (Raw)** | Originaltext der Session-Runde | Direkt gespeichert |
| **L1 (Summary)** | Paragraph-Zusammenfassung mehrerer L0s | LocalSummarizer (Ollama qwen2.5:3b, temp=0) |
| **L2 (Overview)** | Ein-Satz-Zusammenfassung mehrerer L1s | LocalSummarizer + VLM-Fallback-Chain |

Compaction ist deterministisch (temperature=0, seed=42) und läuft über den ConsolidationScheduler.

### 5.5 Structured Claims Extraction

Jeder Consolidation-Durchlauf extrahiert strukturierte Claims aus den Summaries und erstellt eigenständige `MemoryType::Decision`-Nodes:

| Feature | Wert |
|---|---|
| **Methode** | Ollama JSON Schema (GBNF-Grammatik, 100% Format-Compliance) |
| **Prompt** | Evidence-First — erfordert konkrete Belege (Zahlen, Benchmarks, Vergleiche) |
| **Modell** | qwen2.5:3b (92.1% instruction-following, beste 3B-Klasse) |
| **Coverage** | 92.6% der Child-Decision-Nodes haben strukturierte Claims |
| **Spezifität** | ∅4.3/5 (Spike: 5 Testfälle, Evidence-First vs Baseline) |
| **Retrieval-Boost** | 1.77× Scoring-Multiplier (TRUST_PRIMARY + memory_type ×1.5) |
| **Claim-Format** | `decision_what` + `decision_why` in Metadata, optimiert für "Warum?"-Queries |

Claims werden als separate Decision-Nodes mit eigenem Embedding gespeichert und per `parent_tier_id` mit der L1-Overview verkettet.

### 5.6 Retrieval-Ansatz

1. **USearch Vector Search** — semantische Nähe via cosine similarity
2. **BM25 Keyword Search** — exakte Begriffs-Matches
3. **Reciprocal Rank Fusion** — Zusammenführung beider Ergebnislisten
4. **Fractal Zoom** — hierarchisches Zoomen: L2-Match → expandiert zu L1-Kindern → L0-Enkeln
5. **Profilbasierte Gewichtung** — Trust Tier × Retrieval Profile
6. **Score-Debugging** — optionales Debug für Operatoren

### 5.7 Energy Decay (Ebbinghaus)

Memories verlieren mit der Zeit an Energie. Der Decay folgt der Ebbinghaus-Vergessenskurve:
- `/energy/decay` — wendet Decay auf alle Memories an
- `/energy/low` — listet Memories mit niedriger Energie
- `/energy/compress` — komprimiert low-energy Cluster
- `/memories/{id}/energy/boost` — boostet einzelne Memory (z.B. nach Zugriff)

### 5.8 Self-Healing

- Orphaned Nodes: Knoten ohne gültigen Parent → re-parented oder archiviert
- Broken Links: Pointer ins Nichts → cleaned up
- Embedding Drift: Embedding passt nicht mehr zum Content → re-embeddable
- `/self-healing/stats` — Status-Übersicht
- `/memories/{id}/health` — Health-Check für einzelnen Knoten
- `/memories/{id}/reindex` — Neu-Indizierung externer Knoten

### 5.9 Auth

| Modus | Beschreibung |
|-------|-------------|
| Static Admin Key | `KNOWWHERE_API_KEY` als Bearer-Token, volle Rechte |
| Self-Service User | `/register`, `/login`, `/refresh` mit PostgreSQL |
| Capability Endpoint | `GET /auth/me` liefert Token-Art + erlaubte Profile |

### 5.10 Retrieval-Profile

| Profil | Zugriff | Beschreibung |
|--------|---------|-------------|
| `user-facing` | Admin + User | Sichere, konsumierbare Ergebnisse, blendet Interne aus |
| `agent-debug` | Nur Admin | Debug-Sicht mit Score-Einblicken |
| `full-fidelity` | Nur Admin | Rohe, maximale Sicht ohne Filterung |

## 6. Datenmodell

```rust
pub struct FractalNode {
    pub id: Uuid,
    pub memory_type: MemoryType,          // 6-Typen-System
    pub source: MemorySource,              // conversation/document/import/manual/consolidation
    pub embedding: Vec<f32>,                  // Embedding (768-dim nomic-embed-text-v2-moe)
    pub content: Option<String>,           // Session: Volltext. External: None
    pub original_pointer: Option<String>,  // External: URI/Pfad. Session: None
    pub metadata: HashMap<String, Value>,  // role, derivation, trust_tier, ...
    pub confidence: f64,                   // 0.0–1.0, typ-spezifischer Default
    pub sensitivity: Sensitivity,          // normal/low/high/restricted
    pub status: MemoryStatus,              // active/draft/archived/deleted/superseded/stale
    pub importance: i32,                   // 1–10
    pub conflict_state: ConflictState,     // none/pending/resolved
    pub superseded_by: Option<Uuid>,
    pub provenance: Value,
    pub access_count: i32,
    pub context_tier: ContextTier,         // raw(L0)/overview(L1)/summary(L2)
    pub parent_tier_id: Option<Uuid>,
    pub children_tier_ids: Vec<Uuid>,
    pub summary_content: Option<String>,   // L0 one-sentence summary
    pub overview_content: Option<String>,  // L1 paragraph overview
    pub weight: f64,
    pub multimodal: Option<MultimodalData>,// Image/Audio/Sensor
    pub children: Vec<FractalNode>,
    pub relations: Vec<Relation>,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}
```

## 7. Tech-Stack

| Komponente | Technologie |
|-----------|-------------|
| Backend | Rust 1.85+, Axum 0.8, Tokio, Tower |
| Embeddings | Ollama (nomic-embed-text-v2-moe, 768-dim, MoE, multilingual) |
| Retrieval | USearch 2.23 + BM25 2.3.2 + RRF |
| Summarization | Ollama qwen2.5:3b (lokal, JSON Schema GBNF-constrained, 92.1% instruction-following) |
| VLM Fallback | GPT-5-nano → GPT-4o-mini → Grok-4-fast |
| Persistenz default | MemoryStore (JSON) |
| Persistenz erweitert | PostgreSQL/pgvector |
| Auth | bcrypt, Blake3, API Keys + JWT |
| API-Doku | utoipa + Swagger UI |
| Dashboard | React + Vite |
| CI | GitHub Actions |

## 8. Integrationen

- **OpenClaw Plugin:** 6 Hooks — before_prompt_build, message_received, gateway_start, before_reset, session_end, agent_end
- **Python SDK:** `sdk/python`
- **Frigate NVR:** Polling-Connector + Webhook (Phase 1)

## 9. Nicht-Ziele

- Multi-Tenant-SaaS-Plattform
- Automatische Migration zwischen Storage-Backends
- Hot-Swap zwischen Embedding-Providern
- Automatisches Hard-Delete von Memories

## 10. Roadmap

### v1.0 (abgeschlossen in v0.4.0)
- PostgreSQL-Backend stabil → ✅ 41/41 Integration-Tests
- Docker Compose mit allen Features → ✅ (Ollama via host.docker.internal)
- OpenClaw Plugin E2E im Docker → ✅ verifiziert
- ConsolidationScheduler mit Space-Amplification Trigger → ✅ (statt stumpfem 60-Min-Timer)
- Cross-Encoder Reranking → ✅ (bge-reranker-v2-m3 via ONNX Runtime, POST /rerank)
- Streaming JSON Parser für 500-Case Benchmark → ✅ (RAM: 1-2GB → 10-50MB)
- Google Drive Connector → ✅ (Changes API, OAuth2, hinter google-drive Feature-Flag)
- HomeAssistant Webhook → ✅ (DedupCache + Secret-Validierung)
- Docs auf aktuellem Stand → ✅

### v1.1
- Entity-Graph für semantische Verbindungen zwischen Knoten
- Auto-Consolidation Scheduler (vollständig autonom, kein manueller Trigger nötig)

### Phase 2 (abgeschlossen in v0.4.0)
- ~~HomeAssistant Webhook~~ ✅ Done v0.4.0
- ~~Google Drive Connector~~ ✅ Done v0.4.0
- ~~Cross-Modal Embedding~~ ✅ Done v0.4.0

## 11. Integrationsregeln

1. Keine bestehenden Memories löschen oder überschreiben
2. Vorhandenes Wissen zuerst importieren
3. Host-Konfiguration nur ergänzen, nie ersetzen
4. Host-Memory-System parallel weiterlaufen lassen
5. Bei Ausfall von KnowWhere: Host muss degradiert weiter funktionieren

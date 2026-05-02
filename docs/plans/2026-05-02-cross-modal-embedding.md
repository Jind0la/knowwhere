# Cross-Modal Embedding Phase 2 — Implementation Plan

> **For Hermes:** Use subagent-driven-development via Kanban to implement this plan task-by-task.

**Goal:** Replace `PlaceholderCrossModalEmbedder` with real cross-modal embedding that puts images, audio, and sensor data into the same vector space as text — enabling true multimodal retrieval.

**Architecture:** EmbeddingRouter dispatches by content-type. Images via CLIP (Ollama), audio via Whisper (Ollama), sensor data via text serialization. All embeddings go into the existing USearch index at 768 dimensions (same as nomic-embed-text). Backward compatible — text-only flow unchanged.

**Tech Stack:** Rust, Axum, Ollama API (CLIP/Whisper), reqwest, USearch, ONNX Runtime (fallback)

---

## Task 1: CLIP Image Embedder

**Objective:** Implement `ClipProvider` that calls Ollama's CLIP model and returns 768-dim embeddings.

**Files:**
- Create: `src/embedding/clip.rs`
- Modify: `src/embedding/mod.rs` (register provider)

**Step 1:** Create `ClipProvider` struct with Ollama client
- Base URL from `OLLAMA_URL` env (same as text embedding)
- Model: `clip-vit-large` or env-overridable via `OLLAMA_CLIP_MODEL`

**Step 2:** Implement `embed_image(&self, image_bytes: &[u8]) -> Result<Vec<f32>>`
- Send base64-encoded image to Ollama `/api/embeddings` with CLIP model
- Parse response, return 768-dim vector

**Step 3:** Add `ClipProvider` to `ProviderKind` enum and `create_provider()`

**Step 4:** Unit test with a mock Ollama response

**Verification:** `cargo test --lib clip`

---

## Task 2: Audio Embedder (Whisper/CLAP)

**Objective:** Implement `AudioProvider` that transcribes audio via Whisper and embeds the transcript.

**Files:**
- Create: `src/embedding/audio.rs`
- Modify: `src/embedding/mod.rs`

**Approach:** Two-step pipeline
1. Whisper STT: audio bytes → text transcript
2. Text embedding: transcript → vector via `EmbeddingProvider`

**Step 1:** `transcribe(&self, audio_bytes: &[u8]) -> Result<String>`
- Call Ollama `/api/generate` with whisper model
- Return transcribed text

**Step 2:** `embed_audio(&self, audio_bytes: &[u8], text_provider: &dyn EmbeddingProvider) -> Result<Vec<f32>>`
- Chain: transcribe → embed_document → vector

**Step 3:** Register in `ProviderKind`

**Verification:** `cargo test --lib audio` (mock Whisper response)

---

## Task 3: Sensor Embedder

**Objective:** Convert sensor JSON payloads to text descriptions and embed.

**Files:**
- Create: `src/embedding/sensor.rs`

**Step 1:** `sensor_to_text(data: &Value) -> String`
- Serialize key fields into a descriptive sentence
- Example: `{"temperature": 22.5, "humidity": 60}` → "Temperature 22.5°C, humidity 60%"

**Step 2:** `embed_sensor(&self, data: &Value, text_provider: &dyn EmbeddingProvider) -> Result<Vec<f32>>`
- Convert to text → embed_document → vector

**Verification:** `cargo test --lib sensor`

---

## Task 4: EmbeddingRouter

**Objective:** Build the dispatcher that routes content to the right embedder.

**Files:**
- Create: `src/embedding/router.rs`
- Replace: `src/multimodal.rs` (real impl instead of PlaceholderCrossModalEmbedder)

**Content-Type detection:**

| Input | Content-Type | Embedder |
|-------|-------------|----------|
| `text/*` | Text | TextEmbeddingProvider (existing) |
| `image/*` | Image | ClipProvider |
| `audio/*` | Audio | AudioProvider |
| `application/json` + sensor metadata | Sensor | SensorEmbedder |

**Step 1:** Define `EmbeddingRouter` struct holding `Arc<dyn EmbeddingProvider>` + `Arc<ClipProvider>` + `Arc<AudioProvider>`

**Step 2:** Implement `route(&self, content_type: &str, payload: &[u8]) -> Result<Vec<f32>>`
- Match content_type → delegate to correct embedder
- All outputs must be 768-dim

**Step 3:** Implement `CrossModalEmbedder` trait using `EmbeddingRouter`
- Replace `PlaceholderCrossModalEmbedder` in `multimodal.rs`

**Step 4:** Integration test: store image → retrieve by text query

**Verification:** `cargo test --lib router`

---

## Task 5: Content-Type Detection in POST /store

**Objective:** Wire the EmbeddingRouter into the store endpoint.

**Files:**
- Modify: `src/api/routes.rs` (or relevant store handler)
- Modify: `src/services/` (if store logic lives there)

**Step 1:** Parse `Content-Type` header from incoming request

**Step 2:** Route through `EmbeddingRouter` instead of always using text embedding

**Step 3:** Store resulting embedding in `FractalNode.vector`

**Step 4:** Backward compatibility: text/plain and no Content-Type → text embedding (unchanged)

**Verification:** `cargo test --lib store_multimodal`

---

## Task 6: Docker Compose Update

**Objective:** Add CLIP and Whisper models to Ollama configuration.

**Files:**
- Modify: `docker-compose.yml` (pre-pull models)

**Step 1:** Add `clip-vit-large` and `whisper-base` to the Ollama container's pre-pull list

**Step 2:** Verify models load in Docker environment

**Verification:** `curl http://host.docker.internal:11434/api/tags | grep -E "clip|whisper"`

---

## Task 7: Documentation

**Objective:** Update PRD and add multimodal docs.

**Files:**
- Modify: `docs/PRD.md`
- Create: `docs/multimodal-embedding.md`

**Step 1:** Add Cross-Modal Embedding section to PRD
**Step 2:** Document content-type → embedder mapping
**Step 3:** Document Ollama model requirements
**Step 4:** Add example API calls

---

## Risk: CLIP on M3 Performance

**Mitigation:** If Ollama CLIP is too slow on M3, fall back to ONNX Runtime (already used by Cross-Encoder Reranker). CLIP ONNX models are ~600MB.

## Definition of Done

- [ ] Image stored via `POST /store` with `Content-Type: image/png` retrievable by text query
- [ ] Audio stored retrievable by text query  
- [ ] All 93 existing tests still green
- [ ] New tests for ClipProvider, AudioProvider, SensorEmbedder, EmbeddingRouter
- [ ] Docker Compose clean start with CLIP/Whisper models
- [ ] Docs updated

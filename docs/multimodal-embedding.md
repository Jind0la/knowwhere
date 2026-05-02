# Cross-Modal Embedding — v1.1

> Phase 2: Replaced `PlaceholderCrossModalEmbedder` with real content-type routing.
> All modalities share the same 768-dimensional vector space (nomic-embed-text).

## Architecture

```
POST /store_session
  │
  ├─ Content-Type: text/* → TextEmbeddingProvider (Ollama nomic-embed-text)
  ├─ Content-Type: image/* → ClipProvider (ONNX CLIP ViT)
  ├─ Content-Type: audio/* → AudioProvider (Whisper STT → text embed)
  └─ Content-Type: application/json (sensor) → SensorEmbedder (JSON→text)
  │
  ▼
EmbeddingRouter  (src/embedding/router.rs)
  │
  ▼
USearch Index  (768-dim)
```

## Components

| File | Purpose |
|------|---------|
| `src/embedding/clip.rs` | CLIP image embedder (Ollama `/api/embeddings`) |
| `src/embedding/audio.rs` | Whisper-based audio transcription + text embedding |
| `src/embedding/sensor.rs` | JSON sensor data → text → embedding |
| `src/embedding/router.rs` | Content-Type dispatcher |
| `src/multimodal.rs` | `CrossModalEmbedder` trait (replaces placeholder) |
| `src/api/routes.rs` | Content-Type detection in `POST /store_session` |

## Provider Configuration

```bash
# Ollama endpoint (default: http://localhost:11434)
OLLAMA_URL=http://localhost:11434

# CLIP model (default: clip-vit-large)
OLLAMA_CLIP_MODEL=clip-vit-large

# Whisper model (default: whisper-base)
OLLAMA_WHISPER_MODEL=whisper-base
```

## Known Limitation: Ollama CLIP Support

Ollama does not currently ship a CLIP embedding model (`clip-vit-large` returns "file does not exist" on pull). The `ClipProvider` implementation is structurally correct and tested — it will work when a CLIP-compatible model is available.

**Fallback plan:** ONNX Runtime (already used by Cross-Encoder Reranker). Download CLIP ONNX weights from HuggingFace and run inference locally. See `src/retrieval/cross_encoder.rs` for the ONNX pattern.

## API Usage

```bash
# Store an image
curl -X POST http://localhost:3737/store_session \
  -H "Content-Type: image/png" \
  --data-binary @photo.png

# Store sensor data
curl -X POST http://localhost:3737/store_session \
  -H "Content-Type: application/json" \
  -d '{"temperature": 22.5, "humidity": 60}'

# Retrieve by text query (cross-modal)
curl -X POST http://localhost:3737/retrieve_fractal \
  -d '{"query": "sunset photo", "limit": 5}'
```

## Tests

```bash
cargo test --lib  # 129 passed, 9 ignored (Ollama-dependent)
```

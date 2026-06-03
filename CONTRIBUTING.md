# Contributing to KnowWhere

Thank you for your interest in contributing! KnowWhere is a **lossless fractal memory system for AI agents** — pointer-first, with fractal zoom and 0% information loss.

## Quick Start

```bash
git clone https://github.com/Jind0la/knowwhere.git
cd knowwhere
cargo build
cargo test --lib
```

**Prerequisites:** Rust 1.85+, Ollama (for embeddings), PostgreSQL 16+ (optional, for persistent storage).

## Development Workflow

1. **Find something to work on** — Check [GitHub Issues](https://github.com/Jind0la/knowwhere/issues) or the [NEXT_STEPS.md](docs/NEXT_STEPS.md) roadmap.
2. **Create a branch** — `git checkout -b feat/your-feature` or `fix/your-bugfix`
3. **Write tests** — All new features need tests. We target >80% coverage on new code.
4. **Run the full CI suite locally:**
   ```bash
   cargo check --all-features
   cargo clippy --all-features -- -D warnings
   cargo test --lib
   cargo fmt -- --check
   ```
5. **Open a PR** — Describe what you changed and why. Link to any related issues.

## Architecture

KnowWhere v0.6.0 is built on:
- **Axum 0.8** web framework with 14 API submodules (see `src/api/`)
- **nomic-embed-text** (768-dim) for embeddings via Ollama
- **gte-modernbert** ONNX cross-encoder for reranking
- **Turn-Level Storage** — every conversation turn gets its own embedding
- **Hybrid BM25 + Dense Retrieval** with RRF fusion
- **Source-Type Weighting** with provenance tracking

Read [`ARCHITECTURE_MAP.md`](ARCHITECTURE_MAP.md) for the full module diagram and where-to-find-what table.

## Code Style

- **Edition 2021** Rust
- `cargo fmt` before committing
- `cargo clippy --all-features` must pass with 0 warnings
- Use `.expect("...")` with descriptive messages, never bare `.unwrap()` in production code
- All `unsafe` blocks must have `// SAFETY:` comments
- Module-level doc comments (`//!`) on every file

## Testing

```bash
# Unit tests (305 tests, <1s)
cargo test --lib

# With all features
cargo test --all-features

# Specific module
cargo test --lib -- source_weighting
```

Tests that manipulate environment variables use a shared `ENV_LOCK` mutex to prevent race conditions. If you add env-var-dependent tests, acquire the lock first:
```rust
let _lock = ENV_LOCK.lock().unwrap();
```

## Documentation

- **User-facing docs** live in `docs/` — keep them current with code changes
- **Architecture Decision Records** in `docs/adr/` — write one for any non-obvious design choice
- **Plans** go in `docs/plans/` before implementation
- **Archived/obsolete docs** go in `docs/archive/` — never delete historical analysis
- **Rust doc comments** (`///`) on all public APIs

## Commit Messages

Follow the format:
```
type: short description

Longer explanation if needed. What, why, and verification.

Examples:
- fix: eliminate production unwrap()s in storage layer
- feat: add cross-encoder reranking with ONNX
- refactor: split routes.rs into 14 submodules
```

## Questions?

Open a [GitHub Discussion](https://github.com/Jind0la/knowwhere/discussions) or file an issue. For architecture questions, see [`docs/ADR_INDEX.md`](docs/ADR_INDEX.md).

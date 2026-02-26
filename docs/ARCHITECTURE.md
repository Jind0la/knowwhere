# KnowWhere Architecture

## High-Level
- Memory Service (Rust Axum)
- FractalNode struct (content OR original_pointer)
- Client SDKs (Python zuerst)

## Datenstruktur (Rust)
struct FractalNode { ... } (genau wie in unserem Bauplan)

## Ordnerstruktur (zu erstellen)
knowwhere/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── api/
│   ├── memory/
│   ├── embedding/
│   └── storage/
├── docs/
│   ├── PRD.md
│   └── ARCHITECTURE.md
├── .cursor/rules/knowwhere.mdc
└── sdk/python/
//! Tests for Soul.md specification compliance
//!
//! Validates that KnowWhere's implementation matches the contract
//! defined in Soul.md — identity, API conventions, memory architecture,
//! and OpenClaw integration.

use std::path::Path;

/// Section presence in Soul.md
#[test]
fn soul_md_exists() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    assert!(path.exists(), "Soul.md must exist at project root");

    let content = std::fs::read_to_string(&path).unwrap();

    // Core sections must exist
    assert!(
        content.contains("# KnowWhere Soul"),
        "Must have Soul header"
    );
    assert!(
        content.contains("## Identity"),
        "Must have Identity section"
    );
    assert!(
        content.contains("## Core Personality"),
        "Must have Personality section"
    );
    assert!(
        content.contains("## Memory Architecture"),
        "Must have Memory Architecture"
    );
    assert!(
        content.contains("## API Conventions"),
        "Must have API Conventions"
    );
    assert!(
        content.contains("## OpenClaw Integration"),
        "Must have OpenClaw section"
    );
    assert!(
        content.contains("## Configuration"),
        "Must have Configuration section"
    );
    assert!(
        content.contains("## Known Limitations"),
        "Must have Known Limitations"
    );
    assert!(content.contains("## Schemas"), "Must have Schemas section");
    assert!(
        content.contains("## Testing Strategy"),
        "Must have Testing Strategy"
    );
}

/// Soul.md must document Pointer-First as core identity
#[test]
fn soul_documents_pointer_first() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("Pointer-First"),
        "Soul.md must document Pointer-First architecture"
    );
    assert!(content.contains("pointer"), "Soul.md must mention pointers");
}

/// Soul.md must document Fractal Memory tiers
#[test]
fn soul_documents_fractal_tiers() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(content.contains("L0"), "Must document L0 tier");
    assert!(content.contains("L1"), "Must document L1 tier");
    assert!(content.contains("L2"), "Must document L2 tier");
    assert!(
        content.contains("Consolidation"),
        "Must document consolidation"
    );
    assert!(content.contains("VLM"), "Must mention VLM for L2→L1");
}

/// Soul.md must document the actual API endpoints (not aspirational ones)
#[test]
fn soul_documents_api_endpoints() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    // Real protected endpoints (from routes.rs + main.rs)
    assert!(
        content.contains("/store_session"),
        "Must document /store_session endpoint"
    );
    assert!(
        content.contains("/store_external"),
        "Must document /store_external endpoint"
    );
    assert!(
        content.contains("/retrieve_fractal"),
        "Must document /retrieve_fractal endpoint"
    );
    assert!(content.contains("/embed"), "Must document /embed endpoint");
    assert!(
        content.contains("/dream/status"),
        "Must document /dream/status endpoint"
    );

    // Real auth endpoints
    assert!(
        content.contains("/register"),
        "Must document /register endpoint"
    );
    assert!(content.contains("/login"), "Must document /login endpoint");
    assert!(
        content.contains("/refresh"),
        "Must document /refresh endpoint"
    );

    // Public
    assert!(
        content.contains("/health"),
        "Must document /health endpoint"
    );
}

/// Soul.md must document the OpenClaw hooks that are actually implemented
#[test]
fn soul_documents_openclaw_hooks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    // The 3 real hooks from openclaw-plugin/src/index.ts
    assert!(
        content.contains("before_prompt_build"),
        "Must document before_prompt_build hook"
    );
    assert!(
        content.contains("agent_end"),
        "Must document agent_end hook"
    );
    assert!(
        content.contains("before_compaction"),
        "Must document before_compaction hook"
    );
}

/// Soul.md must document required environment variables
#[test]
fn soul_documents_env_vars() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    let required_vars = [
        "DATABASE_URL",
        "OLLAMA_URL",
        "OLLAMA_MODEL",
        "DREAM_ENABLED",
        "CONSOLIDATION_INTERVAL_SECS",
        "AUDIT_INTERVAL_SECS",
    ];

    for var in required_vars {
        assert!(
            content.contains(var),
            "Soul.md must document env var: {var}"
        );
    }
}

/// Soul.md must document L2 consolidation dependency on API key
#[test]
fn soul_documents_l2_key_dependency() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("OPENAI_API_KEY"),
        "Soul.md must mention OPENAI_API_KEY dependency"
    );
    assert!(
        content.contains("VLM") || content.contains("GROK_API_KEY"),
        "Soul.md must explain VLM requirement for L2"
    );
}

/// Soul.md must link to BUG-TRACKING.md
#[test]
fn soul_links_to_bug_tracking() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("BUG-TRACKING.md"),
        "Soul.md must reference BUG-TRACKING.md"
    );
}

/// Soul.md must document tier TTL defaults
#[test]
fn soul_documents_tier_ttls() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(content.contains("L0_TTL"), "Must document L0_TTL");
    assert!(content.contains("L1_TTL"), "Must document L1_TTL");
    assert!(content.contains("L2_TTL"), "Must document L2_TTL");
}

/// Soul.md must document PostgreSQL schema
#[test]
fn soul_documents_postgres_schema() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("memory_items"),
        "Must document memory_items table"
    );
    assert!(
        content.contains("namespaces"),
        "Must document namespaces table"
    );
    assert!(content.contains("api_keys"), "Must document api_keys table");
    assert!(
        content.contains("auth_users"),
        "Must document auth_users table"
    );
}

/// Soul.md must document test command
#[test]
fn soul_documents_test_command() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Soul.md");
    let content = std::fs::read_to_string(&path).unwrap();

    assert!(
        content.contains("cargo test"),
        "Soul.md must document how to run tests"
    );
    assert!(
        content.contains("docker-compose"),
        "Soul.md must mention docker-compose for tests"
    );
}

/// OpenClaw plugin directory must exist if Soul.md says it exists
#[test]
fn openclaw_plugin_directory_exists() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("openclaw-plugin");
    assert!(
        path.exists() || std::env::var("SKIP_OPENCLAW_PLUGIN_CHECK").is_ok(),
        "openclaw-plugin/ directory should exist per Soul.md"
    );
}

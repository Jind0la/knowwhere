use knowwhere_server::api::docs::ApiDoc;
use utoipa::OpenApi;

fn openapi_paths() -> std::collections::HashSet<String> {
    let doc = ApiDoc::openapi();
    let value = serde_json::to_value(doc).expect("openapi must serialize");
    let paths = value["paths"]
        .as_object()
        .expect("openapi paths must be an object");
    paths.keys().cloned().collect()
}

fn assert_has_paths(paths: &std::collections::HashSet<String>, expected: &[&str]) {
    for path in expected {
        assert!(
            paths.contains(*path),
            "OpenAPI contract missing required beta path: {}",
            path
        );
    }
}

#[test]
fn openapi_contains_beta_core_routes() {
    let paths = openapi_paths();
    let expected = [
        "/health",
        "/embed",
        "/store_session",
        "/store_external",
        "/retrieve/{id}",
        "/retrieve_fractal",
        "/nodes/{id}",
        "/nodes/purge_dummy",
        "/nodes/recent",
        "/nodes/reembed_all",
        "/dream/status",
        "/events",
        "/governance/policy",
        "/webhooks/frigate",
    ];

    assert_has_paths(&paths, &expected);
}

#[cfg(feature = "postgres-storage")]
#[test]
fn openapi_contains_beta_postgres_routes() {
    let paths = openapi_paths();
    let expected = [
        "/retrieval/runs",
        "/retrieval/runs/{id}",
        "/retrieval/runs/{id}/trajectory",
        "/energy/decay",
        "/energy/low",
        "/energy/compress",
        "/deduplication/candidates",
        "/deduplication/run",
        "/deduplication/runs",
        "/memories/{id}/reindex",
        "/memories/{id}/health",
        "/self-healing/stats",
        "/namespaces",
        "/namespaces/{path}",
        "/namespaces/{path}/memories",
        "/namespaces/{path}/search",
        "/skills",
        "/skills/{id}",
        "/skills/{id}/use",
        "/skills/match",
    ];

    assert_has_paths(&paths, &expected);
}

use std::collections::HashMap;
use std::time::Instant;

use knowwhere_server::memory::types::{MemorySource, MemoryType};
use knowwhere_server::memory::FractalNode;
use knowwhere_server::storage::{HybridQuery, MemoryStore, RetrievalProfile, StorageBackend};
use uuid::Uuid;

const DIM: usize = 8;
const TOP_K: usize = 5;

#[derive(Clone, Copy)]
struct EchoCase {
    topic: usize,
    memory: &'static str,
    queries: &'static [&'static str],
}

#[derive(Debug)]
struct EchoMetrics {
    precision_at_1: f64,
    recall_at_3: f64,
    mrr: f64,
    semantically_robust: f64,
}

const ECHO_CASES: &[EchoCase] = &[
    EchoCase {
        topic: 0,
        memory: "KnowWhere folgt einer Pointer-First Architektur fuer externe Daten.",
        queries: &[
            "Was bedeutet Pointer-First bei KnowWhere?",
            "external data pointer architecture",
            "Wie speichert KnowWhere externe Dateien?",
        ],
    },
    EchoCase {
        topic: 1,
        memory: "Das API Backend laeuft standardmaessig auf Port 3737.",
        queries: &[
            "Welcher Port ist der Standard fuer die API?",
            "default knowwhere api port",
            "Port 3737",
        ],
    },
    EchoCase {
        topic: 2,
        memory: "Retrieve-Fractal kombiniert Vector Search, BM25 und RRF.",
        queries: &[
            "Wie werden Retrieval Ergebnisse fusioniert?",
            "vector bm25 rrf",
            "was macht retrieve_fractal?",
        ],
    },
    EchoCase {
        topic: 3,
        memory: "GET /auth/me liefert token_kind und allowed_retrieval_profiles.",
        queries: &[
            "Welche Capabilities liefert auth me?",
            "auth me retrieval profiles",
            "token_kind allowed_retrieval_profiles",
        ],
    },
    EchoCase {
        topic: 4,
        memory: "User Tokens duerfen aktuell nur das user-facing Retrieval-Profil verwenden.",
        queries: &[
            "Welche Retrieval Profile darf ein User Token?",
            "user-facing only token",
            "limited user retrieval profile",
        ],
    },
    EchoCase {
        topic: 5,
        memory: "Das aktive Operator-Frontend lebt im dashboard Verzeichnis auf Vite.",
        queries: &[
            "Wo liegt das aktive Dashboard Frontend?",
            "vite dashboard directory",
            "react operator ui path",
        ],
    },
    EchoCase {
        topic: 6,
        memory: "Mit postgres-storage und DATABASE_URL werden erweiterte Lifecycle Routen aktiv.",
        queries: &[
            "Wann sind Energy und Deduplication Routen aktiv?",
            "postgres storage lifecycle routes",
            "DATABASE_URL feature gate routes",
        ],
    },
    EchoCase {
        topic: 7,
        memory: "OLLAMA_EMBEDDING_DIMENSION erlaubt eine explizite Override-Dimension.",
        queries: &[
            "Wie setze ich die Embedding Dimension fuer Ollama?",
            "OLLAMA_EMBEDDING_DIMENSION",
            "explicit embedding dimension override",
        ],
    },
];

fn topic_vector(topic: usize) -> Vec<f32> {
    let mut v = vec![0.02; DIM];
    v[topic % DIM] = 1.0;
    v[(topic + 1) % DIM] = 0.12;
    v
}

fn query_vector(topic: usize, variant: usize) -> Vec<f32> {
    let mut v = topic_vector(topic);
    let drift = ((variant % 3) as f32) * 0.01;
    v[topic % DIM] -= drift;
    v[(topic + 2) % DIM] += drift;
    v
}

fn build_node(content: &str, vector: Vec<f32>) -> FractalNode {
    FractalNode::new_typed(
        Some(content.to_string()),
        None,
        vector,
        HashMap::new(),
        MemoryType::Semantic,
        MemorySource::Conversation,
    )
}

async fn insert_echo_memories(store: &MemoryStore) -> anyhow::Result<Vec<Uuid>> {
    let mut ids = Vec::with_capacity(ECHO_CASES.len());
    for case in ECHO_CASES {
        let id = store
            .insert(build_node(case.memory, topic_vector(case.topic)))
            .await?;
        ids.push(id);
    }
    Ok(ids)
}

fn reciprocal_rank(rank: Option<usize>) -> f64 {
    rank.map(|r| 1.0 / (r as f64)).unwrap_or(0.0)
}

fn calc_metrics(ranks: &[Option<usize>]) -> EchoMetrics {
    let total = ranks.len() as f64;
    let p1 = ranks.iter().filter(|r| matches!(r, Some(1))).count() as f64;
    let r3 = ranks
        .iter()
        .filter(|r| r.map(|v| v <= 3).unwrap_or(false))
        .count() as f64;
    let robust = ranks.iter().filter(|r| r.is_some()).count() as f64;
    let mrr = ranks.iter().map(|r| reciprocal_rank(*r)).sum::<f64>() / total;
    EchoMetrics {
        precision_at_1: p1 / total,
        recall_at_3: r3 / total,
        mrr,
        semantically_robust: robust / total,
    }
}

fn find_rank(results: &[knowwhere_server::storage::ScoredNode], target: Uuid) -> Option<usize> {
    results.iter().position(|r| r.id == target).map(|idx| idx + 1)
}

#[tokio::test]
async fn echo_retrieval_quality_baseline() -> anyhow::Result<()> {
    let started = Instant::now();
    let store = MemoryStore::new();
    let ids = insert_echo_memories(&store).await?;
    let mut ranks = Vec::new();

    for case in ECHO_CASES {
        for (variant_idx, query_text) in case.queries.iter().enumerate() {
            let query = HybridQuery {
                query_text: Some((*query_text).to_string()),
                query_vector: Some(query_vector(case.topic, variant_idx)),
                top_k: TOP_K,
                max_depth: 0,
                profile: RetrievalProfile::FullFidelity,
            };
            let results = StorageBackend::hybrid_retrieve(&store, &query).await?;
            ranks.push(find_rank(&results, ids[case.topic]));
        }
    }

    let metrics = calc_metrics(&ranks);
    let elapsed_ms = started.elapsed().as_millis();
    println!("echo_retrieval_quality metrics={metrics:?} elapsed_ms={elapsed_ms}");

    assert!(
        metrics.precision_at_1 >= 0.70,
        "Precision@1 too low: {:.2}",
        metrics.precision_at_1
    );
    assert!(metrics.recall_at_3 >= 0.85, "Recall@3 too low: {:.2}", metrics.recall_at_3);
    assert!(metrics.mrr >= 0.75, "MRR too low: {:.2}", metrics.mrr);
    assert!(
        metrics.semantically_robust >= 0.80,
        "Semantic robustness too low: {:.2}",
        metrics.semantically_robust
    );

    Ok(())
}

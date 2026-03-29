#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use uuid::Uuid;

    use crate::embedding::provider::EmbeddingProvider;
    use crate::embedding::{LocalOllamaProvider, create_provider, ProviderKind};
    use crate::memory::fractal_node::cosine_similarity;
    use crate::memory::FractalNode;
    use crate::multimodal::MultimodalData;
    use crate::storage::MemoryStore;

    #[test]
    fn new_session_has_content_no_pointer() {
        let node = FractalNode::new_session(
            "hello session".to_string(),
            vec![0.1, 0.2, 0.3],
            HashMap::new(),
        );
        assert!(node.content.is_some());
        assert_eq!(node.content.as_deref(), Some("hello session"));
        assert!(node.original_pointer.is_none());
    }

    #[test]
    fn new_external_has_pointer_no_content() {
        let node = FractalNode::new_external(
            "s3://bucket/photo.jpg".to_string(),
            vec![0.4, 0.5, 0.6],
            HashMap::new(),
        );
        assert!(node.original_pointer.is_some());
        assert_eq!(
            node.original_pointer.as_deref(),
            Some("s3://bucket/photo.jpg")
        );
        assert!(node.content.is_none());
    }

    #[tokio::test]
    async fn store_session_and_external_via_memory_store() {
        let store = MemoryStore::new();

        let session = FractalNode::new_session(
            "stored session".to_string(),
            vec![0.1, 0.2],
            HashMap::new(),
        );
        let external = FractalNode::new_external(
            "gdrive://doc/123".to_string(),
            vec![0.3, 0.4],
            HashMap::new(),
        );

        store.insert(session).await.expect("insert session");
        store.insert(external).await.expect("insert external");

        assert_eq!(store.count().await, 2);
    }

    #[tokio::test]
    async fn retrieve_node_by_id() {
        let store = MemoryStore::new();

        let node = FractalNode::new_session("findme".to_string(), vec![1.0, 2.0], HashMap::new());
        let expected_id = node.id;

        store.insert(node).await.expect("insert node");

        let retrieved = store
            .get(&expected_id)
            .await
            .expect("get should not fail")
            .expect("node should exist");

        assert_eq!(retrieved.id, expected_id);
        assert_eq!(retrieved.content.as_deref(), Some("findme"));
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-5, "expected ~1.0, got {sim}");
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5, "expected ~0.0, got {sim}");
    }

    #[test]
    fn zoom_retrieve_with_children() {
        let mut parent = FractalNode::new_session(
            "parent".to_string(),
            vec![1.0, 0.0, 0.0],
            HashMap::new(),
        );
        let close_child = FractalNode::new_session(
            "close".to_string(),
            vec![0.9, 0.1, 0.0],
            HashMap::new(),
        );
        let far_child = FractalNode::new_session(
            "far".to_string(),
            vec![0.0, 0.0, 1.0],
            HashMap::new(),
        );
        parent.children = vec![close_child, far_child];

        let query = vec![1.0, 0.0, 0.0];
        let results = parent.zoom_retrieve(&query, 1, 0.7);

        assert_eq!(results.len(), 2, "parent + best child");
        assert_eq!(results[0].1.content.as_deref(), Some("parent"));
        assert_eq!(results[1].1.content.as_deref(), Some("close"));
    }

    #[tokio::test]
    async fn retrieve_fractal_top_k() {
        let store = MemoryStore::new();

        let n1 = FractalNode::new_session("alpha".into(), vec![1.0, 0.0], HashMap::new());
        let n2 = FractalNode::new_session("beta".into(), vec![0.9, 0.1], HashMap::new());
        let n3 = FractalNode::new_session("gamma".into(), vec![0.0, 1.0], HashMap::new());

        store.insert(n1).await.unwrap();
        store.insert(n2).await.unwrap();
        store.insert(n3).await.unwrap();

        let query = vec![1.0, 0.0];
        #[cfg(feature = "postgres-storage")]
        let results = store.retrieve_fractal(&query, 2, 0, 0.0, None).await;
        #[cfg(not(feature = "postgres-storage"))]
        let results = store.retrieve_fractal(&query, 2, 0).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content.as_deref(), Some("alpha"));
        assert_eq!(results[1].content.as_deref(), Some("beta"));
    }

    // -- Embedding Tests (Woche 2) --

    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY — run manually with: cargo test test_openai_embedding_generates_valid_vector -- --ignored"]
    async fn test_openai_embedding_generates_valid_vector() {
        let provider = create_provider(
            ProviderKind::OpenAI,
            Some(std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set")),
        );
        let vector = provider.embed("hello world").await.expect("embed failed");

        // OpenAI text-embedding-3-small: 1536 dimensions
        assert_eq!(
            vector.len(),
            1536,
            "OpenAI text-embedding-3-small has 1536 dimensions"
        );

        // Embeddings should be normalized (roughly)
        let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(norm > 0.0, "embedding should not be zero vector");
    }

    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY"]
    async fn test_store_session_auto_embed() {
        let provider = create_provider(
            ProviderKind::OpenAI,
            Some(std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set")),
        );
        let store = MemoryStore::new();

        let text = "This is a test session for auto-embedding";
        let vector = provider.embed(text).await.expect("embed failed");

        let node = FractalNode::new_session(text.to_string(), vector.clone(), HashMap::new());
        let id = store.insert(node).await.expect("insert failed");

        let retrieved = store
            .get(&id)
            .await
            .expect("get failed")
            .expect("node missing");

        assert!(!retrieved.vector.is_empty());
        assert_eq!(retrieved.vector.len(), provider.dimension());
        assert_eq!(retrieved.vector, vector);
    }

    #[tokio::test]
    async fn test_usearch_retrieve_consistency() {
        let store = MemoryStore::new();
        let dim = 8;

        for i in 0..60u32 {
            let mut vec = vec![0.0f32; dim];
            vec[(i as usize) % dim] = 1.0;
            // Add slight variation so vectors aren't exact duplicates
            vec[0] += (i as f32) * 0.001;

            let node = FractalNode::new_session(format!("node-{i}"), vec, HashMap::new());
            store.insert(node).await.unwrap();
        }

        assert!(store.count().await >= 50, "should exceed USearch threshold");

        let query = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        #[cfg(feature = "postgres-storage")]
        let results = store.retrieve_fractal(&query, 5, 0, 0.0, None).await;
        #[cfg(not(feature = "postgres-storage"))]
        let results = store.retrieve_fractal(&query, 5, 0).await;

        assert!(!results.is_empty(), "should return results");
        assert!(results.len() <= 5, "should respect top_k");

        let top = &results[0];
        assert!(
            top.vector[0] > 0.5,
            "top result should have high value in first dimension, got {}",
            top.vector[0]
        );
    }

    // -- SDK/API Integration Tests (Woche 4) --

    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY"]
    async fn test_sdk_store_session_retrieve_roundtrip() {
        let provider = create_provider(
            ProviderKind::OpenAI,
            Some(std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set")),
        );
        let store = MemoryStore::new();

        let content = "Die App soll anonym sein, kein Login nötig";
        let vector = provider.embed(content).await.expect("embed");
        let node = FractalNode::new_session(content.to_string(), vector, HashMap::from([
            ("project".to_string(), serde_json::json!("knowwhere")),
        ]));
        let id = node.id;
        store.insert(node).await.expect("insert");

        let retrieved = store.get(&id).await.unwrap().unwrap();
        assert_eq!(retrieved.content.as_deref(), Some(content));
        assert!(retrieved.original_pointer.is_none());
        assert!(!retrieved.vector.is_empty());
        assert_eq!(retrieved.metadata["project"], "knowwhere");
    }

    #[tokio::test]
    async fn test_sdk_store_external_multimodal_pointer_first() {
        let store = MemoryStore::new();
        let pointer = "frigate://camera/front/2026-02-26T20:15.jpg";
        let embedding = vec![0.1, 0.2, 0.3, 0.4];

        let mm = MultimodalData::Image {
            pointer: pointer.to_string(),
            embedding: embedding.clone(),
        };

        let node = FractalNode::new_external_multimodal(
            pointer.to_string(),
            embedding,
            HashMap::from([
                ("source".to_string(), serde_json::json!("frigate")),
                ("camera".to_string(), serde_json::json!("front_door")),
            ]),
            mm,
        );
        let id = node.id;

        assert!(node.content.is_none(), "pointer-first: never store raw content");

        store.insert(node).await.expect("insert");
        let retrieved = store.get(&id).await.unwrap().unwrap();

        assert!(retrieved.content.is_none(), "pointer-first: content must be None");
        assert_eq!(retrieved.original_pointer.as_deref(), Some(pointer));
        assert!(retrieved.multimodal.is_some());
        assert_eq!(retrieved.metadata["source"], "frigate");
    }

    #[tokio::test]
    async fn test_recent_nodes_ordering() {
        let store = MemoryStore::new();
        for i in 0..5u32 {
            let node = FractalNode::new_session(
                format!("node-{i}"),
                vec![i as f32; 4],
                HashMap::new(),
            );
            store.insert(node).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let recent = store.recent(3).await;
        assert_eq!(recent.len(), 3);
        assert!(recent[0].created_at >= recent[1].created_at);
        assert!(recent[1].created_at >= recent[2].created_at);
    }

    // -- Multimodal Tests (Woche 3) --

    #[tokio::test]
    async fn test_multimodal_image_node() {
        let store = MemoryStore::new();
        let embedding = vec![0.5, 0.3, 0.1, 0.8];

        let mm = MultimodalData::Image {
            pointer: "frigate://camera/front/snapshot/001".to_string(),
            embedding: embedding.clone(),
        };

        let node = FractalNode::new_external_multimodal(
            "frigate://camera/front/snapshot/001".to_string(),
            embedding,
            HashMap::from([
                ("source".to_string(), serde_json::json!("frigate")),
                ("camera".to_string(), serde_json::json!("front_door")),
            ]),
            mm,
        );
        let id = node.id;

        assert!(node.content.is_none(), "pointer-first: no raw content");
        assert!(node.original_pointer.is_some());
        assert!(node.multimodal.is_some());

        store.insert(node).await.expect("insert image node");

        let retrieved = store.get(&id).await.unwrap().unwrap();
        assert!(retrieved.multimodal.is_some());
        assert!(retrieved.content.is_none());
        assert_eq!(
            retrieved.original_pointer.as_deref(),
            Some("frigate://camera/front/snapshot/001")
        );

        if let Some(MultimodalData::Image { pointer, .. }) = &retrieved.multimodal {
            assert!(pointer.starts_with("frigate://"));
        } else {
            panic!("expected MultimodalData::Image");
        }
    }

    #[tokio::test]
    async fn test_multimodal_audio_node() {
        let store = MemoryStore::new();
        let embedding = vec![0.2, 0.7, 0.4, 0.1, 0.9, 0.3];

        let mm = MultimodalData::Audio {
            pointer: "s3://audio-bucket/recording-042.wav".to_string(),
            embedding: embedding.clone(),
        };

        let node = FractalNode::new_external_multimodal(
            "s3://audio-bucket/recording-042.wav".to_string(),
            embedding.clone(),
            HashMap::from([("source".to_string(), serde_json::json!("microphone"))]),
            mm,
        );
        let id = node.id;

        assert!(node.content.is_none(), "pointer-first: no raw content");
        assert!(node.multimodal.is_some());

        store.insert(node).await.expect("insert audio node");

        let retrieved = store.get(&id).await.unwrap().unwrap();
        assert_eq!(retrieved.vector.len(), embedding.len());
        assert!(retrieved.multimodal.is_some());

        if let Some(MultimodalData::Audio { pointer, .. }) = &retrieved.multimodal {
            assert_eq!(pointer, "s3://audio-bucket/recording-042.wav");
        } else {
            panic!("expected MultimodalData::Audio");
        }
    }

    #[tokio::test]
    async fn test_multimodal_sensor_node() {
        let store = MemoryStore::new();
        let embedding = vec![0.6, 0.2, 0.9];

        let sensor_data = serde_json::json!({
            "temperature": 22.5,
            "humidity": 45,
            "location": "living_room"
        });

        let mm = MultimodalData::Sensor {
            data: sensor_data.clone(),
            embedding: embedding.clone(),
        };

        let node = FractalNode::new_external_multimodal(
            "sensor://home/living_room/env".to_string(),
            embedding,
            HashMap::from([("source".to_string(), serde_json::json!("iot_hub"))]),
            mm,
        );
        let id = node.id;

        assert!(node.content.is_none(), "pointer-first: no raw content");

        store.insert(node).await.expect("insert sensor node");

        let retrieved = store.get(&id).await.unwrap().unwrap();
        assert!(retrieved.multimodal.is_some());
        assert!(retrieved.content.is_none());
        assert_eq!(
            retrieved.original_pointer.as_deref(),
            Some("sensor://home/living_room/env")
        );

        if let Some(MultimodalData::Sensor { data, embedding }) = &retrieved.multimodal {
            assert_eq!(data["temperature"], 22.5);
            assert_eq!(data["humidity"], 45);
            assert!(!embedding.is_empty());
        } else {
            panic!("expected MultimodalData::Sensor");
        }
    }

    // -- NodeType Tests --

    #[test]
    fn test_session_node_type() {
        use crate::memory::types::MemoryType;
        let node = FractalNode::new_session("test".into(), vec![0.1], HashMap::new());
        assert_eq!(node.memory_type, MemoryType::Episodic);
    }

    #[test]
    fn test_external_node_type() {
        use crate::memory::types::MemoryType;
        let node = FractalNode::new_external("s3://x".into(), vec![0.1], HashMap::new());
        assert_eq!(node.memory_type, MemoryType::Semantic);
    }

    #[test]
    fn test_node_type_serde_default() {
        use crate::memory::types::MemoryType;
        let json = r#"{"id":"00000000-0000-0000-0000-000000000000","vector":[],"content":null,"original_pointer":null,"metadata":{},"weight":1.0,"multimodal":null,"children":[],"relations":[],"created_at":"2026-01-01T00:00:00Z","last_accessed":"2026-01-01T00:00:00Z"}"#;
        let node: FractalNode = serde_json::from_str(json).expect("deserialize without node_type");
        assert_eq!(node.memory_type, MemoryType::Episodic, "default should be Episodic");
    }

    // -- Phase 1: Task-Prefix Tests --

    #[test]
    #[ignore = "OpenAI provider does not use document prefixes"]
    fn test_document_prefix_applied() {
        // OpenAI has no document prefix
    }

    #[test]
    #[ignore = "OpenAI provider does not use query prefixes"]
    fn test_query_prefix_applied() {
        // OpenAI has no query prefix
    }

    // -- Phase 3: BM25 Tests --

    #[tokio::test]
    async fn test_bm25_exact_keyword_match() {
        let store = MemoryStore::new();

        let n1 = FractalNode::new_session(
            "Der Frigate-Server meldet drei Kameras als aktiv".to_string(),
            vec![0.1, 0.2, 0.3, 0.4],
            HashMap::new(),
        );
        let n2 = FractalNode::new_session(
            "Die App soll anonym sein kein Login noetig".to_string(),
            vec![0.5, 0.6, 0.7, 0.8],
            HashMap::new(),
        );
        store.insert(n1).await.unwrap();
        store.insert(n2).await.unwrap();

        let results = store.search_bm25("Frigate Kameras", 5).await;
        assert!(!results.is_empty(), "BM25 should find keyword match");
        let top_content = store.get(&results[0].0).await.unwrap().unwrap();
        assert!(
            top_content.content.as_deref().unwrap().contains("Frigate"),
            "top BM25 result should contain 'Frigate'"
        );
    }

    #[tokio::test]
    async fn test_rrf_fusion_combines_both() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        let vector_ranked = vec![id_a, id_b, id_c];
        let bm25_ranked = vec![(id_b, 5.0), (id_a, 3.0)];

        let fused = MemoryStore::rrf_fuse(&vector_ranked, &bm25_ranked, 60.0);

        let a_score = fused.iter().find(|(id, _)| *id == id_a).unwrap().1;
        let b_score = fused.iter().find(|(id, _)| *id == id_b).unwrap().1;
        let c_score = fused.iter().find(|(id, _)| *id == id_c).unwrap().1;

        assert!(
            b_score > c_score,
            "node in both lists should rank higher than node in only one"
        );
        assert!(
            a_score > c_score,
            "node in both lists should rank higher than node in only one"
        );
    }

    #[tokio::test]
    async fn test_rrf_fusion_disjoint_lists() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        let vector_ranked = vec![id_a];
        let bm25_ranked = vec![(id_b, 5.0)];

        let fused = MemoryStore::rrf_fuse(&vector_ranked, &bm25_ranked, 60.0);

        assert_eq!(fused.len(), 2, "disjoint lists should merge all entries");
        let a_score = fused.iter().find(|(id, _)| *id == id_a).unwrap().1;
        let b_score = fused.iter().find(|(id, _)| *id == id_b).unwrap().1;
        assert!(
            (a_score - b_score).abs() < 1e-6,
            "equal rank in their respective lists should yield equal RRF score"
        );
    }

    #[tokio::test]
    #[cfg(feature = "postgres-storage")]
    async fn test_hybrid_retrieval_keyword_wins() {
        let store = MemoryStore::new();

        let n_frigate = FractalNode::new_session(
            "Frigate erkennt Bewegung an der Haustuer".to_string(),
            vec![0.1, 0.2, 0.3, 0.4],
            HashMap::new(),
        );
        let n_other = FractalNode::new_session(
            "Allgemeine Information ueber das Wetter".to_string(),
            vec![0.9, 0.8, 0.7, 0.6],
            HashMap::new(),
        );
        let frigate_id = n_frigate.id;

        store.insert(n_frigate).await.unwrap();
        store.insert(n_other).await.unwrap();

        let query_vec = vec![0.5, 0.5, 0.5, 0.5];
        let results = store.hybrid_retrieve(
            Some("Frigate Haustuer"),
            &query_vec,
            5,
            0,
            None,
        ).await;

        assert!(!results.is_empty());
        assert_eq!(
            results[0].1.id, frigate_id,
            "BM25 keyword match should boost Frigate node to top"
        );
        assert!(results[0].0 > 0.0, "score should be positive");
    }

    #[tokio::test]
    async fn test_hybrid_retrieval_semantic_fallback() {
        let store = MemoryStore::new();

        let n1 = FractalNode::new_session(
            "alpha vector".to_string(),
            vec![1.0, 0.0, 0.0, 0.0],
            HashMap::new(),
        );
        let n2 = FractalNode::new_session(
            "beta vector".to_string(),
            vec![0.0, 1.0, 0.0, 0.0],
            HashMap::new(),
        );
        let alpha_id = n1.id;

        store.insert(n1).await.unwrap();
        store.insert(n2).await.unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        #[cfg(feature = "postgres-storage")]
        let results = store.hybrid_retrieve(None, &query_vec, 2, 0, None).await;
        #[cfg(not(feature = "postgres-storage"))]
        let results = store.hybrid_retrieve(None, &query_vec, 2, 0).await;

        assert!(!results.is_empty());
        assert_eq!(
            results[0].1.id, alpha_id,
            "without query_text, pure vector search should work"
        );
        assert!(results[0].0 > 0.0, "score should be positive");
    }
}

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::multimodal::MultimodalData;

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let mag_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Relation {
    pub target_id: Uuid,
    pub relation_type: String,
    pub strength: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FractalNode {
    pub id: Uuid,
    pub vector: Vec<f32>,
    pub content: Option<String>,
    pub original_pointer: Option<String>,
    #[schema(value_type = Object)]
    pub metadata: HashMap<String, Value>,
    pub weight: f64,
    pub multimodal: Option<MultimodalData>,
    #[schema(value_type = Vec<Object>)]
    pub children: Vec<FractalNode>,
    pub relations: Vec<Relation>,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
}

impl FractalNode {
    /// Session-Knoten: speichert den vollen Text + Embedding.
    pub fn new_session(content: String, vector: Vec<f32>, metadata: HashMap<String, Value>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            vector,
            content: Some(content),
            original_pointer: None,
            metadata,
            weight: 1.0,
            multimodal: None,
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    /// Externer Knoten: speichert NUR den Pointer + Embedding, nie Rohdaten.
    pub fn new_external(
        pointer: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            vector,
            content: None,
            original_pointer: Some(pointer),
            metadata,
            weight: 1.0,
            multimodal: None,
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    /// Externer Knoten mit multimodalen Daten (Image/Audio/Sensor).
    pub fn new_external_multimodal(
        pointer: String,
        vector: Vec<f32>,
        metadata: HashMap<String, Value>,
        multimodal: MultimodalData,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            vector,
            content: None,
            original_pointer: Some(pointer),
            metadata,
            weight: 1.0,
            multimodal: Some(multimodal),
            children: Vec::new(),
            relations: Vec::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    pub fn find_best_child(&self, query_vector: &[f32]) -> Option<&FractalNode> {
        self.children.iter().max_by(|a, b| {
            let sim_a = cosine_similarity(&a.vector, query_vector);
            let sim_b = cosine_similarity(&b.vector, query_vector);
            sim_a.partial_cmp(&sim_b).unwrap_or(Ordering::Equal)
        })
    }

    /// Rekursives Zoomen: sammelt (similarity, node) Paare entlang des besten Pfads.
    pub fn zoom_retrieve(
        &self,
        query_vector: &[f32],
        max_depth: usize,
    ) -> Vec<(f32, FractalNode)> {
        let sim = cosine_similarity(&self.vector, query_vector);
        let mut results = vec![(sim, self.clone())];
        if max_depth > 0 {
            if let Some(best) = self.find_best_child(query_vector) {
                results.extend(best.zoom_retrieve(query_vector, max_depth - 1));
            }
        }
        results
    }
}

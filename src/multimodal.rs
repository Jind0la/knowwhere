use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum MultimodalData {
    Image {
        pointer: String,
        embedding: Vec<f32>,
    },
    Audio {
        pointer: String,
        embedding: Vec<f32>,
    },
    Sensor {
        #[schema(value_type = Object)]
        data: Value,
        embedding: Vec<f32>,
    },
}

impl MultimodalData {
    pub fn embedding(&self) -> &[f32] {
        match self {
            Self::Image { embedding, .. } => embedding,
            Self::Audio { embedding, .. } => embedding,
            Self::Sensor { embedding, .. } => embedding,
        }
    }

    pub fn pointer_or_label(&self) -> &str {
        match self {
            Self::Image { pointer, .. } => pointer,
            Self::Audio { pointer, .. } => pointer,
            Self::Sensor { .. } => "sensor-data",
        }
    }
}

pub trait CrossModalEmbedder: Send + Sync {
    fn cross_embed(&self, data: &MultimodalData) -> Vec<f32>;
}

pub struct PlaceholderCrossModalEmbedder;

impl CrossModalEmbedder for PlaceholderCrossModalEmbedder {
    fn cross_embed(&self, data: &MultimodalData) -> Vec<f32> {
        data.embedding().to_vec()
    }
}

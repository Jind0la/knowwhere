pub mod drive;
pub mod frigate;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::FractalNode;
use crate::multimodal::MultimodalData;
use crate::storage::MemoryStore;

pub struct ExternalEvent {
    pub pointer: String,
    pub metadata: HashMap<String, Value>,
    pub multimodal: Option<MultimodalData>,
}

/// Shared helper: embeds + stores an ExternalEvent as a FractalNode.
/// Used by both the HTTP route and the connector manager.
pub async fn store_external_event(
    store: &MemoryStore,
    embedding: &Arc<dyn EmbeddingProvider>,
    event: ExternalEvent,
) -> Result<Uuid> {
    let vector = if let Some(ref mm) = event.multimodal {
        let emb = mm.embedding();
        if !emb.is_empty() {
            emb.to_vec()
        } else {
            embedding.embed(&event.pointer).await?
        }
    } else {
        embedding.embed(&event.pointer).await?
    };

    let node = match event.multimodal {
        Some(mm) => {
            FractalNode::new_external_multimodal(event.pointer, vector, event.metadata, mm)
        }
        None => FractalNode::new_external(event.pointer, vector, event.metadata),
    };

    let id = store.insert(node).await?;
    tracing::info!(%id, "external event stored via connector");
    Ok(id)
}

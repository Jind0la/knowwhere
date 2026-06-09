#[cfg(feature = "google-drive")]
pub mod drive;
pub mod frigate;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::storage::StorageBackend;
use uuid::Uuid;

use crate::embedding::EmbeddingProvider;
use crate::memory::FractalNode;
use crate::multimodal::MultimodalData;

pub struct ExternalEvent {
    pub pointer: String,
    pub metadata: HashMap<String, Value>,
    pub multimodal: Option<MultimodalData>,
    /// Optional historical timestamp. When provided, the stored node
    /// will use this timestamp instead of Utc::now().
    pub created_at: Option<DateTime<Utc>>,
}

/// Shared helper: embeds + stores an ExternalEvent as a FractalNode.
/// Used by both the HTTP route and the connector manager.
pub async fn store_external_event(
    store: &dyn StorageBackend,
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
        Some(mm) => FractalNode::new_external_multimodal(
            event.pointer,
            vector,
            event.metadata,
            mm,
            event.created_at,
        ),
        None => FractalNode::new_external(event.pointer, vector, event.metadata, event.created_at),
    };

    let id = store.insert(node).await?;
    tracing::info!(%id, "external event stored via connector");
    Ok(id)
}

/// Stores multiple ExternalEvents in batch.
/// Only handles events WITHOUT multimodal data (those go through embed() individually).
/// For events with multimodal data, use store_external_event individually.
pub async fn store_external_events_batch(
    store: &dyn StorageBackend,
    embedding: &Arc<dyn EmbeddingProvider>,
    events: Vec<ExternalEvent>,
) -> Result<Vec<Uuid>> {
    let (plain_events, multimodal_events): (Vec<_>, Vec<_>) =
        events.into_iter().partition(|e| e.multimodal.is_none());

    let pointers: Vec<&str> = plain_events.iter().map(|e| e.pointer.as_str()).collect();
    let embeddings =
        crate::embedding::provider::embed_document_batch(embedding.as_ref(), &pointers).await?;

    let nodes: Result<Vec<_>> = plain_events
        .into_iter()
        .zip(embeddings)
        .map(|(event, vector)| {
            Ok(FractalNode::new_external(
                event.pointer,
                vector,
                event.metadata,
                event.created_at,
            ))
        })
        .collect();
    let nodes = nodes?;

    let mut ids = store.insert_many(nodes).await?;

    for event in multimodal_events {
        let id = store_external_event(store, embedding, event).await?;
        ids.push(id);
        tracing::warn!(%id, "multimodal event stored individually (batch follow-up pending)");
    }

    Ok(ids)
}

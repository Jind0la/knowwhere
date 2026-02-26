use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use crate::multimodal::MultimodalData;

use super::ExternalEvent;

pub struct FrigateConnector {
    pub base_url: String,
    pub poll_interval: Duration,
}

impl FrigateConnector {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            poll_interval: Duration::from_secs(30),
        }
    }

    /// Placeholder: returns dummy camera events.
    /// Will be replaced with real Frigate API calls / webhook listener.
    pub async fn poll_events(&self) -> Result<Vec<ExternalEvent>> {
        let event_id = uuid::Uuid::new_v4();
        let pointer = format!(
            "frigate://{}/api/events/{}/snapshot",
            self.base_url, event_id
        );

        let dummy_embedding = vec![0.1; 384];

        let event = ExternalEvent {
            pointer: pointer.clone(),
            metadata: HashMap::from([
                ("source".to_string(), json!("frigate")),
                ("camera".to_string(), json!("front_door")),
                ("label".to_string(), json!("person")),
                ("score".to_string(), json!(0.92)),
            ]),
            multimodal: Some(MultimodalData::Image {
                pointer,
                embedding: dummy_embedding,
            }),
        };

        Ok(vec![event])
    }
}

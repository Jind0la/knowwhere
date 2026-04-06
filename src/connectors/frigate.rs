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

    /// Poll real Frigate events. Returns empty vec if Frigate is unreachable.
    pub async fn poll_events(&self) -> Result<Vec<ExternalEvent>> {
        let url = format!("{}/api/events?limit=5&has_snapshot=1", self.base_url);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;

        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return Ok(vec![]),
        };

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let events: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
        let mut result = Vec::new();

        for ev in events {
            let Some(event_id) = ev.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let pointer = format!(
                "frigate://{}/api/events/{}/snapshot",
                self.base_url, event_id
            );

            let camera = ev
                .get("camera")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let label = ev
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let score = ev.get("top_score").and_then(|v| v.as_f64()).unwrap_or(0.0);

            result.push(ExternalEvent {
                pointer: pointer.clone(),
                metadata: HashMap::from([
                    ("source".to_string(), json!("frigate")),
                    ("camera".to_string(), json!(camera)),
                    ("label".to_string(), json!(label)),
                    ("score".to_string(), json!(score)),
                ]),
                multimodal: Some(MultimodalData::Image {
                    pointer,
                    embedding: vec![],
                }),
            });
        }

        Ok(result)
    }
}

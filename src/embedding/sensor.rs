use anyhow::Result;
use serde_json::Value;

use super::provider::{embed_document, EmbeddingProvider};

/// Convert arbitrary sensor JSON data into a readable plain-text representation.
///
/// - Objects: each key-value pair as "key value"
/// - Arrays: comma-joined values
/// - Scalars: direct to_string()
/// - Nested structures: flattened recursively with space separation
pub fn sensor_to_text(data: &Value) -> String {
    match data {
        Value::Object(map) => {
            let mut parts = Vec::with_capacity(map.len());
            for (key, val) in map {
                parts.push(format!("{} {}", key, sensor_to_text(val)));
            }
            parts.join(" ")
        }
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(sensor_to_text).collect();
            parts.join(", ")
        }
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
    }
}

/// Embed sensor data by first converting it to text, then running it through
/// the configured embedding provider via `embed_document`.
///
/// Returns a 768-dim embedding vector (or whatever the provider's dimension is).
pub async fn embed_sensor(data: &Value, provider: &dyn EmbeddingProvider) -> Result<Vec<f32>> {
    let text = sensor_to_text(data);
    embed_document(provider, &text).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Minimal mock provider that records what text it received and returns
    /// a fixed-length embedding so we can verify both serialization and output shape.
    struct MockSensorProvider {
        received: Mutex<Vec<String>>,
    }

    impl MockSensorProvider {
        fn new() -> Self {
            Self {
                received: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for MockSensorProvider {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.received.lock().unwrap().push(text.to_string());
            // Return a 768-dim vector so embed_sensor consumers can assert on shape
            Ok(vec![0.5; 768])
        }

        fn dimension(&self) -> usize {
            768
        }

        fn name(&self) -> &str {
            "mock-sensor"
        }
    }

    // --- sensor_to_text tests ---

    #[test]
    fn test_sensor_to_text_flat_object() {
        let data = serde_json::json!({
            "temperature": 23.5,
            "humidity": 60,
            "unit": "celsius"
        });
        let text = sensor_to_text(&data);
        // Order is not guaranteed (HashMap), so check each key-value pair is present
        assert!(text.contains("temperature 23.5"));
        assert!(text.contains("humidity 60"));
        assert!(text.contains("unit celsius"));
    }

    #[test]
    fn test_sensor_to_text_array() {
        let data = serde_json::json!([1, 2, 3, 4, 5]);
        let text = sensor_to_text(&data);
        assert_eq!(text, "1, 2, 3, 4, 5");
    }

    #[test]
    fn test_sensor_to_text_scalar_string() {
        let data = Value::String("online".to_string());
        assert_eq!(sensor_to_text(&data), "online");
    }

    #[test]
    fn test_sensor_to_text_scalar_number() {
        let data = Value::Number(serde_json::Number::from(42));
        assert_eq!(sensor_to_text(&data), "42");
    }

    #[test]
    fn test_sensor_to_text_bool() {
        assert_eq!(sensor_to_text(&Value::Bool(true)), "true");
        assert_eq!(sensor_to_text(&Value::Bool(false)), "false");
    }

    #[test]
    fn test_sensor_to_text_null() {
        assert_eq!(sensor_to_text(&Value::Null), "null");
    }

    #[test]
    fn test_sensor_to_text_nested_object() {
        let data = serde_json::json!({
            "device": {
                "id": "esp32-01",
                "location": {"lat": 52.52, "lon": 13.40}
            },
            "reading": 42
        });
        let text = sensor_to_text(&data);
        assert!(text.contains("device id esp32-01 location lat 52.52 lon 13.4"));
        assert!(text.contains("reading 42"));
    }

    #[test]
    fn test_sensor_to_text_nested_array() {
        let data = serde_json::json!({
            "readings": [10, 20, 30],
            "status": "ok"
        });
        let text = sensor_to_text(&data);
        assert!(text.contains("readings 10, 20, 30"));
        assert!(text.contains("status ok"));
    }

    #[test]
    fn test_sensor_to_text_empty_object() {
        let data = serde_json::json!({});
        assert_eq!(sensor_to_text(&data), "");
    }

    #[test]
    fn test_sensor_to_text_empty_array() {
        let data = serde_json::json!([]);
        assert_eq!(sensor_to_text(&data), "");
    }

    // --- embed_sensor tests ---

    #[tokio::test]
    async fn test_embed_sensor_returns_768_dim() {
        let provider = MockSensorProvider::new();
        let data = serde_json::json!({"temp": 23.5, "hum": 60});
        let embedding = embed_sensor(&data, &provider).await.unwrap();
        assert_eq!(embedding.len(), 768);
        assert!(embedding.iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[tokio::test]
    async fn test_embed_sensor_passes_text_to_provider() {
        let provider = MockSensorProvider::new();
        let data = serde_json::json!({"sensor": "dht22", "value": 72.1});
        embed_sensor(&data, &provider).await.unwrap();

        let received = provider.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        let text = &received[0];
        assert!(text.contains("sensor dht22"));
        assert!(text.contains("value 72.1"));
    }
}

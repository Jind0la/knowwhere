use super::*;
use crate::connectors::store_external_event;
use crate::connectors::ExternalEvent;

// -- HomeAssistant Webhook --

/// HomeAssistant webhook payload (flexible — captures entity state changes).
#[derive(Debug, Deserialize, ToSchema)]
pub struct HomeAssistantWebhookPayload {
    /// Unique event ID (used for deduplication). If absent, derived from payload hash.
    #[serde(default)]
    pub event_id: Option<String>,
    /// Entity that triggered the webhook (e.g., "light.living_room").
    #[serde(default)]
    pub entity_id: Option<String>,
    /// Service/action name (e.g., "turn_on").
    #[serde(default)]
    pub service: Option<String>,
    /// New state value (scalar or object).
    #[serde(default)]
    pub state: Option<serde_json::Value>,
    /// Extra attributes from the event.
    #[serde(default)]
    pub attributes: Option<serde_json::Value>,
    /// Full raw payload for metadata passthrough.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl HomeAssistantWebhookPayload {
    fn pointer(&self) -> String {
        match (&self.entity_id, &self.event_id) {
            (Some(entity), Some(id)) => {
                format!("homeassistant://entities/{}/events/{}", entity, id)
            }
            (Some(entity), None) => format!("homeassistant://entities/{}/events/unknown", entity),
            (None, Some(id)) => format!("homeassistant://events/{}", id),
            (None, None) => "homeassistant://events/unknown".to_string(),
        }
    }

    fn dedup_key(&self) -> String {
        if let Some(ref id) = self.event_id {
            return format!("homeassistant:{}", id);
        }
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.extra.to_string().hash(&mut hasher);
        self.entity_id.as_deref().unwrap_or("").hash(&mut hasher);
        format!("homeassistant:hash:{}", hasher.finish())
    }

    fn to_external_event(self) -> ExternalEvent {
        let pointer = self.pointer();
        let mut metadata = std::collections::HashMap::from([(
            "source".to_string(),
            serde_json::json!("homeassistant"),
        )]);
        if let Some(ref entity) = self.entity_id {
            metadata.insert("entity_id".to_string(), serde_json::json!(entity));
        }
        if let Some(ref service) = self.service {
            metadata.insert("service".to_string(), serde_json::json!(service));
        }
        if let Some(ref state) = self.state {
            metadata.insert("state".to_string(), state.clone());
        }
        ExternalEvent {
            pointer,
            metadata,
            multimodal: None,
            created_at: None,
        }
    }
}

#[utoipa::path(
    post,
    path = "/webhooks/homeassistant",
    tag = "webhooks",
    params(
        ("secret" = Option<String>, Query, description = "Webhook secret (alternative to X-Webhook-Secret header)")
    ),
    request_body = HomeAssistantWebhookPayload,
    responses(
        (status = 200, description = "Event stored", body = WebhookResponse),
        (status = 401, description = "Unauthorized — invalid or missing secret", body = String),
        (status = 409, description = "Duplicate event — already processed", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn webhook_homeassistant(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<HomeAssistantWebhookPayload>,
) -> Result<Json<WebhookResponse>, (StatusCode, String)> {
    let webhook_secret = state.homeassistant_webhook_secret.as_deref();
    let header_secret = headers
        .get("X-Webhook-Secret")
        .and_then(|v| v.to_str().ok());
    let query_secret = params.get("secret").map(|s| s.as_str());

    if !check_webhook_secret(webhook_secret, header_secret, query_secret) {
        tracing::warn!("homeassistant webhook: unauthorized (bad secret)");
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid or missing webhook secret".into(),
        ));
    }

    let dedup_key = payload.dedup_key();
    if state.homeassistant_dedup.seen_or_insert(&dedup_key).await {
        tracing::debug!(dedup_key = %dedup_key, "homeassistant webhook: duplicate event");
        return Err((StatusCode::CONFLICT, "event already processed".into()));
    }

    let event_id = payload
        .event_id
        .clone()
        .unwrap_or_else(|| dedup_key.clone());
    let event = payload.to_external_event();
    match store_external_event(state.store.as_ref(), &state.embedding, event).await {
        Ok(id) => {
            tracing::info!(event_id = %event_id, node_id = %id, "homeassistant webhook event stored");
            Ok(Json(WebhookResponse {
                status: "stored".into(),
                event_id,
            }))
        }
        Err(e) => {
            tracing::error!(event_id = %event_id, error = %e, "homeassistant webhook: failed to store");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store failed: {e}"),
            ))
        }
    }
}

/// Frigate event payload from webhook POST body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FrigateWebhookEvent {
    /// Unique event ID from Frigate (used for deduplication).
    pub id: String,
    /// Camera name that captured the event.
    #[serde(default)]
    pub camera: String,
    /// Detected label (e.g., "person", "car").
    #[serde(default)]
    pub label: String,
    /// Confidence/top score of the detection.
    #[serde(default)]
    pub top_score: f64,
    /// Pointer to the snapshot image.
    #[serde(default)]
    pub snapshot_path: Option<String>,
    /// Pointer to the clip video.
    #[serde(default)]
    pub clip_path: Option<String>,
}

impl FrigateWebhookEvent {
    fn pointer(&self) -> String {
        format!("frigate://cameras/{}/events/{}", self.camera, self.id)
    }

    fn multimodal(&self) -> Option<MultimodalData> {
        if let Some(ref path) = self.snapshot_path {
            return Some(MultimodalData::Image {
                pointer: path.clone(),
                embedding: vec![],
            });
        }
        if let Some(ref path) = self.clip_path {
            return Some(MultimodalData::Image {
                pointer: path.clone(),
                embedding: vec![],
            });
        }
        None
    }

    fn to_external_event(self) -> ExternalEvent {
        use serde_json::json;
        let pointer = self.pointer();
        let multimodal = self.multimodal();
        let camera = self.camera;
        let label = self.label;
        let top_score = self.top_score;
        ExternalEvent {
            pointer,
            metadata: std::collections::HashMap::from([
                ("source".to_string(), json!("frigate")),
                ("camera".to_string(), json!(camera)),
                ("label".to_string(), json!(label)),
                ("score".to_string(), json!(top_score)),
            ]),
            multimodal,
            created_at: None,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct WebhookResponse {
    pub status: String,
    pub event_id: String,
}

#[utoipa::path(
    post,
    path = "/webhooks/frigate",
    tag = "webhooks",
    params(
        ("secret" = Option<String>, Query, description = "Webhook secret (alternative to X-Webhook-Secret header)")
    ),
    request_body = FrigateWebhookEvent,
    responses(
        (status = 200, description = "Event stored", body = WebhookResponse),
        (status = 401, description = "Unauthorized — invalid or missing secret", body = String),
        (status = 409, description = "Duplicate event — already processed", body = String),
        (status = 500, description = "Internal error", body = String)
    )
)]
pub async fn webhook_frigate(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<FrigateWebhookEvent>,
) -> Result<Json<WebhookResponse>, (StatusCode, String)> {
    let webhook_secret = state.frigate_webhook_secret.as_deref();
    let header_secret = headers
        .get("X-Webhook-Secret")
        .and_then(|v| v.to_str().ok());
    let query_secret = params.get("secret").map(|s| s.as_str());

    if !check_webhook_secret(webhook_secret, header_secret, query_secret) {
        tracing::warn!("frigate webhook: unauthorized (bad secret)");
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid or missing webhook secret".into(),
        ));
    }

    let event_id = payload.id.clone();
    let dedup_key = format!("frigate:{}", event_id);
    if state.frigate_dedup.seen_or_insert(&dedup_key).await {
        tracing::debug!(event_id = %event_id, "frigate webhook: duplicate event");
        return Err((
            StatusCode::CONFLICT,
            format!("event {} already processed", event_id),
        ));
    }

    let event = payload.to_external_event();
    match store_external_event(state.store.as_ref(), &state.embedding, event).await {
        Ok(id) => {
            tracing::info!(event_id = %event_id, node_id = %id, "frigate webhook event stored");
            Ok(Json(WebhookResponse {
                status: "stored".into(),
                event_id,
            }))
        }
        Err(e) => {
            tracing::error!(event_id = %event_id, error = %e, "frigate webhook: failed to store");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store failed: {e}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::webhooks::check_webhook_secret;
    use serde_json::json;

    #[test]
    fn ha_payload_pointer_full() {
        let payload = HomeAssistantWebhookPayload {
            event_id: Some("evt_42".into()),
            entity_id: Some("light.kitchen".into()),
            service: Some("turn_on".into()),
            state: Some(json!("on")),
            attributes: None,
            extra: json!({}),
        };
        assert_eq!(
            payload.pointer(),
            "homeassistant://entities/light.kitchen/events/evt_42"
        );
    }

    #[test]
    fn ha_payload_pointer_entity_only() {
        let payload = HomeAssistantWebhookPayload {
            event_id: None,
            entity_id: Some("sensor.temp".into()),
            service: None,
            state: Some(json!(22.5)),
            attributes: None,
            extra: json!({}),
        };
        assert_eq!(
            payload.pointer(),
            "homeassistant://entities/sensor.temp/events/unknown"
        );
    }

    #[test]
    fn ha_payload_pointer_event_only() {
        let payload = HomeAssistantWebhookPayload {
            event_id: Some("abc123".into()),
            entity_id: None,
            service: None,
            state: None,
            attributes: None,
            extra: json!({}),
        };
        assert_eq!(payload.pointer(), "homeassistant://events/abc123");
    }

    #[test]
    fn ha_payload_dedup_with_event_id() {
        let payload = HomeAssistantWebhookPayload {
            event_id: Some("unique_event".into()),
            entity_id: None,
            service: None,
            state: None,
            attributes: None,
            extra: json!({}),
        };
        assert_eq!(payload.dedup_key(), "homeassistant:unique_event");
    }

    #[test]
    fn ha_payload_dedup_fallback_hash_stable() {
        let p1 = HomeAssistantWebhookPayload {
            event_id: None,
            entity_id: Some("light.hall".into()),
            service: None,
            state: Some(json!("off")),
            attributes: None,
            extra: json!({}),
        };
        let p2 = HomeAssistantWebhookPayload {
            event_id: None,
            entity_id: Some("light.hall".into()),
            service: None,
            state: Some(json!("off")),
            attributes: None,
            extra: json!({}),
        };
        assert_eq!(p1.dedup_key(), p2.dedup_key());
    }

    #[test]
    fn ha_payload_to_external_event() {
        let payload = HomeAssistantWebhookPayload {
            event_id: Some("evt_1".into()),
            entity_id: Some("switch.fan".into()),
            service: Some("toggle".into()),
            state: Some(json!(true)),
            attributes: Some(json!({"friendly_name": "Fan"})),
            extra: json!({}),
        };
        let event = payload.to_external_event();
        assert!(event.pointer.contains("switch.fan"));
        assert_eq!(
            event.metadata.get("source").unwrap(),
            &json!("homeassistant")
        );
        assert_eq!(
            event.metadata.get("entity_id").unwrap(),
            &json!("switch.fan")
        );
        assert_eq!(event.metadata.get("service").unwrap(), &json!("toggle"));
        assert_eq!(event.metadata.get("state").unwrap(), &json!(true));
        assert!(event.multimodal.is_none());
    }

    #[test]
    fn secret_configured_and_matches_header() {
        assert!(check_webhook_secret(
            Some("my-secret"),
            Some("my-secret"),
            None
        ));
    }

    #[test]
    fn secret_configured_and_matches_query() {
        assert!(check_webhook_secret(
            Some("my-secret"),
            None,
            Some("my-secret")
        ));
    }

    #[test]
    fn secret_not_configured_rejects() {
        assert!(!check_webhook_secret(None, Some("anything"), None));
    }

    #[test]
    fn secret_empty_rejects() {
        assert!(!check_webhook_secret(Some(""), Some(""), None));
    }

    #[test]
    fn secret_mismatch_rejects() {
        assert!(!check_webhook_secret(Some("correct"), Some("wrong"), None));
    }
}

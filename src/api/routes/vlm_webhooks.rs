use super::*;
use crate::connectors::store_external_event;
use crate::connectors::ExternalEvent;

// ---------------------------------------------------------------------------
// VLM Worker Routes — Summarization via Background Worker
// ---------------------------------------------------------------------------

/// Request body for enqueuing a summarization job.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VlmEnqueueRequest {
    /// IDs of memory nodes to summarize.
    pub node_ids: Vec<Uuid>,
    /// Context level for the summary.
    #[serde(default)]
    pub context: SummaryContext,
    /// Optional priority (1–10, higher = processed first). Default 5.
    #[serde(default = "default_vlm_priority")]
    pub priority: u8,
}

fn default_vlm_priority() -> u8 {
    5
}

/// Response after enqueuing a job.
#[derive(Serialize, ToSchema)]
pub struct VlmEnqueueResponse {
    pub job_id: Uuid,
    pub queue_depth: usize,
}

/// GET /vlm/status — Worker queue status.
#[utoipa::path(
    get,
    path = "/vlm/status",
    tag = "vlm",
    responses(
        (status = 200, description = "VLM worker status", body = VlmWorkerStatus),
        (status = 503, description = "VLM worker not configured", body = String)
    )
)]
pub async fn vlm_status(
    State(state): State<AppState>,
) -> Result<Json<VlmWorkerStatus>, (StatusCode, String)> {
    match &state.vlm_worker {
        Some(handle) => {
            let status = handle.status().await;
            Ok(Json(status))
        }
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "VLM worker not configured (set OPENAI_API_KEY or GROK_API_KEY)".into(),
        )),
    }
}

/// POST /vlm/summarize — Enqueue a summarization job (non-blocking).
#[utoipa::path(
    post,
    path = "/vlm/summarize",
    tag = "vlm",
    request_body = VlmEnqueueRequest,
    responses(
        (status = 202, description = "Job enqueued", body = VlmEnqueueResponse),
        (status = 400, description = "Invalid request", body = String),
        (status = 503, description = "VLM worker not configured", body = String)
    )
)]
pub async fn vlm_enqueue(
    State(state): State<AppState>,
    Json(req): Json<VlmEnqueueRequest>,
) -> Result<(StatusCode, Json<VlmEnqueueResponse>), (StatusCode, String)> {
    let handle = state.vlm_worker.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "VLM worker not configured".into(),
        )
    })?;

    if req.node_ids.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "node_ids must not be empty".into()));
    }

    if req.priority == 0 || req.priority > 10 {
        return Err((StatusCode::BAD_REQUEST, "priority must be 1–10".into()));
    }

    let job = VlmJob::new(req.node_ids.clone(), req.context).with_priority(req.priority);
    let job_id = job.id;

    handle
        .enqueue(job)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let status = handle.status().await;

    tracing::info!(job_id = %job_id, queue_depth = status.queue_depth, "VLM job enqueued");

    Ok((
        StatusCode::ACCEPTED,
        Json(VlmEnqueueResponse {
            job_id,
            queue_depth: status.queue_depth,
        }),
    ))
}

// -- Frigate Webhook --

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

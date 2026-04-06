use super::*;

// -- Governance Policy --

/// Get the current governance policy.
#[utoipa::path(
    get,
    path = "/governance/policy",
    tag = "governance",
    responses(
        (status = 200, description = "Current governance policy", body = GovernancePolicy)
    )
)]
pub async fn get_governance_policy(State(state): State<AppState>) -> Json<GovernancePolicy> {
    Json(state.governance_policy.read().await.clone())
}

/// Update the governance policy.
#[derive(Deserialize, ToSchema)]
pub struct UpdatePolicyRequest {
    #[serde(default)]
    pub min_confidence: Option<f64>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
    #[serde(default)]
    pub blocked_sensitivities: Option<Vec<Sensitivity>>,
    #[serde(default)]
    pub supersession_enabled: Option<bool>,
    #[serde(default)]
    pub conflict_check_enabled: Option<bool>,
    #[serde(default)]
    pub recency_boost_enabled: Option<bool>,
    #[serde(default)]
    pub recency_penalty_after_days: Option<u32>,
    /// Preset: "default", "strict", or "lenient". Overrides other fields if set.
    #[serde(default)]
    pub preset: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct UpdatePolicyResponse {
    pub message: String,
    pub policy: GovernancePolicy,
}

#[utoipa::path(
    post,
    path = "/governance/policy",
    tag = "governance",
    request_body = UpdatePolicyRequest,
    responses(
        (status = 200, description = "Policy updated", body = UpdatePolicyResponse)
    )
)]
pub async fn update_governance_policy(
    State(state): State<AppState>,
    Json(req): Json<UpdatePolicyRequest>,
) -> Json<UpdatePolicyResponse> {
    let mut policy = state.governance_policy.read().await.clone();

    if let Some(preset) = req.preset {
        policy = match preset.as_str() {
            "strict" => GovernancePolicy::strict(),
            "lenient" => GovernancePolicy::lenient(),
            _ => GovernancePolicy::default_policy(),
        };
    }

    if let Some(v) = req.min_confidence {
        policy.min_confidence = v.clamp(0.0, 1.0);
    }
    if let Some(v) = req.max_age_days {
        policy.max_age_days = Some(v);
    }
    if let Some(v) = req.blocked_sensitivities {
        policy.blocked_sensitivities = v;
    }
    if let Some(v) = req.supersession_enabled {
        policy.supersession_enabled = v;
    }
    if let Some(v) = req.conflict_check_enabled {
        policy.conflict_check_enabled = v;
    }
    if let Some(v) = req.recency_boost_enabled {
        policy.recency_boost_enabled = v;
    }
    if let Some(v) = req.recency_penalty_after_days {
        policy.recency_penalty_after_days = v;
    }

    *state.governance_policy.write().await = policy.clone();
    tracing::info!(?policy, "governance policy updated");

    Json(UpdatePolicyResponse {
        message: "governance policy updated".to_string(),
        policy,
    })
}

// -- Event Log (Layer 0 — read-only) --

#[derive(Deserialize, IntoParams)]
pub struct EventsQuery {
    /// Only return events after this ID (cursor-based pagination).
    #[serde(default)]
    pub after_id: Option<Uuid>,
    /// Maximum number of events to return (default 100, max 1000).
    #[serde(default = "default_events_limit")]
    pub limit: i64,
}

fn default_events_limit() -> i64 {
    100
}

#[utoipa::path(
    get,
    path = "/events",
    tag = "system",
    params(EventsQuery),
    responses(
        (status = 200, description = "Event log entries (in-memory, single-node)")
    )
)]
pub async fn list_events(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
) -> Json<Vec<Event>> {
    let limit = q.limit.min(1000);
    match state.events.read_after(q.after_id, limit).await {
        Ok(events) => Json(events),
        Err(e) => {
            tracing::warn!("failed to read events: {e}");
            Json(vec![])
        }
    }
}

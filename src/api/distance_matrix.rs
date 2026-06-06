/// Distance matrix endpoint with OSRM + Haversine fallback.
///
/// Queries a local OSRM service at http://localhost:5000 for driving distances.
/// Falls back to Haversine great-circle distance when OSRM is unreachable or returns no route.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::types::AppState;

/// Earth's mean radius in meters (WGS-84).
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// OSRM service base URL.
const OSRM_BASE_URL: &str = "http://localhost:5000";

/// Haversine formula — great-circle distance between two points in meters.
///
/// Computes the shortest distance over the earth's surface, ignoring elevation.
/// Accurate to within ~0.3% for most distances.
pub fn haversine_distance(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let lat1 = lat1.to_radians();
    let lng1 = lng1.to_radians();
    let lat2 = lat2.to_radians();
    let lng2 = lng2.to_radians();

    let dlat = lat2 - lat1;
    let dlng = lng2 - lng1;

    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS_M * c
}

/// Query OSRM for driving distance between two points. Returns None on any failure.
async fn osrm_distance(client: &reqwest::Client, lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> Option<f64> {
    let url = format!(
        "{}/route/v1/driving/{},{};{},{}?overview=false",
        OSRM_BASE_URL, lng1, lat1, lng2, lat2
    );

    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;
    let routes = body.get("routes")?.as_array()?;
    let first_route = routes.first()?;
    let distance = first_route.get("distance")?.as_f64()?;

    // OSRM returns distance in meters
    Some(distance)
}

/// Compute distance for a single origin-destination pair.
/// Tries OSRM first; falls back to Haversine on any failure.
async fn compute_distance(
    client: &reqwest::Client,
    origin: &LatLng,
    dest: &LatLng,
) -> f64 {
    // Try OSRM driving distance first
    if let Some(d) = osrm_distance(client, origin.lat, origin.lng, dest.lat, dest.lng).await {
        return d;
    }

    // Fall back to Haversine great-circle distance
    haversine_distance(origin.lat, origin.lng, dest.lat, dest.lng)
}

/// A coordinate pair [latitude, longitude].
#[derive(Deserialize, Serialize, ToSchema)]
pub struct LatLng {
    /// Latitude in decimal degrees.
    pub lat: f64,
    /// Longitude in decimal degrees.
    pub lng: f64,
}

impl From<[f64; 2]> for LatLng {
    fn from(arr: [f64; 2]) -> Self {
        LatLng {
            lat: arr[0],
            lng: arr[1],
        }
    }
}

/// Request body for the distance matrix endpoint.
#[derive(Deserialize, ToSchema)]
pub struct DistanceMatrixRequest {
    /// List of origin coordinates as [lat, lng] pairs.
    pub origins: Vec<LatLng>,
    /// List of destination coordinates as [lat, lng] pairs.
    pub destinations: Vec<LatLng>,
}

/// Response body: 2D array where response[i][j] = distance(origins[i], destinations[j]) in meters.
#[derive(Serialize, ToSchema)]
pub struct DistanceMatrixResponse {
    pub distances: Vec<Vec<f64>>,
}

/// POST /distance-matrix
///
/// For each origin-destination pair, queries the local OSRM service (http://localhost:5000)
/// for driving distance. Falls back to Haversine great-circle distance if OSRM fails.
#[utoipa::path(
    post,
    path = "/distance-matrix",
    request_body = DistanceMatrixRequest,
    responses(
        (status = 200, description = "Distance matrix", body = DistanceMatrixResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    )
)]
pub async fn distance_matrix(
    State(_state): State<AppState>,
    Json(req): Json<DistanceMatrixRequest>,
) -> Result<Json<DistanceMatrixResponse>, (StatusCode, String)> {
    let rows = req.origins.len();
    let cols = req.destinations.len();

    if rows == 0 || cols == 0 {
        return Err((StatusCode::BAD_REQUEST, "origins and destinations must be non-empty".into()));
    }

    // Build a reqwest client with a short timeout for OSRM.
    // If OSRM is unreachable, each request fails fast (~2s) and we fall back to Haversine.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Compute distances. For each pair, try OSRM first, then fall back to Haversine.
    let mut distances: Vec<Vec<f64>> = Vec::with_capacity(rows);
    for origin in &req.origins {
        let mut row: Vec<f64> = Vec::with_capacity(cols);
        for dest in &req.destinations {
            row.push(compute_distance(&client, origin, dest).await);
        }
        distances.push(row);
    }

    Ok(Json(DistanceMatrixResponse { distances }))
}

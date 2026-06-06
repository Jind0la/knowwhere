/// Distance matrix endpoint with OSRM + Haversine fallback.
/// Created during TDD RED phase — stub implementation.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::types::AppState;

/// Earth's mean radius in meters (WGS-84).
const EARTH_RADIUS_M: f64 = 6_371_000.0;

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

    // Build distance matrix using Haversine for each pair (OSRM integration in GREEN phase).
    let mut distances: Vec<Vec<f64>> = Vec::with_capacity(rows);
    for origin in &req.origins {
        let mut row: Vec<f64> = Vec::with_capacity(cols);
        for dest in &req.destinations {
            row.push(haversine_distance(origin.lat, origin.lng, dest.lat, dest.lng));
        }
        distances.push(row);
    }

    Ok(Json(DistanceMatrixResponse { distances }))
}

//! axum HTTP server with SSE live feed and REST endpoints.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::event::{Alternative, DetectionEvent, SpeciesInfo};
use sitta_store::db::Database;
use sitta_store::models::uuid_from_blob;

/// Shared state for all axum handlers.
#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    /// Clone of the broadcast sender — call `.subscribe()` per SSE client.
    pub detection_tx: broadcast::Sender<DetectionEvent>,
    pub station_name: String,
}

/// Build the axum router with all routes.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/stream/events", get(sse_handler))
        .route("/api/v1/detections", get(list_detections))
        .route("/api/v1/detections/{id}", get(get_detection))
        .route("/api/v1/species", get(list_species))
        .route("/api/v1/status", get(status_handler))
        .with_state(state)
}

/// Bind and serve until the shutdown token is cancelled.
pub async fn serve(addr: SocketAddr, state: ApiState, shutdown: CancellationToken) {
    let app = router(state);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(addr = %addr, error = %e, "Failed to bind API server");
            return;
        }
    };
    tracing::info!(%addr, "API server listening");
    let _ = axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await;
}

// ── SSE live feed ───────────────────────────────────────────────

async fn sse_handler(
    State(state): State<ApiState>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.detection_tx.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok::<_, Infallible>(
                            Event::default().event("detection").data(json)
                        );
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(dropped = n, "SSE client lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ── REST: detections ────────────────────────────────────────────

#[derive(Deserialize)]
struct ListParams {
    /// Start of range (Unix ms). Default: 24 hours ago.
    since: Option<i64>,
    /// End of range (Unix ms). Default: now.
    until: Option<i64>,
    /// Filter by scientific name.
    species: Option<String>,
    /// Max results (default 50, max 500).
    limit: Option<i64>,
    /// Pagination offset.
    offset: Option<i64>,
}

async fn list_detections(
    State(state): State<ApiState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<DetectionSummary>>, StatusCode> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);
    let limit = params.limit.unwrap_or(50).min(500);
    let offset = params.offset.unwrap_or(0);

    let rows = state
        .db
        .recent_detections(since, until, limit, offset, params.species.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query detections");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let detections: Vec<DetectionSummary> = rows.into_iter().filter_map(|r| {
        Some(DetectionSummary {
            id: uuid_from_blob(r.id).ok()?.to_string(),
            detected_at: millis_to_rfc3339(r.detected_at)?,
            source_name: r.source_name,
            model: r.model_name,
            model_version: r.model_version,
            species: SpeciesInfo {
                scientific_name: r.scientific_name.unwrap_or_default(),
                common_name: r.common_name,
                taxon_code: r.taxon_code,
            },
            confidence: r.confidence as f32,
        })
    }).collect();

    Ok(Json(detections))
}

async fn get_detection(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<DetectionDetail>, StatusCode> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let id_bytes = uuid.as_bytes().as_slice();

    let row = state
        .db
        .get_detection(id_bytes)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query detection");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let predictions = state
        .db
        .get_predictions(id_bytes)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query predictions");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let alternatives: Vec<Alternative> = predictions
        .into_iter()
        .map(|p| Alternative {
            rank: p.rank as u32,
            scientific_name: p.scientific_name.unwrap_or_default(),
            common_name: p.common_name,
            confidence: p.confidence as f32,
        })
        .collect();

    let detail = DetectionDetail {
        id: uuid.to_string(),
        detected_at: millis_to_rfc3339(row.detected_at).unwrap_or_default(),
        source_name: row.source_name,
        model: row.model_name,
        model_version: row.model_version,
        species: SpeciesInfo {
            scientific_name: row.scientific_name.unwrap_or_default(),
            common_name: row.common_name,
            taxon_code: row.taxon_code,
        },
        confidence: row.confidence as f32,
        alternatives,
        snippet_path: row.snippet_path,
        metadata: row.metadata,
    };

    Ok(Json(detail))
}

// ── REST: species ───────────────────────────────────────────────

#[derive(Deserialize)]
struct SpeciesParams {
    since: Option<i64>,
    until: Option<i64>,
}

async fn list_species(
    State(state): State<ApiState>,
    Query(params): Query<SpeciesParams>,
) -> Result<Json<Vec<SpeciesSummary>>, StatusCode> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);

    let rows = state
        .db
        .species_summary(since, until)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query species summary");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let species: Vec<SpeciesSummary> = rows.into_iter().filter_map(|r| {
        Some(SpeciesSummary {
            scientific_name: r.scientific_name.unwrap_or_default(),
            common_name: r.common_name,
            taxon_code: r.taxon_code,
            detection_count: r.detection_count,
            last_detected_at: millis_to_rfc3339(r.last_detected_at)?,
            avg_confidence: r.avg_confidence,
        })
    }).collect();

    Ok(Json(species))
}

// ── REST: status ────────────────────────────────────────────────

async fn status_handler(State(state): State<ApiState>) -> Json<StatusResponse> {
    let detection_count = state.db.detection_count().await.unwrap_or(-1);
    Json(StatusResponse {
        station_name: state.station_name.clone(),
        status: "running",
        detection_count,
    })
}

// ── Response types ──────────────────────────────────────────────

#[derive(Serialize)]
struct DetectionSummary {
    id: String,
    detected_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_name: Option<String>,
    model: String,
    model_version: String,
    species: SpeciesInfo,
    confidence: f32,
}

#[derive(Serialize)]
struct DetectionDetail {
    id: String,
    detected_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_name: Option<String>,
    model: String,
    model_version: String,
    species: SpeciesInfo,
    confidence: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    alternatives: Vec<Alternative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<String>,
}

#[derive(Serialize)]
struct SpeciesSummary {
    scientific_name: String,
    common_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    taxon_code: Option<String>,
    detection_count: i64,
    last_detected_at: String,
    avg_confidence: f64,
}

#[derive(Serialize)]
struct StatusResponse {
    station_name: String,
    status: &'static str,
    detection_count: i64,
}

// ── Helpers ─────────────────────────────────────────────────────

fn millis_to_rfc3339(ms: i64) -> Option<String> {
    DateTime::from_timestamp_millis(ms).map(|dt: DateTime<Utc>| dt.to_rfc3339())
}

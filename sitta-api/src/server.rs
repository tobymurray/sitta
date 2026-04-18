//! axum HTTP server with SSE live feed and REST endpoints.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
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

use crate::dashboard;
use crate::event::{Alternative, DetectionEvent, SpeciesInfo};
use crate::settings::{
    self, InitialConfig, RuntimeSettings, SettingsResponse, SettingsUpdate,
    RESTART_REQUIRED_FIELDS,
};
use sitta_store::db::Database;
use sitta_store::models::uuid_from_blob;

/// Shared state for all axum handlers.
#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    /// Clone of the broadcast sender — call `.subscribe()` per SSE client.
    pub detection_tx: broadcast::Sender<DetectionEvent>,
    /// Lock-free runtime settings.
    pub settings: Arc<ArcSwap<RuntimeSettings>>,
    /// Notify consumers when settings change so they can rebuild classifiers.
    pub settings_notify: Arc<tokio::sync::watch::Sender<()>>,
    /// Path to config.toml for persisting changes.
    pub config_path: PathBuf,
    /// Read-only snapshot of restart-required values.
    pub initial_config: Arc<InitialConfig>,
    /// Pipeline metrics (chunks processed/dropped per consumer).
    pub metrics: Arc<PipelineMetrics>,
    /// Individual matcher for enrollment reload. None if no Perch configured.
    pub matcher: Option<Arc<sitta_store::matcher::IndividualMatcher>>,
}

/// Pipeline metrics tracked via atomic counters.
/// Shared between inference consumers and the API.
#[derive(Default)]
pub struct PipelineMetrics {
    pub birdnet_chunks_processed: AtomicU64,
    pub birdnet_chunks_dropped: AtomicU64,
    pub perch_chunks_processed: AtomicU64,
    pub perch_chunks_dropped: AtomicU64,
}

/// Build the axum router with all routes.
pub fn router(state: ApiState) -> Router {
    Router::new()
        // API endpoints
        .route("/api/v1/stream/events", get(sse_handler))
        .route("/api/v1/detections", get(list_detections))
        .route("/api/v1/detections/{id}", get(get_detection))
        .route("/api/v1/species", get(list_species))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/individuals", get(list_individuals).post(enroll_individual))
        .route("/api/v1/individuals/{id}", get(get_individual))
        // Dashboard pages
        .route("/", get(dashboard_page))
        .route("/species", get(species_page))
        .route("/status", get(status_page))
        .route("/individuals", get(individuals_page))
        .route("/settings", get(settings_page))
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

    let display_conf = f64::from(state.settings.load().display_min_confidence);
    let rows = state
        .db
        .recent_detections(since, until, limit, offset, params.species.as_deref(), Some(display_conf))
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
            has_embedding: r.has_embedding,
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

    let display_conf = f64::from(state.settings.load().display_min_confidence);
    let rows = state
        .db
        .species_summary(since, until, Some(display_conf))
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
    let s = state.settings.load();
    let m = &state.metrics;
    Json(StatusResponse {
        station_name: s.station_name.clone(),
        status: "running",
        detection_count,
        pipeline: PipelineStatus {
            birdnet_chunks_processed: m.birdnet_chunks_processed.load(Ordering::Relaxed),
            birdnet_chunks_dropped: m.birdnet_chunks_dropped.load(Ordering::Relaxed),
            perch_chunks_processed: m.perch_chunks_processed.load(Ordering::Relaxed),
            perch_chunks_dropped: m.perch_chunks_dropped.load(Ordering::Relaxed),
        },
    })
}

// ── REST: settings ──────────────────────────────────────────────

async fn get_settings(State(state): State<ApiState>) -> Json<SettingsResponse> {
    let runtime = (**state.settings.load()).clone();
    Json(SettingsResponse {
        runtime,
        initial: (*state.initial_config).clone(),
        restart_required: RESTART_REQUIRED_FIELDS.to_vec(),
    })
}

async fn update_settings(
    State(state): State<ApiState>,
    Json(update): Json<SettingsUpdate>,
) -> Result<Json<UpdateResponse>, (StatusCode, String)> {
    let current = state.settings.load();
    let (merged, changed) = settings::apply_update(&current, &update);

    if changed.is_empty() {
        return Ok(Json(UpdateResponse {
            updated: vec![],
            rebuild_triggered: false,
            persist_error: None,
        }));
    }

    // Persist to disk first (best-effort).
    let persist_error = settings::persist_to_toml(&state.config_path, &merged).err();
    if let Some(ref e) = persist_error {
        tracing::warn!(error = %e, "Settings applied in memory but failed to persist to disk");
    }

    // Check if inference rebuild is needed.
    let rebuild = changed.iter().any(|f| {
        matches!(
            *f,
            "birdnet_min_confidence"
                | "birdnet_top_k"
                | "birdnet_meta_threshold"
                | "birdnet_force_allow"
                | "perch_min_confidence"
                | "perch_top_k"
                | "station_latitude"
                | "station_longitude"
        )
    });

    // Swap settings atomically.
    state.settings.store(Arc::new(merged));

    // Notify consumers.
    if rebuild {
        let _ = state.settings_notify.send(());
    }

    tracing::info!(?changed, rebuild, "Settings updated");

    Ok(Json(UpdateResponse {
        updated: changed.iter().map(|s| s.to_string()).collect(),
        rebuild_triggered: rebuild,
        persist_error,
    }))
}

#[derive(Serialize)]
struct UpdateResponse {
    updated: Vec<String>,
    rebuild_triggered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    persist_error: Option<String>,
}

// ── REST: individuals ───────────────────────────────────────────

async fn list_individuals(
    State(state): State<ApiState>,
    Query(params): Query<IndividualParams>,
) -> Result<Json<Vec<IndividualSummary>>, StatusCode> {
    let rows = state
        .db
        .list_individuals(params.species.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to list individuals");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let individuals = rows
        .into_iter()
        .filter_map(|r| {
            Some(IndividualSummary {
                id: uuid_from_blob(r.id).ok()?.to_string(),
                scientific_name: r.scientific_name,
                label: r.label,
                enrolled_at: millis_to_rfc3339(r.enrolled_at)?,
                notes: r.notes,
            })
        })
        .collect();

    Ok(Json(individuals))
}

#[derive(Deserialize)]
struct IndividualParams {
    species: Option<String>,
}

async fn get_individual(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<IndividualSummary>, StatusCode> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let row = state
        .db
        .get_individual(uuid.as_bytes().as_slice())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to get individual");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(IndividualSummary {
        id: uuid.to_string(),
        scientific_name: row.scientific_name,
        label: row.label,
        enrolled_at: millis_to_rfc3339(row.enrolled_at).unwrap_or_default(),
        notes: row.notes,
    }))
}

async fn enroll_individual(
    State(state): State<ApiState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<IndividualSummary>, (StatusCode, String)> {
    let det_uuid = req
        .detection_id
        .parse::<uuid::Uuid>()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid detection_id".into()))?;

    // Fetch the detection to get the species.
    let det = state
        .db
        .get_detection(det_uuid.as_bytes().as_slice())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "detection not found".into()))?;

    // Fetch the embedding for this detection.
    let emb_blob = state
        .db
        .get_embedding_for_detection(det_uuid.as_bytes().as_slice())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "detection has no embedding (only Perch detections have embeddings)".into(),
            )
        })?;

    let individual_id = uuid::Uuid::now_v7();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let dim = (emb_blob.len() / 4) as i64;
    let scientific_name = det.scientific_name.unwrap_or_default();

    state
        .db
        .insert_individual(&sitta_store::models::NewIndividual {
            id: &individual_id,
            scientific_name: &scientific_name,
            label: &req.label,
            reference_embedding: Some(&emb_blob),
            reference_embedding_dim: Some(dim),
            enrolled_at: now_ms,
            notes: req.notes.as_deref(),
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Reload the matcher cache so future detections see this individual.
    if let Some(matcher) = &state.matcher
        && let Err(e) = matcher.reload().await
    {
        tracing::warn!(error = %e, "Failed to reload matcher after enrollment");
    }

    tracing::info!(
        individual = %individual_id,
        label = %req.label,
        species = %scientific_name,
        "Individual enrolled"
    );

    Ok(Json(IndividualSummary {
        id: individual_id.to_string(),
        scientific_name,
        label: req.label,
        enrolled_at: millis_to_rfc3339(now_ms).unwrap_or_default(),
        notes: req.notes,
    }))
}

#[derive(Deserialize)]
struct EnrollRequest {
    detection_id: String,
    label: String,
    notes: Option<String>,
}

#[derive(Serialize)]
struct IndividualSummary {
    id: String,
    scientific_name: String,
    label: String,
    enrolled_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
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
    has_embedding: bool,
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
    pipeline: PipelineStatus,
}

#[derive(Serialize)]
struct PipelineStatus {
    birdnet_chunks_processed: u64,
    birdnet_chunks_dropped: u64,
    perch_chunks_processed: u64,
    perch_chunks_dropped: u64,
}

// ── Helpers ─────────────────────────────────────────────────────

fn millis_to_rfc3339(ms: i64) -> Option<String> {
    DateTime::from_timestamp_millis(ms).map(|dt: DateTime<Utc>| dt.to_rfc3339())
}

// ── Dashboard pages ─────────────────────────────────────────────

async fn dashboard_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.settings.load();
    let content = dashboard::dashboard_content(&s.station_name);
    dashboard::page("Dashboard", "dashboard", &content, &s.timezone)
}

async fn species_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.settings.load();
    let content = dashboard::species_content();
    dashboard::page("Species", "species", &content, &s.timezone)
}

async fn status_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.settings.load();
    let content = dashboard::status_content(&s.station_name);
    dashboard::page("Status", "status", &content, &s.timezone)
}

async fn individuals_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.settings.load();
    let content = dashboard::individuals_content();
    dashboard::page("Individuals", "individuals", &content, &s.timezone)
}

async fn settings_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.settings.load();
    let content = dashboard::settings_content(&s, &state.initial_config);
    dashboard::page("Settings", "settings", &content, &s.timezone)
}

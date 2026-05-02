//! axum HTTP server with SSE live feed and REST endpoints.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use arc_swap::ArcSwap;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::{delete, get};
use axum::Router;
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::dashboard;
use crate::event::{Alternative, DetectionEvent, RarityInfo, SpeciesInfo};
use crate::settings::{
    self, InitialConfig, RuntimeSettings, SettingsResponse, SettingsUpdate,
    RESTART_REQUIRED_FIELDS,
};
use sitta_store::db::Database;
use sitta_store::models::uuid_from_blob;

/// Unified API error type. Logs the error and returns a JSON body.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!(error = %e, "API error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}

impl From<sitta_store::StoreError> for ApiError {
    fn from(e: sitta_store::StoreError) -> Self {
        Self::internal(e)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::internal(e)
    }
}

/// Shared state for all axum handlers, grouped by concern.
#[derive(Clone)]
pub struct ApiState {
    /// Core: database, settings, config persistence.
    pub core: CoreState,
    /// Audio: broadcast channel, source management.
    pub audio: AudioState,
    /// Inference: detection broadcast, matcher, pipeline metrics.
    pub inference: InferenceState,
    /// Integrations: MQTT, audio clips.
    pub integrations: IntegrationState,
}

/// Database, settings, and config persistence.
#[derive(Clone)]
pub struct CoreState {
    pub db: Database,
    pub settings: Arc<ArcSwap<RuntimeSettings>>,
    pub settings_notify: Arc<tokio::sync::watch::Sender<()>>,
    pub config_path: PathBuf,
    pub initial_config: Arc<InitialConfig>,
}

/// Audio streaming and source lifecycle.
#[derive(Clone)]
pub struct AudioState {
    pub audio_tx: broadcast::Sender<Arc<sitta_audio::chunk::AudioChunk>>,
    pub source_manager: sitta_audio::manager::SourceManager,
}

/// Callback that looks up a species' location score from the BirdNET meta-model.
pub type RangeScoreFn = Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>;

/// Inference pipeline: detections, matching, metrics.
#[derive(Clone)]
pub struct InferenceState {
    pub detection_tx: broadcast::Sender<DetectionEvent>,
    pub matcher: Option<Arc<sitta_store::matcher::IndividualMatcher>>,
    pub metrics: Arc<PipelineMetrics>,
    /// Look up today's BirdNET meta-model location score for a species.
    /// None when no range filter is configured.
    pub range_scorer: Option<RangeScoreFn>,
}

/// External integrations: MQTT, audio clips.
#[derive(Clone)]
pub struct IntegrationState {
    pub mqtt_control: Option<Arc<dyn MqttControl>>,
    pub clip_dir: Option<PathBuf>,
    /// Snippet writer metrics. `None` when snippet saving is disabled.
    pub snippet_metrics: Option<Arc<SnippetMetrics>>,
    /// Snippet retention configuration (for diagnostics).
    pub snippet_retention: Option<SnippetRetention>,
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

/// Snippet writer metrics tracked via atomic counters.
/// Owned by the snippet writer in `sitta-bin`; surfaced via the API.
#[derive(Default)]
pub struct SnippetMetrics {
    pub clips_saved: AtomicU64,
    pub clips_dropped: AtomicU64,
    pub bytes_written: AtomicU64,
}

/// Snippet retention configuration snapshot for diagnostics.
#[derive(Clone, Copy)]
pub struct SnippetRetention {
    pub retention_days: u32,
    pub max_disk_mb: u64,
}

/// Build the axum router with all routes.
/// Trait for controlling the MQTT publisher from API handlers.
/// Implemented in sitta-bin to avoid circular dependencies.
#[async_trait::async_trait]
pub trait MqttControl: Send + Sync {
    async fn start(&self, settings: &settings::MqttSettings);
    async fn stop(&self);
    async fn is_running(&self) -> bool;
    /// Attempt a test connection to the broker. Returns Ok(()) on success
    /// or an error message on failure. Does not affect the running publisher.
    async fn test_connection(&self, settings: &settings::MqttSettings) -> Result<(), String>;
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        // API endpoints
        .route("/api/v1/stream/events", get(sse_handler))
        .route("/api/v1/detections", get(list_detections))
        .route("/api/v1/detections/{id}", get(get_detection).delete(delete_detection_handler))
        .route("/api/v1/species", get(list_species))
        .route("/api/v1/activity/hourly", get(hourly_activity))
        .route("/api/v1/species/{name}/insights", get(species_insights))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/audio-health", get(audio_health_handler))
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/individuals", get(list_individuals).post(enroll_individual).delete(delete_all_individuals))
        .route("/api/v1/individuals/{id}", get(get_individual))
        .route("/api/v1/candidates", get(list_candidate_clusters))
        .route("/api/v1/candidates/{id}/enroll", axum::routing::post(enroll_cluster))
        .route("/api/v1/candidates/{id}/dismiss", axum::routing::post(dismiss_cluster))
        .route("/api/v1/mqtt", get(get_mqtt_config).put(put_mqtt_config))
        .route("/api/v1/mqtt/test", axum::routing::post(test_mqtt_connection))
        .route("/api/v1/effort", get(effort_handler))
        .route("/api/v1/sources", get(list_sources).post(add_source))
        .route("/api/v1/sources/{name}", delete(remove_source))
        .route("/api/v1/detections/{id}/audio", get(detection_audio_handler))
        .route("/api/v1/detections/{id}/spectrogram", get(detection_spectrogram_handler))
        .route("/api/v1/detections/{id}/review", get(get_review).put(put_review).delete(delete_review))
        .route("/api/v1/audio/sources", get(list_audio_sources))
        .route("/api/v1/audio/levels", get(audio_levels_handler))
        .route("/api/v1/audio/stream/{source_name}", get(audio_stream_handler))
        // Dashboard pages
        .route("/", get(dashboard_page))
        .route("/detections/{id}", get(detection_detail_page))
        .route("/species", get(species_page))
        .route("/species/{name}", get(species_detail_page))
        .route("/status", get(status_page))
        .route("/diagnostics", get(diagnostics_page))
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
    let mut rx = state.inference.detection_tx.subscribe();
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
) -> Result<Json<PaginatedDetections>, ApiError> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);
    let limit = params.limit.unwrap_or(50).min(500);
    let offset = params.offset.unwrap_or(0);

    // Request one extra to determine if more results exist.
    let display_conf = f64::from(state.core.settings.load().display_min_confidence);
    let rows = state
        .core.db
        .recent_detections(since, until, limit + 1, offset, params.species.as_deref(), Some(display_conf))
        .await
        ?;

    let has_more = rows.len() as i64 > limit;
    let mut detections: Vec<DetectionSummary> = rows.into_iter().take(limit as usize).filter_map(|r| {
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
            has_audio: r.snippet_path.is_some(),
            rarity: None,
            range_unverified: match r.range_status.as_deref() {
                Some("not_in_meta_model") => Some(true),
                Some("allowed") | Some("force_allowed") => Some(false),
                _ => None,
            },
        })
    }).collect();

    // Hide range-unverified detections if the setting is off.
    if !state.core.settings.load().show_range_unverified {
        detections.retain(|d| d.range_unverified != Some(true));
    }

    // Populate rarity scores for each detection.
    for det in &mut detections {
        if let Ok(uuid) = det.id.parse::<uuid::Uuid>()
            && let Ok(Some(r)) = state.core.db.get_rarity(uuid.as_bytes().as_slice()).await
        {
            det.rarity = Some(rarity_row_to_info(&r));
        }
    }

    Ok(Json(PaginatedDetections {
        items: detections,
        offset,
        limit,
        has_more,
    }))
}

async fn get_detection(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<DetectionDetail>, ApiError> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    let row = state
        .core.db
        .get_detection(id_bytes)
        .await
        ?
        .ok_or(ApiError::not_found("not found"))?;

    let predictions = state
        .core.db
        .get_predictions(id_bytes)
        .await
        ?;

    let alternatives: Vec<Alternative> = predictions
        .into_iter()
        .map(|p| Alternative {
            rank: p.rank as u32,
            scientific_name: p.scientific_name.unwrap_or_default(),
            common_name: p.common_name,
            confidence: p.confidence as f32,
        })
        .collect();

    // Fetch review status.
    let review = state.core.db.get_review(id_bytes).await
        .ok()
        .flatten()
        .map(|r| ReviewInfo {
            status: r.status,
            reviewed_at: millis_to_rfc3339(r.reviewed_at).unwrap_or_default(),
            comment: r.comment,
        });

    // Fetch correlated detections (other models, ±5s window).
    let correlated_rows = state.core.db
        .correlated_detections(id_bytes, row.detected_at, 10)
        .await
        .unwrap_or_default();

    let correlated: Vec<CorrelatedDetection> = correlated_rows
        .into_iter()
        .filter_map(|r| {
            Some(CorrelatedDetection {
                id: uuid_from_blob(r.id).ok()?.to_string(),
                model: r.model_name,
                model_version: r.model_version,
                species: SpeciesInfo {
                    scientific_name: r.scientific_name.unwrap_or_default(),
                    common_name: r.common_name,
                    taxon_code: r.taxon_code,
                },
                confidence: r.confidence as f32,
                has_audio: r.snippet_path.is_some(),
            })
        })
        .collect();

    let rarity = state.core.db.get_rarity(id_bytes).await
        .ok()
        .flatten()
        .map(|r| rarity_row_to_info(&r));

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
        has_audio: row.snippet_path.is_some(),
        snippet_path: row.snippet_path,
        metadata: row.metadata,
        review,
        correlated,
        rarity,
        range_unverified: match row.range_status.as_deref() {
            Some("not_in_meta_model") => Some(true),
            Some("allowed") | Some("force_allowed") => Some(false),
            _ => None,
        },
    };

    Ok(Json(detail))
}

async fn delete_detection_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    let snippet_path = state
        .core.db
        .delete_detection(id_bytes)
        .await?;

    // Clean up audio clip and cached spectrogram on disk.
    if let (Some(clip_dir), Some(rel_path)) = (&state.integrations.clip_dir, &snippet_path)
        && let Ok(clip_path) = safe_join(clip_dir, rel_path)
    {
        let _ = tokio::fs::remove_file(&clip_path).await;
        let spectrogram_path = clip_path.with_extension("png");
        let _ = tokio::fs::remove_file(&spectrogram_path).await;
    }

    tracing::info!(detection_id = %uuid, "Detection deleted");
    Ok(StatusCode::NO_CONTENT)
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
) -> Result<Json<Vec<SpeciesSummary>>, ApiError> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);

    // No confidence filter: show every species with any detection in the window.
    // Individual detection lists still respect display_min_confidence.
    let rows = state
        .core.db
        .species_summary(since, until, None)
        .await
        ?;

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

// ── REST: hourly activity ───────────────────────────────────────

#[derive(Deserialize)]
struct ActivityParams {
    /// Start of window (Unix ms). Default: start of today in UTC.
    since: Option<i64>,
    /// End of window (Unix ms). Default: since + 24h.
    until: Option<i64>,
}

async fn hourly_activity(
    State(state): State<ApiState>,
    Query(params): Query<ActivityParams>,
) -> Result<Json<HourlyActivityResponse>, ApiError> {
    let now = Utc::now();
    let since = params.since.unwrap_or_else(|| {
        now.date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    });
    let until = params.until.unwrap_or(since + 86_400_000);

    let display_conf = f64::from(state.core.settings.load().display_min_confidence);
    let rows = state
        .core.db
        .hourly_activity(since, until, Some(display_conf))
        .await
        ?;

    // Group flat rows into per-species hour arrays.
    let mut species_map: std::collections::BTreeMap<String, SpeciesActivity> =
        std::collections::BTreeMap::new();

    for row in rows {
        let key = row.scientific_name.clone().unwrap_or_default();
        let entry = species_map.entry(key).or_insert_with(|| SpeciesActivity {
            common_name: row.common_name.clone(),
            scientific_name: row.scientific_name.clone().unwrap_or_default(),
            taxon_code: row.taxon_code.clone(),
            total: 0,
            hours: vec![0; 24],
        });
        let h = row.hour_bucket as usize;
        if h < 24 {
            entry.hours[h] = row.count;
            entry.total += row.count;
        }
    }

    let mut species: Vec<SpeciesActivity> = species_map.into_values().collect();
    species.sort_by_key(|s| std::cmp::Reverse(s.total));

    Ok(Json(HourlyActivityResponse { since, until, species }))
}

#[derive(Serialize)]
struct HourlyActivityResponse {
    since: i64,
    until: i64,
    species: Vec<SpeciesActivity>,
}

#[derive(Serialize)]
struct SpeciesActivity {
    common_name: String,
    scientific_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    taxon_code: Option<String>,
    total: i64,
    hours: Vec<i64>,
}

// ── REST: species insights ──────────────────────────────────────

async fn species_insights(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<Json<SpeciesInsightsResponse>, ApiError> {
    let display_conf = f64::from(state.core.settings.load().display_min_confidence);

    let stats = state
        .core.db
        .species_stats(&name, Some(display_conf))
        .await
        ?
        .ok_or(ApiError::not_found("not found"))?;

    let profile_rows = state
        .core.db
        .species_hourly_profile(&name, Some(display_conf))
        .await
        ?;

    // Build a full 24-element array from sparse rows.
    let mut hourly_distribution = vec![0i64; 24];
    for row in profile_rows {
        let h = row.hour_utc as usize;
        if h < 24 {
            hourly_distribution[h] = row.count;
        }
    }

    // Monthly distribution (12 calendar months).
    let monthly_rows = state
        .core.db
        .species_monthly_distribution(&name, Some(display_conf))
        .await
        ?;
    let mut monthly_distribution = vec![0i64; 12];
    for row in monthly_rows {
        let m = row.month as usize;
        if (1..=12).contains(&m) {
            monthly_distribution[m - 1] = row.count;
        }
    }

    // Range score for today.
    let range_score = state
        .inference
        .range_scorer
        .as_ref()
        .and_then(|f| f(&name));

    // Notable detections (high rarity).
    let notable_rows = state
        .core.db
        .notable_detections(&name, 5, display_conf)
        .await
        .unwrap_or_default();
    let notable_detections: Vec<NotableDetection> = notable_rows
        .into_iter()
        .filter_map(|r| {
            Some(NotableDetection {
                detection_id: sitta_store::models::uuid_from_blob(r.detection_id).ok()?.to_string(),
                detected_at: millis_to_rfc3339(r.detected_at)?,
                confidence: r.confidence as f32,
                rarity_score: r.score as f32,
                first_ever: r.first_ever,
                first_season: r.first_season,
            })
        })
        .collect();

    // Today likelihood: how likely is it to see this species today at this station?
    let today_likelihood = compute_today_likelihood(
        &stats,
        &hourly_distribution,
        &monthly_distribution,
        range_score,
    );

    // Data sufficiency analysis.
    let data_sufficiency = compute_data_sufficiency(
        &stats,
        &hourly_distribution,
        &monthly_distribution,
    );

    let s = state.core.settings.load();

    Ok(Json(SpeciesInsightsResponse {
        scientific_name: name,
        common_name: stats.common_name,
        total_detections: stats.total,
        first_detected_at: millis_to_rfc3339(stats.first_detected_at).unwrap_or_default(),
        last_detected_at: millis_to_rfc3339(stats.last_detected_at).unwrap_or_default(),
        days_detected: stats.distinct_days,
        avg_confidence: stats.avg_confidence,
        hourly_distribution,
        monthly_distribution,
        range_score,
        today_likelihood,
        data_sufficiency,
        notable_detections,
        station_latitude: s.station_latitude,
        station_longitude: s.station_longitude,
    }))
}

/// Estimate how likely this species is to be detected today (0.0–1.0).
///
/// Combines four signals:
/// - Range score: meta-model's occurrence probability for this location + date
/// - Monthly frequency: how active is this month historically?
/// - Hourly coverage: what fraction of today's hours have historical detections?
/// - Detection consistency: what fraction of days has this species been seen?
fn compute_today_likelihood(
    stats: &sitta_store::models::SpeciesStatsRow,
    hourly: &[i64],
    monthly: &[i64],
    range_score: Option<f32>,
) -> f32 {
    if stats.total < 3 {
        // Too few detections for a meaningful prediction.
        return 0.0;
    }

    // Monthly signal: fraction of this month's detections vs peak month.
    let now = chrono::Utc::now();
    let this_month = now.month0() as usize;
    let peak_month = monthly.iter().copied().max().unwrap_or(1).max(1);
    let monthly_signal = monthly[this_month] as f32 / peak_month as f32;

    // Hourly signal: fraction of today's remaining hours that have activity.
    let current_hour = now.hour() as usize;
    let remaining_active: usize = hourly[current_hour..].iter().filter(|&&c| c > 0).count();
    let remaining_hours = (24 - current_hour).max(1);
    let hourly_signal = remaining_active as f32 / remaining_hours as f32;

    // Consistency signal: detection days / total days in observation window.
    let first_ms = stats.first_detected_at;
    let last_ms = stats.last_detected_at;
    let total_days = ((last_ms - first_ms) / 86_400_000).max(1);
    let consistency = (stats.distinct_days as f32 / total_days as f32).min(1.0);

    // Weight by availability of range score.
    match range_score {
        Some(rs) => rs * 0.35 + monthly_signal * 0.25 + hourly_signal * 0.15 + consistency * 0.25,
        None => monthly_signal * 0.35 + hourly_signal * 0.25 + consistency * 0.40,
    }
}

/// Identify gaps in the data and suggest what additional observations would help.
fn compute_data_sufficiency(
    stats: &sitta_store::models::SpeciesStatsRow,
    hourly: &[i64],
    monthly: &[i64],
) -> DataSufficiency {
    let mut gaps = Vec::new();

    if stats.total < 20 {
        gaps.push("Need more detections for reliable patterns (have %TOTAL%, want 20+).".to_string()
            .replace("%TOTAL%", &stats.total.to_string()));
    }

    // Check month coverage: how many months have detections?
    let months_with_data = monthly.iter().filter(|&&c| c > 0).count();
    if months_with_data < 4 && stats.distinct_days >= 7 {
        gaps.push(format!(
            "Only {} of 12 months have data \u{2014} check back as more seasons are observed.",
            months_with_data
        ));
    }

    // Check hour coverage: how many hours have detections?
    let hours_with_data = hourly.iter().filter(|&&c| c > 0).count();
    if hours_with_data <= 4 && stats.total >= 10 {
        gaps.push(format!(
            "Detections concentrated in {} of 24 hours \u{2014} activity pattern may be incomplete.",
            hours_with_data
        ));
    }

    // Observation span
    let first_ms = stats.first_detected_at;
    let last_ms = stats.last_detected_at;
    let span_days = (last_ms - first_ms) / 86_400_000;
    if span_days < 30 && stats.total >= 5 {
        gaps.push(format!(
            "Observation window is only {} days \u{2014} seasonal patterns need longer history.",
            span_days
        ));
    }

    DataSufficiency {
        total_detections: stats.total >= 20,
        seasonal_coverage: months_with_data >= 4,
        hourly_coverage: hours_with_data >= 6,
        observation_span_days: span_days,
        gaps,
    }
}

#[derive(Serialize)]
struct SpeciesInsightsResponse {
    scientific_name: String,
    common_name: String,
    total_detections: i64,
    first_detected_at: String,
    last_detected_at: String,
    days_detected: i64,
    avg_confidence: f64,
    /// 24 elements, indexed by UTC hour (0-23).
    hourly_distribution: Vec<i64>,
    /// 12 elements, indexed by calendar month (0=Jan, 11=Dec).
    monthly_distribution: Vec<i64>,
    /// BirdNET meta-model location score for today (0.0–1.0). None if no range filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    range_score: Option<f32>,
    /// Estimated likelihood of detecting this species today (0.0–1.0).
    today_likelihood: f32,
    /// Data sufficiency analysis with gap descriptions.
    data_sufficiency: DataSufficiency,
    /// Recent notable (high-rarity) detections.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notable_detections: Vec<NotableDetection>,
    /// Station coordinates for sunrise/sunset calculation.
    #[serde(skip_serializing_if = "Option::is_none")]
    station_latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    station_longitude: Option<f64>,
}

#[derive(Serialize)]
struct DataSufficiency {
    total_detections: bool,
    seasonal_coverage: bool,
    hourly_coverage: bool,
    observation_span_days: i64,
    /// Human-readable descriptions of what data is missing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    gaps: Vec<String>,
}

#[derive(Serialize)]
struct NotableDetection {
    detection_id: String,
    detected_at: String,
    confidence: f32,
    rarity_score: f32,
    first_ever: bool,
    first_season: bool,
}

// ── REST: audio health ──────────────────────────────────────────

#[derive(Serialize)]
struct AudioHealthResponse {
    /// Whether snippet saving is enabled in config.
    enabled: bool,
    /// Path of the clip directory (if enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    clip_dir: Option<String>,
    /// Snippet writer counters since process start.
    metrics: AudioHealthMetrics,
    /// Retention configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    retention: Option<AudioHealthRetention>,
    /// All-time totals: detections vs detections with a saved clip.
    totals: AudioHealthTotalsView,
    /// Daily breakdown for the requested window. Most recent day first.
    daily: Vec<AudioHealthDay>,
    /// Window start for the daily breakdown (Unix ms).
    window_since_ms: i64,
}

#[derive(Serialize, Default)]
struct AudioHealthMetrics {
    clips_saved: u64,
    clips_dropped: u64,
    bytes_written: u64,
}

#[derive(Serialize)]
struct AudioHealthRetention {
    retention_days: u32,
    max_disk_mb: u64,
}

#[derive(Serialize)]
struct AudioHealthTotalsView {
    total: i64,
    with_clip: i64,
    without_clip: i64,
}

#[derive(Serialize)]
struct AudioHealthDay {
    day: String,
    total: i64,
    with_clip: i64,
    without_clip: i64,
}

#[derive(Deserialize)]
struct AudioHealthParams {
    /// Days to include in the daily breakdown. Default 30, clamped to [1, 365].
    days: Option<u32>,
}

async fn audio_health_handler(
    State(state): State<ApiState>,
    Query(params): Query<AudioHealthParams>,
) -> Result<Json<AudioHealthResponse>, ApiError> {
    let days = params.days.unwrap_or(30).clamp(1, 365);
    let since_ms = Utc::now().timestamp_millis() - i64::from(days) * 86_400_000;

    let totals = state
        .core
        .db
        .audio_health_totals()
        .await
        .map_err(ApiError::internal)?;
    let daily_rows = state
        .core
        .db
        .daily_audio_health(since_ms)
        .await
        .map_err(ApiError::internal)?;

    let metrics = state
        .integrations
        .snippet_metrics
        .as_ref()
        .map(|m| AudioHealthMetrics {
            clips_saved: m.clips_saved.load(Ordering::Relaxed),
            clips_dropped: m.clips_dropped.load(Ordering::Relaxed),
            bytes_written: m.bytes_written.load(Ordering::Relaxed),
        })
        .unwrap_or_default();

    let retention = state
        .integrations
        .snippet_retention
        .map(|r| AudioHealthRetention {
            retention_days: r.retention_days,
            max_disk_mb: r.max_disk_mb,
        });

    let clip_dir = state
        .integrations
        .clip_dir
        .as_ref()
        .map(|p| p.display().to_string());

    let enabled = state.integrations.snippet_metrics.is_some();

    let daily = daily_rows
        .into_iter()
        .map(|d| AudioHealthDay {
            day: d.day,
            total: d.total,
            with_clip: d.with_clip,
            without_clip: d.total - d.with_clip,
        })
        .collect();

    Ok(Json(AudioHealthResponse {
        enabled,
        clip_dir,
        metrics,
        retention,
        totals: AudioHealthTotalsView {
            total: totals.total,
            with_clip: totals.with_clip,
            without_clip: totals.total - totals.with_clip,
        },
        daily,
        window_since_ms: since_ms,
    }))
}

// ── REST: status ────────────────────────────────────────────────

async fn status_handler(State(state): State<ApiState>) -> Json<StatusResponse> {
    let detection_count = state.core.db.detection_count().await.unwrap_or(-1);
    let s = state.core.settings.load();
    let m = &state.inference.metrics;

    let active_sources = state
        .core
        .db
        .active_sessions()
        .await
        .map(|sessions| sessions.into_iter().map(|s| s.source_name).collect())
        .unwrap_or_default();

    Json(StatusResponse {
        station_name: s.station_name.clone(),
        status: "running",
        detection_count,
        active_sources,
        pipeline: PipelineStatus {
            birdnet_chunks_processed: m.birdnet_chunks_processed.load(Ordering::Relaxed),
            birdnet_chunks_dropped: m.birdnet_chunks_dropped.load(Ordering::Relaxed),
            perch_chunks_processed: m.perch_chunks_processed.load(Ordering::Relaxed),
            perch_chunks_dropped: m.perch_chunks_dropped.load(Ordering::Relaxed),
        },
    })
}

// ── REST: effort tracking ──────────────────────────────────────

#[derive(Deserialize)]
struct EffortParams {
    /// Start of range (Unix ms). Default: 24 hours ago.
    since: Option<i64>,
    /// End of range (Unix ms). Default: now.
    until: Option<i64>,
}

async fn effort_handler(
    State(state): State<ApiState>,
    Query(params): Query<EffortParams>,
) -> Result<Json<EffortResponse>, ApiError> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);
    let window_seconds = (until - since) as f64 / 1000.0;

    let source_rows = state.core.db.effort_summary(since, until).await?;
    let total_seconds = state.core.db.total_effort_seconds(since, until).await?;

    let sources: Vec<SourceEffort> = source_rows
        .into_iter()
        .map(|r| {
            let coverage = if window_seconds > 0.0 {
                (r.total_seconds / window_seconds).min(1.0)
            } else {
                0.0
            };
            SourceEffort {
                source_name: r.source_name,
                total_seconds: r.total_seconds,
                session_count: r.session_count,
                coverage,
            }
        })
        .collect();

    let active_sessions = state.core.db.active_sessions().await?;
    let active: Vec<ActiveSession> = active_sessions
        .into_iter()
        .filter_map(|s| {
            Some(ActiveSession {
                source_name: s.source_name,
                started_at: millis_to_rfc3339(s.started_at)?,
                chunks_received: s.chunks_received,
                duration_seconds: (now - s.started_at) as f64 / 1000.0,
            })
        })
        .collect();

    // Overall coverage: fraction of the window with at least one source recording.
    // Approximate as total_seconds / window_seconds, capped at 1.0.
    let overall_coverage = if window_seconds > 0.0 {
        (total_seconds / window_seconds).min(1.0)
    } else {
        0.0
    };

    Ok(Json(EffortResponse {
        since: millis_to_rfc3339(since).unwrap_or_default(),
        until: millis_to_rfc3339(until).unwrap_or_default(),
        total_recording_seconds: total_seconds,
        overall_coverage,
        sources,
        active_sessions: active,
    }))
}

#[derive(Serialize)]
struct EffortResponse {
    since: String,
    until: String,
    /// Total seconds of audio recording across all sources in the window.
    total_recording_seconds: f64,
    /// Fraction of the time window covered by at least one source (0.0–1.0).
    overall_coverage: f64,
    /// Per-source breakdown.
    sources: Vec<SourceEffort>,
    /// Currently active recording sessions.
    active_sessions: Vec<ActiveSession>,
}

#[derive(Serialize)]
struct SourceEffort {
    source_name: String,
    total_seconds: f64,
    session_count: i64,
    /// Fraction of the time window this source was recording (0.0–1.0).
    coverage: f64,
}

#[derive(Serialize)]
struct ActiveSession {
    source_name: String,
    started_at: String,
    chunks_received: i64,
    duration_seconds: f64,
}

// ── REST: settings ──────────────────────────────────────────────

async fn get_settings(State(state): State<ApiState>) -> Json<SettingsResponse> {
    let runtime = (**state.core.settings.load()).clone();
    Json(SettingsResponse {
        runtime,
        initial: (*state.core.initial_config).clone(),
        restart_required: RESTART_REQUIRED_FIELDS.to_vec(),
    })
}

async fn update_settings(
    State(state): State<ApiState>,
    Json(update): Json<SettingsUpdate>,
) -> Result<Json<UpdateResponse>, ApiError> {
    let current = state.core.settings.load();
    let (merged, changed) = settings::apply_update(&current, &update);

    if changed.is_empty() {
        return Ok(Json(UpdateResponse {
            updated: vec![],
            rebuild_triggered: false,
            persist_error: None,
        }));
    }

    // Persist to disk first (best-effort).
    let persist_error = settings::persist_to_toml(&state.core.config_path, &merged).err();
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
    state.core.settings.store(Arc::new(merged));

    // Notify consumers.
    if rebuild {
        let _ = state.core.settings_notify.send(());
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
) -> Result<Json<Vec<IndividualSummary>>, ApiError> {
    let rows = state
        .core.db
        .list_individuals(params.species.as_deref())
        .await
        ?;

    let individuals = rows
        .into_iter()
        .filter_map(|r| {
            Some(IndividualSummary {
                id: uuid_from_blob(r.id).ok()?.to_string(),
                scientific_name: r.scientific_name,
                common_name: r.common_name,
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
) -> Result<Json<IndividualSummary>, ApiError> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let row = state
        .core.db
        .get_individual(uuid.as_bytes().as_slice())
        .await
        ?
        .ok_or(ApiError::not_found("not found"))?;

    Ok(Json(IndividualSummary {
        id: uuid.to_string(),
        scientific_name: row.scientific_name,
        common_name: row.common_name,
        label: row.label,
        enrolled_at: millis_to_rfc3339(row.enrolled_at).unwrap_or_default(),
        notes: row.notes,
    }))
}

async fn delete_all_individuals(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let deleted = state.core.db.delete_all_individuals().await?;

    // Reload matcher to clear the in-memory cache.
    if let Some(matcher) = &state.inference.matcher
        && let Err(e) = matcher.reload().await
    {
        tracing::warn!(error = %e, "Failed to reload matcher after bulk delete");
    }

    tracing::info!(deleted, "Deleted all individuals");
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

async fn enroll_individual(
    State(state): State<ApiState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<IndividualSummary>, ApiError> {
    let det_uuid = req
        .detection_id
        .parse::<uuid::Uuid>()
        .map_err(|_| ApiError::bad_request("invalid detection_id"))?;

    // Fetch the detection to get the species.
    let det = state
        .core.db
        .get_detection(det_uuid.as_bytes().as_slice())
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("detection not found"))?;

    // Fetch the embedding for this detection.
    let emb_blob = state
        .core.db
        .get_embedding_for_detection(det_uuid.as_bytes().as_slice())
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::bad_request("detection has no embedding (only Perch detections have embeddings)"))?;

    let individual_id = uuid::Uuid::now_v7();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let dim = (emb_blob.len() / 4) as i64;
    let scientific_name = det.scientific_name.unwrap_or_default();
    let common_name = Some(det.common_name);

    state
        .core.db
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
        .map_err(ApiError::internal)?;

    // Reload the matcher cache so future detections see this individual.
    if let Some(matcher) = &state.inference.matcher
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
        common_name,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    common_name: Option<String>,
    label: String,
    enrolled_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

// ── Candidate clusters ─────────────────────────────────────────

async fn list_candidate_clusters(
    State(state): State<ApiState>,
) -> Result<Json<Vec<CandidateClusterSummary>>, ApiError> {
    let min_members = state.core.initial_config.min_cluster_size;
    let min_days = state.core.initial_config.min_distinct_days;

    let rows = state
        .core.db
        .ready_clusters(min_members, min_days)
        .await
        ?;

    let mut clusters: Vec<CandidateClusterSummary> = rows
        .into_iter()
        .map(|r| CandidateClusterSummary {
            id: r.id,
            scientific_name: r.scientific_name,
            common_name: None,
            member_count: r.member_count,
            distinct_days: r.distinct_days,
            first_seen_at: millis_to_rfc3339(r.first_seen_at).unwrap_or_default(),
            last_seen_at: millis_to_rfc3339(r.last_seen_at).unwrap_or_default(),
        })
        .collect();

    // Resolve common names from the labels table.
    for c in &mut clusters {
        if let Ok(name) = state.core.db.common_name_for(&c.scientific_name).await {
            c.common_name = name;
        }
    }

    Ok(Json(clusters))
}

async fn enroll_cluster(
    State(state): State<ApiState>,
    Path(cluster_id): Path<i64>,
    Json(req): Json<ClusterEnrollRequest>,
) -> Result<Json<IndividualSummary>, ApiError> {
    let cluster = state
        .core.db
        .get_cluster(cluster_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("cluster not found"))?;

    if cluster.status != "pending" {
        return Err(ApiError::conflict(format!("cluster is already {}", cluster.status)));
    }

    // Create individual from cluster centroid.
    let individual_id = uuid::Uuid::now_v7();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let dim = cluster.centroid_dim;

    state
        .core.db
        .insert_individual(&sitta_store::models::NewIndividual {
            id: &individual_id,
            scientific_name: &cluster.scientific_name,
            label: &req.label,
            reference_embedding: Some(&cluster.centroid),
            reference_embedding_dim: Some(dim),
            enrolled_at: now_ms,
            notes: req.notes.as_deref(),
        })
        .await
        .map_err(ApiError::internal)?;

    // Mark cluster as enrolled.
    state
        .core.db
        .enroll_cluster(cluster_id, &individual_id)
        .await
        .map_err(ApiError::internal)?;

    // Link cluster member detections to the new individual.
    let detection_ids = state
        .core.db
        .cluster_detection_ids(cluster_id)
        .await
        .map_err(ApiError::internal)?;
    for det_bytes in &detection_ids {
        if let Ok(det_uuid) = uuid_from_blob(det_bytes.clone()) {
            let match_id = uuid::Uuid::now_v7();
            // Use similarity 0.0 as a sentinel — these are founding members, not runtime matches.
            let _ = state
                .core.db
                .insert_individual_match(&match_id, &individual_id, &det_uuid, 1.0, now_ms)
                .await;
        }
    }

    // Reload matcher so future detections match against this individual.
    if let Some(matcher) = &state.inference.matcher
        && let Err(e) = matcher.reload().await
    {
        tracing::warn!(error = %e, "Failed to reload matcher after cluster enrollment");
    }

    tracing::info!(
        cluster_id,
        individual = %individual_id,
        label = %req.label,
        species = %cluster.scientific_name,
        members = detection_ids.len(),
        "Cluster enrolled as individual"
    );

    Ok(Json(IndividualSummary {
        id: individual_id.to_string(),
        scientific_name: cluster.scientific_name,
        common_name: None,
        label: req.label,
        enrolled_at: millis_to_rfc3339(now_ms).unwrap_or_default(),
        notes: req.notes,
    }))
}

async fn dismiss_cluster(
    State(state): State<ApiState>,
    Path(cluster_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let cluster = state
        .core.db
        .get_cluster(cluster_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("cluster not found"))?;

    if cluster.status != "pending" {
        return Err(ApiError::conflict(format!("cluster is already {}", cluster.status)));
    }

    state
        .core.db
        .dismiss_cluster(cluster_id)
        .await
        .map_err(ApiError::internal)?;

    tracing::info!(cluster_id, species = %cluster.scientific_name, "Cluster dismissed");

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CandidateClusterSummary {
    id: i64,
    scientific_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    common_name: Option<String>,
    member_count: i64,
    distinct_days: i64,
    first_seen_at: String,
    last_seen_at: String,
}

#[derive(Deserialize)]
struct ClusterEnrollRequest {
    label: String,
    notes: Option<String>,
}

// ── Audio rebroadcast ───────────────────────────────────────────

// ── MQTT config ─────────────────────────────────────────────────

async fn get_mqtt_config(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut mqtt = serde_json::to_value(settings::read_mqtt_from_toml(&state.core.config_path))
        .map_err(ApiError::internal)?;
    let running = if let Some(ref ctrl) = state.integrations.mqtt_control {
        ctrl.is_running().await
    } else {
        false
    };
    if let Some(obj) = mqtt.as_object_mut() {
        obj.insert("running".into(), running.into());
    }
    Ok(Json(mqtt))
}

async fn put_mqtt_config(
    State(state): State<ApiState>,
    Json(mqtt): Json<settings::MqttSettings>,
) -> Result<Json<settings::MqttSettings>, ApiError> {
    // Persist to config.toml.
    settings::persist_mqtt_to_toml(&state.core.config_path, &mqtt)
        .map_err(ApiError::internal)?;

    // Dynamically start or stop the MQTT publisher.
    if let Some(ref ctrl) = state.integrations.mqtt_control {
        if mqtt.enabled && !mqtt.host.is_empty() {
            ctrl.start(&mqtt).await;
        } else {
            ctrl.stop().await;
        }
    }

    Ok(Json(mqtt))
}

async fn test_mqtt_connection(
    State(state): State<ApiState>,
    Json(mqtt): Json<settings::MqttSettings>,
) -> Result<Json<MqttTestResult>, ApiError> {
    let ctrl = state
        .integrations
        .mqtt_control
        .as_ref()
        .ok_or(ApiError { status: StatusCode::SERVICE_UNAVAILABLE, message: "MQTT controller not available".into() })?;

    match ctrl.test_connection(&mqtt).await {
        Ok(()) => Ok(Json(MqttTestResult {
            success: true,
            message: format!("Connected to {}:{}", mqtt.host, mqtt.port),
        })),
        Err(e) => Ok(Json(MqttTestResult {
            success: false,
            message: e,
        })),
    }
}

#[derive(Serialize)]
struct MqttTestResult {
    success: bool,
    message: String,
}

// ── Source management ────────────────────────────────────────────

async fn list_sources(
    State(state): State<ApiState>,
) -> Json<Vec<SourceSummary>> {
    let configs = state.audio.source_manager.list().await;
    let summaries = configs
        .into_iter()
        .map(|c| {
            let (source_type, url) = match &c {
                sitta_audio::source::SourceConfig::Rtsp(r) => ("rtsp", Some(r.url.clone())),
                sitta_audio::source::SourceConfig::Local(l) => ("local", Some(l.device.clone())),
                sitta_audio::source::SourceConfig::Remote(r) => ("remote", Some(r.url.clone())),
            };
            SourceSummary {
                name: c.name().to_string(),
                source_type: source_type.to_string(),
                url,
            }
        })
        .collect();
    Json(summaries)
}

async fn add_source(
    State(state): State<ApiState>,
    body: String,
) -> Result<Json<SourceSummary>, ApiError> {
    let config: sitta_audio::source::SourceConfig = serde_json::from_str(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid source config: {e}")))?;
    let name = config.name().to_string();
    let (source_type, url) = match &config {
        sitta_audio::source::SourceConfig::Rtsp(r) => ("rtsp", Some(r.url.clone())),
        sitta_audio::source::SourceConfig::Local(l) => ("local", Some(l.device.clone())),
        sitta_audio::source::SourceConfig::Remote(r) => ("remote", Some(r.url.clone())),
    };

    state
        .audio.source_manager
        .add(config)
        .await
        .map_err(ApiError::conflict)?;

    // Persist to config.toml so sources survive restart.
    let all_sources = state.audio.source_manager.list().await;
    if let Err(e) = settings::persist_sources_to_toml(&state.core.config_path, &all_sources) {
        tracing::warn!(error = %e, "Source added but failed to persist to config");
    }

    Ok(Json(SourceSummary {
        name,
        source_type: source_type.to_string(),
        url,
    }))
}

async fn remove_source(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .audio.source_manager
        .remove(&name)
        .await
        .map_err(ApiError::not_found)?;

    let all_sources = state.audio.source_manager.list().await;
    if let Err(e) = settings::persist_sources_to_toml(&state.core.config_path, &all_sources) {
        tracing::warn!(error = %e, "Source removed but failed to persist to config");
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct SourceSummary {
    name: String,
    source_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

async fn audio_levels_handler(
    State(state): State<ApiState>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.audio.audio_tx.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(chunk) => {
                    let rms = chunk.rms();
                    let dbfs = chunk.rms_dbfs();
                    let peak = chunk.peak();
                    let json = serde_json::json!({
                        "source": chunk.source_name,
                        "rms": rms,
                        "rms_dbfs": dbfs,
                        "peak": peak,
                        "sample_rate": chunk.sample_rate,
                        "duration_secs": chunk.duration_secs(),
                    });
                    if let Ok(data) = serde_json::to_string(&json) {
                        yield Ok::<_, Infallible>(
                            Event::default().event("level").data(data)
                        );
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn list_audio_sources(
    State(state): State<ApiState>,
) -> Json<Vec<String>> {
    Json(state.audio.source_manager.names().await)
}

async fn audio_stream_handler(
    State(state): State<ApiState>,
    Path(source_name): Path<String>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::Response;

    if !state.audio.source_manager.contains(&source_name).await {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(format!("unknown source: {source_name}")))
            .expect("static response");
    }

    let mut rx = state.audio.audio_tx.subscribe();
    let header_source = source_name.clone();
    let stream = async_stream::stream! {
        let mut header_sent = false;

        loop {
            match rx.recv().await {
                Ok(chunk) => {
                    if chunk.source_name != source_name {
                        continue;
                    }

                    if !header_sent {
                        let header = sitta_audio::chunk::PcmStreamHeader {
                            sample_rate: chunk.sample_rate,
                            channels: chunk.channels,
                            _pad: 0,
                            chunk_samples: chunk.samples.len() as u32,
                            _reserved: [0; 8],
                        };
                        yield Ok::<_, Infallible>(Bytes::copy_from_slice(
                            bytemuck::bytes_of(&header)
                        ));
                        header_sent = true;
                    }

                    let sample_bytes: &[u8] = bytemuck::cast_slice(&chunk.samples);
                    yield Ok(Bytes::copy_from_slice(sample_bytes));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(dropped = n, source = %source_name, "Audio stream client lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Response::builder()
        .header("content-type", "application/octet-stream")
        .header("x-sitta-format", "f32le")
        .header("x-sitta-source", &header_source)
        .body(Body::from_stream(stream))
        .expect("static response")
}

// ── Path traversal guard ───────────────────────────────────────

/// Resolve `base.join(rel)` and verify the result stays within `base`.
/// Returns the canonical path on success, or a not-found error if the
/// path escapes (e.g., `../../etc/passwd`) or doesn't exist.
fn safe_join(base: &std::path::Path, rel: &str) -> Result<PathBuf, ApiError> {
    let full = base.join(rel);
    let canonical = full
        .canonicalize()
        .map_err(|_| ApiError::not_found("not found"))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| ApiError::not_found("not found"))?;
    if !canonical.starts_with(&base_canonical) {
        return Err(ApiError::not_found("not found"));
    }
    Ok(canonical)
}

// ── Audio clip serving ──────────────────────────────────────────

async fn detection_audio_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    use axum::body::Body;

    let clip_dir = state.integrations.clip_dir.as_ref().ok_or(ApiError::not_found("not found"))?;
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    let row = state
        .core.db
        .get_detection(id_bytes)
        .await
        ?
        .ok_or(ApiError::not_found("not found"))?;

    let rel_path = row.snippet_path.ok_or(ApiError::not_found("not found"))?;
    let full_path = safe_join(clip_dir, &rel_path)?;

    // If a .tmp file exists, the clip is still being written.
    let tmp_path = full_path.with_extension("wav.tmp");
    if tokio::fs::try_exists(&tmp_path).await.unwrap_or(false) {
        return axum::response::Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("retry-after", "1")
            .body(Body::empty())
            .map_err(ApiError::internal);
    }

    let data = tokio::fs::read(&full_path).await.map_err(|_| ApiError::not_found("file not found"))?;
    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "audio/wav")
        .header("accept-ranges", "bytes")
        .header("content-disposition", "inline")
        .header("cache-control", "public, max-age=31536000, immutable")
        .body(Body::from(data))
        .map_err(ApiError::internal)
}

// ── Spectrogram serving (on-demand with disk cache) ────────────

async fn detection_spectrogram_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    use axum::body::Body;
    use mel_spec_png::{generate_spectrogram, SpectrogramParams};

    let clip_dir = state.integrations.clip_dir.as_ref().ok_or(ApiError::not_found("not found"))?;
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    let row = state
        .core.db
        .get_detection(id_bytes)
        .await
        ?
        .ok_or(ApiError::not_found("not found"))?;

    let rel_path = row.snippet_path.ok_or(ApiError::not_found("not found"))?;
    let wav_path = safe_join(clip_dir, &rel_path)?;
    let png_path = wav_path.with_extension("png");

    // Serve cached PNG if it exists.
    if let Ok(data) = tokio::fs::read(&png_path).await {
        return axum::response::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "image/png")
            .header("cache-control", "public, max-age=31536000, immutable")
            .body(Body::from(data))
            .map_err(ApiError::internal);
    }

    // Generate on demand: read WAV, render spectrogram, cache PNG.
    let png_path_clone = png_path.clone();
    let data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, std::io::Error> {
        let (samples, sample_rate, _channels) = sitta_audio::wav::read_wav(&wav_path)?;
        let params = SpectrogramParams {
            width: 800,
            height: 200,
            ..Default::default()
        };
        generate_spectrogram(&samples, sample_rate, &params, &png_path_clone)?;
        std::fs::read(&png_path_clone)
    })
    .await
    .map_err(ApiError::internal)?
    ?;

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "image/png")
        .header("cache-control", "public, max-age=31536000, immutable")
        .body(Body::from(data))
        .map_err(ApiError::internal)
}

// ── Detection review ───────────────────────────────────────────

#[derive(Deserialize)]
struct ReviewRequest {
    status: String,
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Serialize)]
struct ReviewResponse {
    detection_id: String,
    status: String,
    reviewed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
}

async fn put_review(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ReviewRequest>,
) -> Result<Json<ReviewResponse>, ApiError> {
    if body.status != "correct" && body.status != "false_positive" {
        return Err(ApiError::bad_request("status must be 'correct' or 'false_positive'"));
    }
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    // Verify the detection exists.
    state.core.db.get_detection(id_bytes).await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("not found"))?;

    let now = Utc::now();
    state.core.db.upsert_review(id_bytes, &body.status, now.timestamp_millis(), body.comment.as_deref())
        .await
        ?;

    Ok(Json(ReviewResponse {
        detection_id: uuid.to_string(),
        status: body.status,
        reviewed_at: now.to_rfc3339(),
        comment: body.comment,
    }))
}

async fn get_review(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<ReviewResponse>, ApiError> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    let review = state.core.db.get_review(id_bytes).await
        ?
        .ok_or(ApiError::not_found("not found"))?;

    let det_uuid = sitta_store::models::uuid_from_blob(review.detection_id)
        .map_err(ApiError::internal)?;

    Ok(Json(ReviewResponse {
        detection_id: det_uuid.to_string(),
        status: review.status,
        reviewed_at: millis_to_rfc3339(review.reviewed_at).unwrap_or_default(),
        comment: review.comment,
    }))
}

async fn delete_review(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| ApiError::bad_request("invalid id"))?;
    let id_bytes = uuid.as_bytes().as_slice();

    let deleted = state.core.db.delete_review(id_bytes).await
        ?;

    if deleted { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found("review not found")) }
}

// ── Response types ──────────────────────────────────────────────

#[derive(Serialize)]
struct PaginatedDetections {
    items: Vec<DetectionSummary>,
    offset: i64,
    limit: i64,
    has_more: bool,
}

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
    /// Whether this detection has a saved audio clip.
    has_audio: bool,
    /// Rarity scoring breakdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    rarity: Option<RarityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    range_unverified: Option<bool>,
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
    has_audio: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review: Option<ReviewInfo>,
    /// Detections from other models within ±5 seconds of this detection.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    correlated: Vec<CorrelatedDetection>,
    /// Rarity scoring breakdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    rarity: Option<RarityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    range_unverified: Option<bool>,
}

#[derive(Serialize)]
struct ReviewInfo {
    status: String,
    reviewed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
}

#[derive(Serialize)]
struct CorrelatedDetection {
    id: String,
    model: String,
    model_version: String,
    species: SpeciesInfo,
    confidence: f32,
    has_audio: bool,
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
    /// Sources that are currently receiving audio.
    active_sources: Vec<String>,
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

fn rarity_row_to_info(r: &sitta_store::models::RarityRow) -> RarityInfo {
    RarityInfo {
        score: r.score as f32,
        first_ever: r.first_ever,
        first_season: r.first_season,
        first_week: r.first_week,
        first_day: r.first_day,
        days_since_last: r.days_since_last,
        local_count: r.local_count,
        range_score: r.range_score.map(|s| s as f32),
        temporal_score: r.temporal_score as f32,
    }
}

// ── Dashboard pages ─────────────────────────────────────────────

async fn dashboard_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::dashboard_content(&s.station_name);
    dashboard::page("Dashboard", "dashboard", &content, &s.timezone)
}

async fn species_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::species_content();
    dashboard::page("Species", "species", &content, &s.timezone)
}

async fn detection_detail_page(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::detection_detail_content(&id);
    dashboard::page("Detection", "dashboard", &content, &s.timezone)
}

async fn species_detail_page(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::species_detail_content(&name);
    dashboard::page(&format!("{name} — Species"), "species", &content, &s.timezone)
}

async fn status_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::status_content(&s.station_name);
    dashboard::page("Status", "status", &content, &s.timezone)
}

async fn diagnostics_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::diagnostics_content();
    dashboard::page("Audio Health", "diagnostics", &content, &s.timezone)
}

async fn individuals_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::individuals_content();
    dashboard::page("Individuals", "individuals", &content, &s.timezone)
}

async fn settings_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::settings_content(&s, &state.core.initial_config);
    dashboard::page("Settings", "settings", &content, &s.timezone)
}

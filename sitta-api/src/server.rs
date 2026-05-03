//! axum HTTP server with SSE live feed and REST endpoints.

mod audio_health;
mod individuals;
mod pages;
mod species;

use std::collections::HashMap;
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
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::event::{Alternative, DetectionEvent, IndividualInfo, RarityInfo, SpeciesInfo};
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
    pub first_ever_multiplier: u32,
    pub first_season_multiplier: u32,
    pub first_week_multiplier: u32,
    pub first_day_multiplier: u32,
    pub high_score_multiplier: u32,
    pub per_species_cap: u32,
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
        .route("/api/v1/dashboard/feed", get(dashboard_feed_handler))
        .route("/api/v1/detections/{id}", get(get_detection).delete(delete_detection_handler))
        .route("/api/v1/species", get(species::list_species))
        .route("/api/v1/activity/hourly", get(species::hourly_activity))
        .route("/api/v1/species/{name}/insights", get(species::species_insights))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/audio-health", get(audio_health::audio_health_handler))
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/individuals", get(individuals::list_individuals).post(individuals::enroll_individual).delete(individuals::delete_all_individuals))
        .route("/api/v1/individuals/{id}", get(individuals::get_individual))
        .route("/api/v1/candidates", get(individuals::list_candidate_clusters))
        .route("/api/v1/candidates/{id}/enroll", axum::routing::post(individuals::enroll_cluster))
        .route("/api/v1/candidates/{id}/dismiss", axum::routing::post(individuals::dismiss_cluster))
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
        .route("/", get(pages::dashboard_page))
        .route("/today", get(pages::today_page))
        .route("/detections/{id}", get(pages::detection_detail_page))
        .route("/species", get(pages::species_page))
        .route("/species/{name}", get(pages::species_detail_page))
        .route("/rare", get(pages::rare_page))
        .route("/status", get(pages::status_page))
        .route("/diagnostics", get(pages::diagnostics_page))
        .route("/individuals", get(pages::individuals_page))
        .route("/settings", get(pages::settings_page))
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
    /// If true, only return detections flagged as rare
    /// (first_ever / first_season / first_week / first_day, or score >= 0.6).
    rarity: Option<bool>,
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

    // Request one extra to determine if more results exist. Rarity filter
    // and rarity field both come back from the SQL, so has_more is truthful
    // even when ?rarity=true.
    let display_conf = f64::from(state.core.settings.load().display_min_confidence);
    let rare_only = params.rarity.unwrap_or(false);
    let rows = state
        .core.db
        .recent_detections(since, until, limit + 1, offset, params.species.as_deref(), Some(display_conf), rare_only)
        .await
        ?;

    let has_more = rows.len() as i64 > limit;
    let mut detections: Vec<DetectionSummary> = rows.into_iter().take(limit as usize).filter_map(|r| {
        let individual = detection_row_individual(&r);
        let rarity = r.rarity.as_ref().map(rarity_row_to_info);
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
            alternatives: Vec::new(),
            individual,
            rarity,
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

    // Alternatives are per-detection rows; fetch in a loop. This is still
    // N+1 but smaller than the rarity loop was — and folding into the main
    // SELECT would multiply rows (one per prediction rank).
    for det in &mut detections {
        if let Ok(uuid) = det.id.parse::<uuid::Uuid>()
            && let Ok(preds) = state.core.db.get_predictions(uuid.as_bytes().as_slice()).await
        {
            det.alternatives = preds
                .into_iter()
                .map(|p| Alternative {
                    rank: p.rank as u32,
                    scientific_name: p.scientific_name.unwrap_or_default(),
                    common_name: p.common_name,
                    confidence: p.confidence as f32,
                })
                .collect();
        }
    }

    Ok(Json(PaginatedDetections {
        items: detections,
        offset,
        limit,
        has_more,
    }))
}

fn is_rare(r: &RarityInfo) -> bool {
    r.first_ever || r.first_season || r.first_week || r.first_day || r.score >= 0.6
}

// ── REST: dashboard feed (bucketed) ─────────────────────────────

/// Single bucket: many detections of one species inside a sliding-window
/// session, plus the highest-confidence detection's full data so the UI can
/// render a spectrogram, play button, etc.
#[derive(Serialize)]
struct DashboardFeedItem {
    /// The highest-confidence detection in the bucket. Carries species,
    /// confidence, has_audio, source_name, model, rarity, etc. — all the
    /// fields a card needs to render its primary content.
    best: DetectionSummary,
    /// RFC3339 of the earliest detection in the bucket.
    first_detected_at: String,
    /// RFC3339 of the latest detection in the bucket.
    last_detected_at: String,
    /// How many detections of this species were folded into the bucket.
    /// 1 = a single (or rare) detection; >1 = multiple folded sightings.
    count: u32,
}

#[derive(Deserialize)]
struct DashboardFeedParams {
    /// Start of range (Unix ms). Default: 24 hours ago.
    since: Option<i64>,
    /// End of range (Unix ms). Default: now.
    until: Option<i64>,
    /// Sliding-window bucket size in seconds. Two consecutive non-rare
    /// detections of the same species fold into one bucket if they are
    /// within this many seconds of each other. Default 1800 (30 min).
    bucket_seconds: Option<i64>,
    /// Max bucket count to return (default 50, max 200).
    limit: Option<i64>,
}

async fn dashboard_feed_handler(
    State(state): State<ApiState>,
    Query(params): Query<DashboardFeedParams>,
) -> Result<Json<Vec<DashboardFeedItem>>, ApiError> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);
    let bucket_seconds = params.bucket_seconds.unwrap_or(1800).clamp(30, 86_400);
    let bucket_ms: i64 = bucket_seconds * 1000;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let display_conf = f64::from(state.core.settings.load().display_min_confidence);
    // recent_detections returns rows ordered by detected_at DESC, already
    // dedup'd across overlapping inference windows and with rarity folded
    // into the SELECT. Pull a generous slice so chatty species don't crowd
    // out everything else after bucketing.
    let raw_limit = (limit * 10).max(200);
    let rows = state
        .core
        .db
        .recent_detections(since, until, raw_limit, 0, None, Some(display_conf), false)
        .await?;

    let show_range = state.core.settings.load().show_range_unverified;
    let rows: Vec<sitta_store::models::DetectionRow> = if show_range {
        rows
    } else {
        rows.into_iter()
            .filter(|r| r.range_status.as_deref() != Some("not_in_meta_model"))
            .collect()
    };

    // Walk DESC, building open buckets per species. A non-rare detection
    // folds into the species' open bucket if the time gap to the bucket's
    // earliest entry is within the window. Rare detections always emit
    // standalone (count=1) and close the species' open bucket.
    let mut open: HashMap<String, OpenBucket> = HashMap::new();
    let mut closed: Vec<OpenBucket> = Vec::new();

    for r in rows {
        let sci = r.scientific_name.clone().unwrap_or_default();
        let rarity = r.rarity.as_ref().map(rarity_row_to_info);
        let rare = rarity.as_ref().is_some_and(is_rare);

        if rare {
            if let Some(b) = open.remove(&sci) {
                closed.push(b);
            }
            closed.push(OpenBucket::new(r, rarity));
            continue;
        }

        let det_ms = r.detected_at;
        match open.get_mut(&sci) {
            Some(b) if b.earliest_ms - det_ms <= bucket_ms => {
                b.add(r);
            }
            _ => {
                if let Some(prev) = open.remove(&sci) {
                    closed.push(prev);
                }
                open.insert(sci, OpenBucket::new(r, rarity));
            }
        }
    }
    for (_, b) in open.into_iter() {
        closed.push(b);
    }

    // Sort by latest detection first (the "this just sang" timestamp).
    closed.sort_by_key(|b| std::cmp::Reverse(b.latest_ms));

    let items: Vec<DashboardFeedItem> = closed
        .into_iter()
        .take(limit as usize)
        .filter_map(|b| b.into_item())
        .collect();

    Ok(Json(items))
}

/// In-progress bucket built while walking detections DESC. Exits to
/// `DashboardFeedItem` once finalized.
struct OpenBucket {
    /// Detection currently considered the "best" (highest-confidence)
    /// of the bucket. We render its spectrogram, audio, etc. on the card.
    best_row: sitta_store::models::DetectionRow,
    best_rarity: Option<RarityInfo>,
    /// Earliest detected_at seen so far (ms). Used to test whether the
    /// next (older) detection is still within the bucket window.
    earliest_ms: i64,
    /// Latest detected_at seen so far (ms). Used to sort buckets.
    latest_ms: i64,
    count: u32,
}

impl OpenBucket {
    fn new(row: sitta_store::models::DetectionRow, rarity: Option<RarityInfo>) -> Self {
        let ms = row.detected_at;
        Self {
            earliest_ms: ms,
            latest_ms: ms,
            best_rarity: rarity,
            best_row: row,
            count: 1,
        }
    }

    fn add(&mut self, row: sitta_store::models::DetectionRow) {
        // Earliest decreases as we walk DESC; latest stays at first-seen.
        if row.detected_at < self.earliest_ms {
            self.earliest_ms = row.detected_at;
        }
        if row.detected_at > self.latest_ms {
            self.latest_ms = row.detected_at;
        }
        if row.confidence > self.best_row.confidence {
            // The new row is the new best — but we keep the existing
            // rarity blob: a rarity belongs to a specific detection, and
            // bucketed (non-rare) detections wouldn't carry one anyway.
            self.best_row = row;
            self.best_rarity = None;
        }
        self.count += 1;
    }

    fn into_item(self) -> Option<DashboardFeedItem> {
        let individual = detection_row_individual(&self.best_row);
        let r = self.best_row;
        let id = uuid_from_blob(r.id).ok()?.to_string();
        let detected_at = millis_to_rfc3339(r.detected_at)?;
        let first_detected_at = millis_to_rfc3339(self.earliest_ms)?;
        let last_detected_at = millis_to_rfc3339(self.latest_ms)?;
        Some(DashboardFeedItem {
            best: DetectionSummary {
                id,
                detected_at,
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
                // The bucketed feed favours scrolling speed over per-card
                // alternatives — the user can click through to /detections/{id}
                // for the full alternates list.
                alternatives: Vec::new(),
                individual,
                rarity: self.best_rarity,
                range_unverified: match r.range_status.as_deref() {
                    Some("not_in_meta_model") => Some(true),
                    Some("allowed") | Some("force_allowed") => Some(false),
                    _ => None,
                },
            },
            first_detected_at,
            last_detected_at,
            count: self.count,
        })
    }
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

    let individual = detection_row_individual(&row);

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
        has_embedding: row.has_embedding,
        has_audio: row.snippet_path.is_some(),
        snippet_path: row.snippet_path,
        metadata: row.metadata,
        review,
        correlated,
        individual,
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
pub(crate) struct DetectionSummary {
    pub id: String,
    pub detected_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    pub model: String,
    pub model_version: String,
    pub species: SpeciesInfo,
    pub confidence: f32,
    pub has_embedding: bool,
    /// Whether this detection has a saved audio clip.
    pub has_audio: bool,
    /// Ranked alternative predictions (rank 1 = second-best). Empty when
    /// none above threshold.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Alternative>,
    /// Individual match info, if this detection matched a known individual.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub individual: Option<IndividualInfo>,
    /// Rarity scoring breakdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rarity: Option<RarityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_unverified: Option<bool>,
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
    has_embedding: bool,
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
    /// Individual match info, if this detection matched a known individual.
    #[serde(skip_serializing_if = "Option::is_none")]
    individual: Option<IndividualInfo>,
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
pub(crate) struct SpeciesSummary {
    pub scientific_name: String,
    pub common_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taxon_code: Option<String>,
    pub detection_count: i64,
    pub last_detected_at: String,
    pub avg_confidence: f64,
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

pub(crate) fn millis_to_rfc3339(ms: i64) -> Option<String> {
    DateTime::from_timestamp_millis(ms).map(|dt: DateTime<Utc>| dt.to_rfc3339())
}

pub(crate) fn rarity_row_to_info(r: &sitta_store::models::RarityRow) -> RarityInfo {
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

/// Build an `IndividualInfo` from a `DetectionRow`'s match fields. Returns
/// `None` unless all three match fields are populated.
pub(crate) fn detection_row_individual(
    r: &sitta_store::models::DetectionRow,
) -> Option<IndividualInfo> {
    let id_blob = r.individual_id.as_ref()?;
    let label = r.individual_label.clone()?;
    let similarity = r.individual_similarity?;
    let id = sitta_store::models::uuid_from_blob(id_blob.clone())
        .ok()?
        .to_string();
    Some(IndividualInfo {
        individual_id: id,
        label,
        similarity: similarity as f32,
    })
}


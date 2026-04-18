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
use axum::response::Json;
use axum::routing::{delete, get};
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
    /// Audio broadcast channel for PCM rebroadcast to remote consumers.
    pub audio_tx: broadcast::Sender<Arc<sitta_audio::chunk::AudioChunk>>,
    /// Dynamic source manager for add/remove at runtime.
    pub source_manager: sitta_audio::manager::SourceManager,
    /// Base directory for audio clips. None if snippet saving is disabled.
    pub clip_dir: Option<PathBuf>,
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
        .route("/api/v1/activity/hourly", get(hourly_activity))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/individuals", get(list_individuals).post(enroll_individual).delete(delete_all_individuals))
        .route("/api/v1/individuals/{id}", get(get_individual))
        .route("/api/v1/candidates", get(list_candidate_clusters))
        .route("/api/v1/candidates/{id}/enroll", axum::routing::post(enroll_cluster))
        .route("/api/v1/candidates/{id}/dismiss", axum::routing::post(dismiss_cluster))
        .route("/api/v1/mqtt", get(get_mqtt_config).put(put_mqtt_config))
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
        .route("/species", get(species_page))
        .route("/species/{name}", get(species_detail_page))
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
            has_audio: r.snippet_path.is_some(),
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
) -> Result<Json<HourlyActivityResponse>, StatusCode> {
    let now = Utc::now();
    let since = params.since.unwrap_or_else(|| {
        now.date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    });
    let until = params.until.unwrap_or(since + 86_400_000);

    let display_conf = f64::from(state.settings.load().display_min_confidence);
    let rows = state
        .db
        .hourly_activity(since, until, Some(display_conf))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query hourly activity");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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

async fn delete_all_individuals(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let deleted = state.db.delete_all_individuals().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to delete all individuals");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Reload matcher to clear the in-memory cache.
    if let Some(matcher) = &state.matcher
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

// ── Candidate clusters ─────────────────────────────────────────

async fn list_candidate_clusters(
    State(state): State<ApiState>,
) -> Result<Json<Vec<CandidateClusterSummary>>, StatusCode> {
    let min_members = state.initial_config.min_cluster_size;
    let min_days = state.initial_config.min_distinct_days;

    let rows = state
        .db
        .ready_clusters(min_members, min_days)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to list candidate clusters");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let clusters = rows
        .into_iter()
        .map(|r| CandidateClusterSummary {
            id: r.id,
            scientific_name: r.scientific_name,
            member_count: r.member_count,
            distinct_days: r.distinct_days,
            first_seen_at: millis_to_rfc3339(r.first_seen_at).unwrap_or_default(),
            last_seen_at: millis_to_rfc3339(r.last_seen_at).unwrap_or_default(),
        })
        .collect();

    Ok(Json(clusters))
}

async fn enroll_cluster(
    State(state): State<ApiState>,
    Path(cluster_id): Path<i64>,
    Json(req): Json<ClusterEnrollRequest>,
) -> Result<Json<IndividualSummary>, (StatusCode, String)> {
    let cluster = state
        .db
        .get_cluster(cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "cluster not found".into()))?;

    if cluster.status != "pending" {
        return Err((StatusCode::CONFLICT, format!("cluster is already {}", cluster.status)));
    }

    // Create individual from cluster centroid.
    let individual_id = uuid::Uuid::now_v7();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let dim = cluster.centroid_dim;

    state
        .db
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
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Mark cluster as enrolled.
    state
        .db
        .enroll_cluster(cluster_id, &individual_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Link cluster member detections to the new individual.
    let detection_ids = state
        .db
        .cluster_detection_ids(cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    for det_bytes in &detection_ids {
        if let Ok(det_uuid) = uuid_from_blob(det_bytes.clone()) {
            let match_id = uuid::Uuid::now_v7();
            // Use similarity 0.0 as a sentinel — these are founding members, not runtime matches.
            let _ = state
                .db
                .insert_individual_match(&match_id, &individual_id, &det_uuid, 1.0, now_ms)
                .await;
        }
    }

    // Reload matcher so future detections match against this individual.
    if let Some(matcher) = &state.matcher
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
        label: req.label,
        enrolled_at: millis_to_rfc3339(now_ms).unwrap_or_default(),
        notes: req.notes,
    }))
}

async fn dismiss_cluster(
    State(state): State<ApiState>,
    Path(cluster_id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    let cluster = state
        .db
        .get_cluster(cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "cluster not found".into()))?;

    if cluster.status != "pending" {
        return Err((StatusCode::CONFLICT, format!("cluster is already {}", cluster.status)));
    }

    state
        .db
        .dismiss_cluster(cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(cluster_id, species = %cluster.scientific_name, "Cluster dismissed");

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CandidateClusterSummary {
    id: i64,
    scientific_name: String,
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
) -> Json<settings::MqttSettings> {
    Json(settings::read_mqtt_from_toml(&state.config_path))
}

async fn put_mqtt_config(
    State(state): State<ApiState>,
    Json(mqtt): Json<settings::MqttSettings>,
) -> Result<Json<settings::MqttSettings>, (StatusCode, String)> {
    settings::persist_mqtt_to_toml(&state.config_path, &mqtt)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    tracing::info!(
        enabled = mqtt.enabled,
        host = %mqtt.host,
        port = mqtt.port,
        "MQTT config updated (restart required)"
    );

    Ok(Json(mqtt))
}

// ── Source management ────────────────────────────────────────────

async fn list_sources(
    State(state): State<ApiState>,
) -> Json<Vec<SourceSummary>> {
    let configs = state.source_manager.list().await;
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
) -> Result<Json<SourceSummary>, (StatusCode, String)> {
    let config: sitta_audio::source::SourceConfig = serde_json::from_str(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid source config: {e}")))?;
    let name = config.name().to_string();
    let (source_type, url) = match &config {
        sitta_audio::source::SourceConfig::Rtsp(r) => ("rtsp", Some(r.url.clone())),
        sitta_audio::source::SourceConfig::Local(l) => ("local", Some(l.device.clone())),
        sitta_audio::source::SourceConfig::Remote(r) => ("remote", Some(r.url.clone())),
    };

    state
        .source_manager
        .add(config)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e))?;

    // Persist to config.toml so sources survive restart.
    let all_sources = state.source_manager.list().await;
    if let Err(e) = settings::persist_sources_to_toml(&state.config_path, &all_sources) {
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
) -> Result<StatusCode, (StatusCode, String)> {
    state
        .source_manager
        .remove(&name)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    let all_sources = state.source_manager.list().await;
    if let Err(e) = settings::persist_sources_to_toml(&state.config_path, &all_sources) {
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
    let mut rx = state.audio_tx.subscribe();
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
    Json(state.source_manager.names().await)
}

async fn audio_stream_handler(
    State(state): State<ApiState>,
    Path(source_name): Path<String>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::Response;

    if !state.source_manager.contains(&source_name).await {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(format!("unknown source: {source_name}")))
            .unwrap();
    }

    let mut rx = state.audio_tx.subscribe();
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
        .unwrap()
}

// ── Audio clip serving ──────────────────────────────────────────

async fn detection_audio_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::body::Body;

    let clip_dir = state.clip_dir.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let id_bytes = uuid.as_bytes().as_slice();

    let row = state
        .db
        .get_detection(id_bytes)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query detection for audio");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let rel_path = row.snippet_path.ok_or(StatusCode::NOT_FOUND)?;
    let full_path = clip_dir.join(&rel_path);

    // If a .tmp file exists, the clip is still being written.
    let tmp_path = full_path.with_extension("wav.tmp");
    if tokio::fs::try_exists(&tmp_path).await.unwrap_or(false) {
        return Ok(axum::response::Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("retry-after", "1")
            .body(Body::empty())
            .unwrap());
    }

    let data = tokio::fs::read(&full_path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "audio/wav")
        .header("accept-ranges", "bytes")
        .header("content-disposition", "inline")
        .header("cache-control", "public, max-age=31536000, immutable")
        .body(Body::from(data))
        .unwrap())
}

// ── Spectrogram serving (on-demand with disk cache) ────────────

async fn detection_spectrogram_handler(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::body::Body;
    use sitta_audio::spectrogram::{generate_spectrogram, SpectrogramParams};

    let clip_dir = state.clip_dir.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let id_bytes = uuid.as_bytes().as_slice();

    let row = state
        .db
        .get_detection(id_bytes)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query detection for spectrogram");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let rel_path = row.snippet_path.ok_or(StatusCode::NOT_FOUND)?;
    let wav_path = clip_dir.join(&rel_path);
    let png_path = wav_path.with_extension("png");

    // Serve cached PNG if it exists.
    if let Ok(data) = tokio::fs::read(&png_path).await {
        return Ok(axum::response::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "image/png")
            .header("cache-control", "public, max-age=31536000, immutable")
            .body(Body::from(data))
            .unwrap());
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
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to generate spectrogram");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "image/png")
        .header("cache-control", "public, max-age=31536000, immutable")
        .body(Body::from(data))
        .unwrap())
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
) -> Result<Json<ReviewResponse>, StatusCode> {
    if body.status != "correct" && body.status != "false_positive" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let id_bytes = uuid.as_bytes().as_slice();

    // Verify the detection exists.
    state.db.get_detection(id_bytes).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let now = Utc::now();
    state.db.upsert_review(id_bytes, &body.status, now.timestamp_millis(), body.comment.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to save review");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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
) -> Result<Json<ReviewResponse>, StatusCode> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let id_bytes = uuid.as_bytes().as_slice();

    let review = state.db.get_review(id_bytes).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to query review");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let det_uuid = sitta_store::models::uuid_from_blob(review.detection_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
) -> Result<StatusCode, StatusCode> {
    let uuid = id.parse::<uuid::Uuid>().map_err(|_| StatusCode::BAD_REQUEST)?;
    let id_bytes = uuid.as_bytes().as_slice();

    let deleted = state.db.delete_review(id_bytes).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to delete review");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::NOT_FOUND) }
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
    /// Whether this detection has a saved audio clip.
    has_audio: bool,
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

async fn species_detail_page(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> axum::response::Html<String> {
    let s = state.settings.load();
    let content = dashboard::species_detail_content(&name);
    dashboard::page(&format!("{name} — Species"), "species", &content, &s.timezone)
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

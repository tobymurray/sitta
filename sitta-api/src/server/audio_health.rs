//! `/api/v1/audio-health` — clip retention diagnostics.

use std::sync::atomic::Ordering;

use axum::extract::{Query, State};
use axum::response::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::server::{ApiError, ApiState};

#[derive(Serialize)]
pub(super) struct AudioHealthResponse {
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
    /// Per-rarity-tier clip counts. Each clip falls in exactly one tier
    /// (reviewed-correct overrides everything; otherwise tested most-protective
    /// first). Lets the user see what retention is actually preserving.
    tiers: AudioHealthTiers,
    /// Top species by saved-clip count. Highlights who's dominating the
    /// pool so the user can decide whether to enable `per_species_cap`.
    top_species: Vec<AudioHealthSpeciesClips>,
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
    first_ever_multiplier: u32,
    first_season_multiplier: u32,
    first_week_multiplier: u32,
    first_day_multiplier: u32,
    high_score_multiplier: u32,
    per_species_cap: u32,
}

#[derive(Serialize)]
struct AudioHealthTiers {
    /// Reviewed `correct` — never evicted regardless of tier.
    reviewed_correct: i64,
    first_ever: i64,
    first_season: i64,
    first_week: i64,
    first_day: i64,
    high_score: i64,
    common: i64,
}

#[derive(Serialize)]
struct AudioHealthSpeciesClips {
    scientific_name: String,
    common_name: String,
    clip_count: i64,
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
pub(super) struct AudioHealthParams {
    /// Days to include in the daily breakdown. Default 30, clamped to [1, 365].
    days: Option<u32>,
}

pub(super) async fn audio_health_handler(
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
            first_ever_multiplier: r.first_ever_multiplier,
            first_season_multiplier: r.first_season_multiplier,
            first_week_multiplier: r.first_week_multiplier,
            first_day_multiplier: r.first_day_multiplier,
            high_score_multiplier: r.high_score_multiplier,
            per_species_cap: r.per_species_cap,
        });

    let tiers_row = state
        .core
        .db
        .clip_tier_breakdown()
        .await
        .map_err(ApiError::internal)?;
    let tiers = AudioHealthTiers {
        reviewed_correct: tiers_row.reviewed_correct,
        first_ever: tiers_row.first_ever,
        first_season: tiers_row.first_season,
        first_week: tiers_row.first_week,
        first_day: tiers_row.first_day,
        high_score: tiers_row.high_score,
        common: tiers_row.common,
    };

    let top_species_rows = state
        .core
        .db
        .top_species_by_clip_count(10)
        .await
        .map_err(ApiError::internal)?;
    let top_species: Vec<AudioHealthSpeciesClips> = top_species_rows
        .into_iter()
        .map(|r| AudioHealthSpeciesClips {
            scientific_name: r.scientific_name,
            common_name: r.common_name,
            clip_count: r.clip_count,
        })
        .collect();

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
        tiers,
        top_species,
    }))
}

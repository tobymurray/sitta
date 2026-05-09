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
    /// Lifetime snippet writer counters. Persisted across restarts via
    /// the `lifetime_metrics` table — reading 0 here means "really 0",
    /// not "we just rebooted".
    metrics: AudioHealthMetrics,
    /// Disk-usage gauge: clip-dir size at last retention run vs the cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    disk: Option<AudioHealthDisk>,
    /// Earliest / latest detected_at among detections with no saved clip,
    /// plus the count. None when nothing is missing. Lets the user tell
    /// at a glance whether the missing-audio gap is historical or ongoing.
    #[serde(skip_serializing_if = "Option::is_none")]
    clipless: Option<AudioHealthClipless>,
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
    /// pool so the user can confirm the per-(species, day) quota is doing
    /// its job (or tighten the knobs if a species is still over-represented).
    top_species: Vec<AudioHealthSpeciesClips>,
}

#[derive(Serialize, Default)]
struct AudioHealthMetrics {
    clips_saved: u64,
    /// Submission-time drops — the bounded mpsc channel was full when
    /// `submit()` ran. Indicates SD-card backpressure.
    clips_dropped: u64,
    /// Process-time errors — write_wav, fs metadata, or update_snippet_path
    /// failed. The detection row exists but `snippet_path` stayed NULL.
    /// Surfaced separately because the failure modes differ from drops.
    clips_failed: u64,
    bytes_written: u64,
    /// RFC3339 timestamp of the most recent successful clip save. None
    /// when the writer has saved nothing since the last DB reset.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_clip_saved_at: Option<String>,
    /// RFC3339 of the most recent retention worker run. None before the
    /// worker has completed its first sweep (≤ 1 h after startup).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_retention_at: Option<String>,
    /// Number of clips evicted in the most recent retention run.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_retention_evicted: Option<u64>,
}

/// Disk usage gauge — clip-dir size measured at the last retention run
/// against the configured cap. Lets the UI flag "you're at the cap, the
/// size sweep is actively evicting" without the operator running `du`.
#[derive(Serialize)]
struct AudioHealthDisk {
    /// Bytes used in the clip directory at last retention sweep. None if
    /// the worker hasn't run yet.
    used_bytes: Option<u64>,
    /// `max_disk_mb` × 1024² — the size sweep ceiling. 0 = unlimited.
    cap_bytes: u64,
    /// Computed used / cap as a percent, capped at 999. None if either
    /// number is unavailable.
    used_pct: Option<u32>,
}

#[derive(Serialize)]
struct AudioHealthClipless {
    /// RFC3339 timestamp of the oldest detection without a clip.
    first_detected_at: String,
    /// RFC3339 timestamp of the most recent detection without a clip.
    last_detected_at: String,
    /// Total detections without a clip (all-time).
    count: i64,
    /// Subset of `count` with `detected_at` in the last 15 minutes.
    /// `> 0` means the writer is currently failing to save *new* clips;
    /// `== 0` means the writer is keeping up and the total above is
    /// purely historical.
    recent_count: i64,
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
    per_species_per_day_recent: u32,
    per_species_per_day_top_confidence: u32,
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
    /// Days to include in the daily breakdown. Default 90 so the chart
    /// covers the typical 30-day retention period plus headroom; clamped
    /// to [1, 365].
    days: Option<u32>,
}

pub(super) async fn audio_health_handler(
    State(state): State<ApiState>,
    Query(params): Query<AudioHealthParams>,
) -> Result<Json<AudioHealthResponse>, ApiError> {
    let days = params.days.unwrap_or(90).clamp(1, 365);
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
    // "Recent" cutoff for the clipless query — anything within this window
    // is considered an *active* gap, anything older is historical. 15 min
    // gives the writer plenty of time to catch up under normal load.
    let recent_cutoff = Utc::now().timestamp_millis() - 15 * 60_000;
    let clipless_row = state
        .core
        .db
        .clipless_range(recent_cutoff)
        .await
        .map_err(ApiError::internal)?;

    let clipless = match (clipless_row.first_ms, clipless_row.last_ms) {
        (Some(first), Some(last)) if clipless_row.count > 0 => Some(AudioHealthClipless {
            first_detected_at: crate::server::millis_to_rfc3339(first).unwrap_or_default(),
            last_detected_at: crate::server::millis_to_rfc3339(last).unwrap_or_default(),
            count: clipless_row.count,
            recent_count: clipless_row.recent_count,
        }),
        _ => None,
    };

    let metrics = state
        .integrations
        .snippet_metrics
        .as_ref()
        .map(|m| {
            // Convert Unix-ms atomics to RFC3339, treating 0 as "never observed".
            let to_ts = |ms: i64| if ms > 0 { crate::server::millis_to_rfc3339(ms) } else { None };
            let evicted_raw = m.last_retention_evicted.load(Ordering::Relaxed);
            let last_retention_ms = m.last_retention_ms.load(Ordering::Relaxed);
            AudioHealthMetrics {
                clips_saved: m.clips_saved.load(Ordering::Relaxed),
                clips_dropped: m.clips_dropped.load(Ordering::Relaxed),
                clips_failed: m.clips_failed.load(Ordering::Relaxed),
                bytes_written: m.bytes_written.load(Ordering::Relaxed),
                last_clip_saved_at: to_ts(m.last_clip_saved_ms.load(Ordering::Relaxed)),
                last_retention_at: to_ts(last_retention_ms),
                // Only meaningful once at least one sweep has run.
                last_retention_evicted: (last_retention_ms > 0).then_some(evicted_raw),
            }
        })
        .unwrap_or_default();

    // Disk gauge: cap from the retention config snapshot, used bytes from
    // the metric updated by the retention worker. Both might be missing
    // (snippets disabled, or no sweep has run yet) — render whatever we
    // have and let the UI handle the unknowns.
    let disk = state.integrations.snippet_retention.map(|r| {
        let cap_bytes = r.max_disk_mb.saturating_mul(1024 * 1024);
        let used_bytes = state
            .integrations
            .snippet_metrics
            .as_ref()
            .map(|m| m.last_disk_size_bytes.load(Ordering::Relaxed))
            .filter(|&b| b > 0);
        let used_pct = match (used_bytes, cap_bytes) {
            (Some(used), cap) if cap > 0 => {
                Some((used.saturating_mul(100) / cap).min(999) as u32)
            }
            _ => None,
        };
        AudioHealthDisk {
            used_bytes,
            cap_bytes,
            used_pct,
        }
    });

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
            per_species_per_day_recent: r.per_species_per_day_recent,
            per_species_per_day_top_confidence: r.per_species_per_day_top_confidence,
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
        disk,
        clipless,
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

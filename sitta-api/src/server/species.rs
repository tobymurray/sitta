//! Species summary, hourly activity, and species-insights endpoints.

use axum::extract::{Path, Query, State};
use axum::response::Json;
use chrono::{Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::server::{millis_to_rfc3339, ApiError, ApiState, SpeciesSummary};

#[derive(Deserialize)]
pub(super) struct SpeciesParams {
    since: Option<i64>,
    until: Option<i64>,
}

pub(super) async fn list_species(
    State(state): State<ApiState>,
    Query(params): Query<SpeciesParams>,
) -> Result<Json<Vec<SpeciesSummary>>, ApiError> {
    let now = Utc::now().timestamp_millis();
    let since = params.since.unwrap_or(now - 86_400_000);
    let until = params.until.unwrap_or(now);

    // No confidence filter: show every species with any detection in the window.
    // Individual detection lists still respect display_min_confidence.
    let rows = state.core.db.species_summary(since, until, None).await?;

    let species: Vec<SpeciesSummary> = rows
        .into_iter()
        .filter_map(|r| {
            Some(SpeciesSummary {
                scientific_name: r.scientific_name.unwrap_or_default(),
                common_name: r.common_name,
                taxon_code: r.taxon_code,
                detection_count: r.detection_count,
                last_detected_at: millis_to_rfc3339(r.last_detected_at)?,
                avg_confidence: r.avg_confidence,
            })
        })
        .collect();

    Ok(Json(species))
}

#[derive(Deserialize)]
pub(super) struct ActivityParams {
    /// Start of window (Unix ms). Default: start of today in UTC.
    since: Option<i64>,
    /// End of window (Unix ms). Default: since + 24h.
    until: Option<i64>,
}

pub(super) async fn hourly_activity(
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
        .core
        .db
        .hourly_activity(since, until, Some(display_conf))
        .await?;

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
pub(super) struct HourlyActivityResponse {
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

pub(super) async fn species_insights(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<Json<SpeciesInsightsResponse>, ApiError> {
    let display_conf = f64::from(state.core.settings.load().display_min_confidence);

    let stats = state
        .core
        .db
        .species_stats(&name, Some(display_conf))
        .await?
        .ok_or(ApiError::not_found("not found"))?;

    let profile_rows = state
        .core
        .db
        .species_hourly_profile(&name, Some(display_conf))
        .await?;

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
        .core
        .db
        .species_monthly_distribution(&name, Some(display_conf))
        .await?;
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
        .core
        .db
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
    let data_sufficiency =
        compute_data_sufficiency(&stats, &hourly_distribution, &monthly_distribution);

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
        gaps.push(
            "Need more detections for reliable patterns (have %TOTAL%, want 20+)."
                .to_string()
                .replace("%TOTAL%", &stats.total.to_string()),
        );
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
pub(super) struct SpeciesInsightsResponse {
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use chrono::{Datelike, NaiveDate, Utc};
use sitta_api::event::{Alternative, DetectionEvent, IndividualInfo, RarityInfo, SpeciesInfo};
use sitta_api::settings::RuntimeSettings;
use sitta_audio::chunk::AudioChunk;
use sitta_inference::model::{Classification, RangeStatus};
use sitta_inference::rangefilter::RangeFilter;
use sitta_store::db::Database;
use sitta_store::matcher::IndividualMatcher;
use sitta_store::models::{NewDetection, NewPrediction, NewRarity};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::seed::parse_model_name;
use crate::snippets::SnippetWriter;

/// Shared context for persisting detections from any consumer.
#[derive(Clone)]
pub struct PersistCtx {
    pub db: Database,
    /// (model_db_id, label_index) → label_db_id
    pub label_cache: Arc<HashMap<(i64, i64), i64>>,
    /// classifier display name → model_db_id
    pub model_ids: Arc<HashMap<String, i64>>,
    /// source display name → source UUID
    pub source_ids: Arc<HashMap<String, Uuid>>,
    pub station_id: Uuid,
    /// Broadcast channel for live detection events (SSE, MQTT, etc.).
    pub detection_tx: broadcast::Sender<DetectionEvent>,
    /// Individual matcher for Perch embeddings. None if no Perch configured.
    pub matcher: Option<Arc<IndividualMatcher>>,
    /// Runtime settings (for display_min_confidence threshold).
    pub settings: Arc<ArcSwap<RuntimeSettings>>,
    /// Audio snippet writer. None if snippet saving is disabled.
    pub snippet_writer: Option<SnippetWriter>,
    /// SSE deduplication: last broadcast time (ms) per species.
    /// Suppresses duplicate broadcasts within a 5-second window.
    pub broadcast_dedup: Arc<Mutex<HashMap<String, i64>>>,
    /// Range filter for regional rarity scoring. None if no meta-model loaded.
    pub range_filter: Option<Arc<RangeFilter>>,
    /// Station latitude — used to determine hemisphere for season calculation.
    pub station_latitude: Option<f64>,
}

/// Deduplication window in milliseconds. Detections of the same species
/// within this window are stored in the DB but not broadcast to SSE/MQTT.
const DEDUP_WINDOW_MS: i64 = 5_000;

/// Persist a detection, its secondary predictions, optional embedding,
/// and broadcast a live event to SSE subscribers.
pub async fn persist_detections(
    ctx: &PersistCtx,
    model_id: i64,
    classifier_name: &str,
    chunk: &AudioChunk,
    detections: &[Classification],
    embeddings: Option<&Vec<f32>>,
) {
    let top = match detections.first() {
        Some(d) => d,
        None => return,
    };
    let Some(&label_id) = ctx.label_cache.get(&(model_id, top.label_index as i64)) else {
        tracing::warn!(model_id, label_index = top.label_index, "Label not in cache");
        return;
    };

    let detection_id = Uuid::now_v7();
    let detected_at = chunk.captured_at.timestamp_millis();
    let source_id = ctx.source_ids.get(&chunk.source_name);

    if let Err(e) = ctx
        .db
        .insert_detection(&NewDetection {
            id: &detection_id,
            station_id: &ctx.station_id,
            source_id,
            model_id,
            label_id,
            detected_at,
            confidence: f64::from(top.confidence),
            snippet_path: None,
            snippet_duration_ms: None,
            snippet_sample_rate: None,
            metadata: None,
            range_status: match top.range_status {
                RangeStatus::Allowed => Some("allowed"),
                RangeStatus::ForceAllowed => Some("force_allowed"),
                RangeStatus::NotInMetaModel => Some("not_in_meta_model"),
                RangeStatus::Unfiltered => None,
            },
        })
        .await
    {
        tracing::error!(error = %e, "Failed to persist detection");
        return;
    }

    // Submit audio clip for async saving.
    if let Some(ref writer) = ctx.snippet_writer {
        writer.submit(crate::snippets::SnippetJob {
            detection_id,
            detected_at: chunk.captured_at,
            samples: chunk.samples.clone(),
            sample_rate: chunk.sample_rate,
            channels: chunk.channels,
        });
    }

    // Secondary predictions (rank 1+).
    let predictions: Vec<NewPrediction> = detections
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(r, p)| {
            let label_id = *ctx.label_cache.get(&(model_id, p.label_index as i64))?;
            Some(NewPrediction {
                rank: r as i64,
                label_id,
                confidence: f64::from(p.confidence),
            })
        })
        .collect();
    if let Err(e) = ctx.db.insert_predictions(&detection_id, &predictions).await {
        tracing::error!(error = %e, "Failed to persist predictions");
    }

    // Embedding (Perch path).
    let has_embedding = embeddings.is_some();
    if let Some(emb) = embeddings
        && let Err(e) = ctx.db.insert_embedding(&detection_id, emb).await
    {
        tracing::error!(error = %e, "Failed to persist embedding");
    }

    // Individual matching: check against enrolled individuals, or pool as candidate.
    let individual_match = if let Some(emb) = embeddings
        && let Some(matcher) = &ctx.matcher
    {
        match matcher.find_match(&top.species.scientific_name, emb) {
            Some(m) => {
                // Known individual — record the match.
                let match_id = Uuid::now_v7();
                let now_ms = chunk.captured_at.timestamp_millis();
                if let Err(e) = ctx
                    .db
                    .insert_individual_match(&match_id, &m.individual_id, &detection_id, f64::from(m.similarity), now_ms)
                    .await
                {
                    tracing::error!(error = %e, "Failed to persist individual match");
                }
                Some(m)
            }
            None => {
                // No match — add to candidate pool for background clustering.
                let emb_bytes: &[u8] = bytemuck::cast_slice(emb);
                if let Err(e) = ctx
                    .db
                    .insert_candidate(
                        &detection_id,
                        &top.species.scientific_name,
                        emb_bytes,
                        chunk.captured_at.timestamp_millis(),
                    )
                    .await
                {
                    tracing::error!(error = %e, "Failed to insert candidate embedding");
                }
                None
            }
        }
    } else {
        None
    };

    // ── Rarity scoring ──────────────────────────────────────────
    let rarity_info = compute_rarity(ctx, label_id, detected_at, &top.species.scientific_name).await;
    if let Some(ref ri) = rarity_info {
        let new_rarity = NewRarity {
            detection_id: &detection_id,
            score: f64::from(ri.score),
            first_ever: ri.first_ever,
            first_season: ri.first_season,
            first_week: ri.first_week,
            first_day: ri.first_day,
            days_since_last: ri.days_since_last,
            local_count: ri.local_count,
            range_score: ri.range_score.map(f64::from),
            temporal_score: f64::from(ri.temporal_score),
        };
        if let Err(e) = ctx.db.insert_rarity(&new_rarity).await {
            tracing::error!(error = %e, "Failed to persist rarity score");
        }
    }

    // Broadcast to live subscribers (SSE, MQTT).
    let (model_name, model_version) = parse_model_name(classifier_name);
    let alternatives: Vec<Alternative> = detections
        .iter()
        .enumerate()
        .skip(1)
        .map(|(r, c)| Alternative {
            rank: r as u32,
            scientific_name: c.species.scientific_name.clone(),
            common_name: c.species.common_name.clone(),
            confidence: c.confidence,
        })
        .collect();

    let event = DetectionEvent {
        id: detection_id.to_string(),
        detected_at: chunk.captured_at.to_rfc3339(),
        station_id: ctx.station_id.to_string(),
        source_name: Some(chunk.source_name.clone()),
        model: model_name.to_string(),
        model_version: model_version.to_string(),
        species: SpeciesInfo {
            scientific_name: top.species.scientific_name.clone(),
            common_name: top.species.common_name.clone(),
            taxon_code: top.species.taxon_code.clone(),
        },
        confidence: top.confidence,
        alternatives,
        has_embedding,
        has_audio: ctx.snippet_writer.is_some(),
        individual: individual_match.map(|m| IndividualInfo {
            individual_id: m.individual_id.to_string(),
            label: m.individual_label,
            similarity: m.similarity,
        }),
        rarity: rarity_info,
        range_unverified: match top.range_status {
            RangeStatus::NotInMetaModel => Some(true),
            RangeStatus::Allowed | RangeStatus::ForceAllowed => Some(false),
            RangeStatus::Unfiltered => None,
        },
    };

    // Only broadcast to live UI if above the display threshold AND not a
    // duplicate of a recent broadcast for the same species (5-second window).
    let display_threshold = ctx.settings.load().display_min_confidence;
    if top.confidence >= display_threshold {
        let now_ms = detected_at;
        let species_key = top.species.scientific_name.clone();
        let should_broadcast = {
            let mut dedup = ctx.broadcast_dedup.lock().unwrap();
            // Clean stale entries (older than 2x window) to prevent unbounded growth.
            if dedup.len() > 500 {
                dedup.retain(|_, ts| now_ms - *ts < DEDUP_WINDOW_MS * 2);
            }
            match dedup.get(&species_key) {
                Some(&last_ts) if now_ms - last_ts < DEDUP_WINDOW_MS => false,
                _ => {
                    dedup.insert(species_key, now_ms);
                    true
                }
            }
        };
        if should_broadcast {
            let _ = ctx.detection_tx.send(event);
        }
    }
}

/// Compute rarity score for a detection.
///
/// Components:
///   - **Local rarity**: first-ever, first-of-season, first-of-week, first-of-day,
///     days since last detection, prior detection count.
///   - **Regional rarity**: inverted BirdNET meta-model location score (low score = rare).
///   - **Temporal rarity**: how unusual this hour-of-day is for the species.
async fn compute_rarity(
    ctx: &PersistCtx,
    label_id: i64,
    detected_at_ms: i64,
    scientific_name: &str,
) -> Option<RarityInfo> {
    let station_bytes = ctx.station_id.as_bytes().as_slice();
    let min_conf = f64::from(ctx.settings.load().display_min_confidence);

    // ── Local history ─────────────────────────────────────────
    let (local_count, last_at) = ctx
        .db
        .species_local_history(label_id, station_bytes, detected_at_ms, min_conf)
        .await
        .ok()?;

    let first_ever = local_count == 0;

    let detection_date = chrono::DateTime::from_timestamp_millis(detected_at_ms)?
        .with_timezone(&Utc)
        .date_naive();

    let (first_season, first_week, first_day, days_since_last) = if first_ever {
        (true, true, true, None)
    } else {
        let last_ms = last_at.unwrap_or(0);
        let last_date = chrono::DateTime::from_timestamp_millis(last_ms)?
            .with_timezone(&Utc)
            .date_naive();

        let days = (detection_date - last_date).num_days();

        let first_day = detection_date != last_date;
        let first_week = detection_date.iso_week() != last_date.iso_week()
            || detection_date.year() != last_date.year();
        let first_season = meteorological_season(detection_date, ctx.station_latitude)
            != meteorological_season(last_date, ctx.station_latitude);

        (first_season, first_week, first_day, Some(days))
    };

    // ── Regional rarity ───────────────────────────────────────
    let range_score = ctx
        .range_filter
        .as_ref()
        .and_then(|f| f.score_for(scientific_name));

    // ── Temporal rarity ───────────────────────────────────────
    let hour_utc = (detected_at_ms / 3_600_000) % 24;
    let hour_fraction = ctx
        .db
        .species_hour_fraction(label_id, hour_utc, detected_at_ms, min_conf)
        .await
        .unwrap_or(0.0);

    // Invert: a low fraction means this hour is unusual for the species.
    // Clamp to [0, 1]. If no prior data, treat as not unusual (0.0).
    let temporal_score = if local_count == 0 {
        0.0
    } else {
        (1.0 - hour_fraction * 24.0).clamp(0.0, 1.0) as f32
    };

    // ── Composite score ───────────────────────────────────────
    // Weight: local 0.40, regional 0.35, temporal 0.25.
    let local_score = compute_local_score(first_ever, first_season, first_week, first_day, days_since_last);
    let regional_score = range_score.map(|s| 1.0 - s).unwrap_or(0.0); // invert: low location score = rare
    let score = local_score * 0.40 + regional_score * 0.35 + temporal_score * 0.25;

    Some(RarityInfo {
        score,
        first_ever,
        first_season,
        first_week,
        first_day,
        days_since_last,
        local_count,
        range_score,
        temporal_score,
    })
}

/// Local rarity sub-score (0.0 = common, 1.0 = very rare).
fn compute_local_score(
    first_ever: bool,
    first_season: bool,
    first_week: bool,
    first_day: bool,
    days_since_last: Option<i64>,
) -> f32 {
    if first_ever {
        return 1.0;
    }
    if first_season {
        return 0.8;
    }

    // Decay based on days since last detection.
    let recency = days_since_last
        .map(|d| (d as f32 / 30.0).min(1.0))
        .unwrap_or(0.0);

    if first_week {
        0.5 + recency * 0.3
    } else if first_day {
        0.2 + recency * 0.2
    } else {
        recency * 0.1
    }
}

/// Returns a season identifier (0-3) for a given date, adjusted for hemisphere.
/// Northern: Spring=0 (Mar-May), Summer=1 (Jun-Aug), Autumn=2 (Sep-Nov), Winter=3 (Dec-Feb).
/// Southern: seasons are shifted by 6 months.
fn meteorological_season(date: NaiveDate, latitude: Option<f64>) -> u8 {
    let month = date.month();
    let northern = match month {
        3..=5 => 0,   // Spring
        6..=8 => 1,   // Summer
        9..=11 => 2,  // Autumn
        _ => 3,        // Winter (Dec, Jan, Feb)
    };
    // Southern hemisphere: shift by 2 seasons.
    if latitude.is_some_and(|lat| lat < 0.0) {
        (northern + 2) % 4
    } else {
        northern
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meteorological_season_northern() {
        let jan = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let apr = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        let jul = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let oct = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();

        let lat = Some(43.0); // Northern hemisphere
        assert_eq!(meteorological_season(jan, lat), 3); // Winter
        assert_eq!(meteorological_season(apr, lat), 0); // Spring
        assert_eq!(meteorological_season(jul, lat), 1); // Summer
        assert_eq!(meteorological_season(oct, lat), 2); // Autumn
    }

    #[test]
    fn meteorological_season_southern() {
        let jan = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let apr = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        let jul = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let oct = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();

        let lat = Some(-34.0); // Southern hemisphere
        assert_eq!(meteorological_season(jan, lat), 1); // Summer
        assert_eq!(meteorological_season(apr, lat), 2); // Autumn
        assert_eq!(meteorological_season(jul, lat), 3); // Winter
        assert_eq!(meteorological_season(oct, lat), 0); // Spring
    }

    #[test]
    fn meteorological_season_no_latitude() {
        let jul = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        // No latitude defaults to northern hemisphere behavior.
        assert_eq!(meteorological_season(jul, None), 1);
    }

    #[test]
    fn local_score_first_ever() {
        assert_eq!(compute_local_score(true, true, true, true, None), 1.0);
    }

    #[test]
    fn local_score_first_season() {
        assert_eq!(compute_local_score(false, true, true, true, Some(90)), 0.8);
    }

    #[test]
    fn local_score_first_week() {
        let score = compute_local_score(false, false, true, true, Some(5));
        assert!(score > 0.5 && score < 0.8);
    }

    #[test]
    fn local_score_first_day() {
        let score = compute_local_score(false, false, false, true, Some(1));
        assert!(score > 0.2 && score < 0.5);
    }

    #[test]
    fn local_score_same_day() {
        let score = compute_local_score(false, false, false, false, Some(0));
        assert_eq!(score, 0.0);
    }
}

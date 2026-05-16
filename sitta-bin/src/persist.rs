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
    /// Base URL for detection links (e.g., "http://192.168.1.132:8080").
    /// Used to construct `detection_url` in MQTT and SSE events.
    pub api_base_url: Option<String>,
    /// Presence confirmation tracker. Requires repeated detections of the same
    /// species within a time window before broadcasting to SSE/MQTT.
    pub presence_tracker: Arc<Mutex<PresenceTracker>>,
}

/// Deduplication window in milliseconds. Detections of the same species
/// within this window are stored in the DB but not broadcast to SSE/MQTT.
const DEDUP_WINDOW_MS: i64 = 5_000;

/// Accumulates detections per species and requires N hits within a sliding
/// time window before broadcasting a confirmed-presence event.
///
/// This dramatically reduces false positives: a single 3-second window
/// at 0.72 confidence is weak evidence, but 3 hits in 10 minutes is
/// strong evidence of actual presence.
pub struct PresenceTracker {
    min_detections: u32,
    window_ms: i64,
    /// Confidence at or above which a single detection bypasses the repeat requirement.
    immediate_threshold: Option<f32>,
    /// Per-species: list of (timestamp_ms, confidence, event) within the window.
    pending: HashMap<String, Vec<(i64, f32, DetectionEvent)>>,
    /// Per-species: timestamp of last confirmed broadcast (cooldown).
    confirmed_at: HashMap<String, i64>,
}

impl PresenceTracker {
    pub fn new(min_detections: u32, window_minutes: u32) -> Self {
        Self {
            min_detections,
            window_ms: i64::from(window_minutes) * 60_000,
            immediate_threshold: None,
            pending: HashMap::new(),
            confirmed_at: HashMap::new(),
        }
    }

    /// Update configuration if settings changed at runtime.
    pub fn update_config(&mut self, min_detections: u32, window_minutes: u32, immediate_threshold: Option<f32>) {
        let new_window_ms = i64::from(window_minutes) * 60_000;
        if self.min_detections != min_detections
            || self.window_ms != new_window_ms
            || self.immediate_threshold != immediate_threshold
        {
            self.min_detections = min_detections;
            self.window_ms = new_window_ms;
            self.immediate_threshold = immediate_threshold;
            // Clear accumulators since the rules changed.
            self.pending.clear();
            self.confirmed_at.clear();
            tracing::info!(min_detections, window_minutes, ?immediate_threshold, "Presence tracker reconfigured");
        }
    }

    /// Record a detection and return Some(event) if this triggers a confirmation.
    ///
    /// The returned event is the one with the highest confidence in the window,
    /// decorated with `peak_confidence` and `confirmed_count`.
    pub fn track(&mut self, species: &str, timestamp_ms: i64, event: DetectionEvent) -> Option<DetectionEvent> {
        // Bypass: min_detections <= 1 means every detection confirms immediately.
        if self.min_detections <= 1 {
            return Some(event);
        }

        // Still in cooldown from a recent confirmation for this species?
        if let Some(&last_confirmed) = self.confirmed_at.get(species)
            && timestamp_ms - last_confirmed < self.window_ms
        {
            return None;
        }

        // Immediate threshold: a single very-high-confidence detection bypasses
        // the repeat requirement. Useful for flyover calls and other brief vocalizations.
        if let Some(threshold) = self.immediate_threshold
            && event.confidence >= threshold
        {
            // Set cooldown so the normal tracker doesn't re-alert.
            self.confirmed_at.insert(species.to_string(), timestamp_ms);
            self.pending.remove(species);
            return Some(event);
        }

        let entries = self.pending.entry(species.to_string()).or_default();

        // Prune entries outside the sliding window.
        entries.retain(|(ts, _, _)| timestamp_ms - *ts < self.window_ms);

        entries.push((timestamp_ms, event.confidence, event));

        if entries.len() as u32 >= self.min_detections {
            // Confirmed! Pick the detection with peak confidence.
            let count = entries.len() as u32;
            let peak = entries.iter().map(|(_, c, _)| *c).fold(0.0_f32, f32::max);
            let best_idx = entries
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);

            let (_, _, mut best_event) = entries.swap_remove(best_idx);
            best_event.peak_confidence = Some(peak);
            best_event.confirmed_count = Some(count);

            // Clear accumulator and set cooldown.
            entries.clear();
            self.confirmed_at.insert(species.to_string(), timestamp_ms);

            // Periodic cleanup of stale cooldown entries.
            if self.confirmed_at.len() > 500 {
                self.confirmed_at.retain(|_, ts| timestamp_ms - *ts < self.window_ms * 2);
            }

            Some(best_event)
        } else {
            // Periodic cleanup of stale pending species.
            if self.pending.len() > 500 {
                self.pending.retain(|_, v| !v.is_empty());
            }
            None
        }
    }
}

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

    // Environment-label filters. The label is "environmental" (Perch's
    // Animal / Vehicle / Bark / voice / Bass_drum etc.) when its top
    // species name isn't a Linnaean binomial — same predicate the rarity
    // gate uses. Two independent knobs gate persistence:
    //   * skip_environment_detections: drop the row entirely
    //   * skip_environment_clips:      keep the row, skip the WAV
    let is_environment_label = !looks_like_species_name(&top.species.scientific_name);
    if is_environment_label && ctx.settings.load().skip_environment_detections {
        return;
    }

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

    // Submit audio clip for async saving — unless this is an environment
    // label and the user has asked us to skip those.
    let skip_clip = is_environment_label && ctx.settings.load().skip_environment_clips;
    if !skip_clip
        && let Some(ref writer) = ctx.snippet_writer
    {
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
    let rarity_info = compute_rarity(ctx, detected_at, &top.species.scientific_name).await;
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
        detection_url: ctx.api_base_url.as_ref().map(|base| {
            format!("{base}/detections/{detection_id}")
        }),
        peak_confidence: None,
        confirmed_count: None,
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
                    dedup.insert(species_key.clone(), now_ms);
                    true
                }
            }
        };
        if should_broadcast {
            // Feed through the presence tracker: requires N detections within
            // T minutes before actually broadcasting a confirmed event.
            let confirmed_event = {
                let mut tracker = ctx.presence_tracker.lock().unwrap();
                let settings = ctx.settings.load();
                tracker.update_config(
                    settings.presence_min_detections,
                    settings.presence_window_minutes,
                    settings.presence_immediate_threshold,
                );
                tracker.track(&species_key, now_ms, event)
            };
            if let Some(evt) = confirmed_event {
                let _ = ctx.detection_tx.send(evt);
            }
        }
    }
}

/// True when `s` looks like a Linnaean binomial — "Genus species" with a
/// capitalised first word and a lowercase (optionally hyphenated) second
/// word. Used to gate rarity scoring so Perch's ambient-sound pseudo-labels
/// ("Animal", "Vehicle", "Bass" from "Bass_drum") can't masquerade as new
/// species. Real species at runtime always arrive as proper binomials —
/// either taxonomy-resolved or pulled from the BirdNET "Genus species_Common"
/// label format.
fn looks_like_species_name(s: &str) -> bool {
    let mut parts = s.split_whitespace();
    let Some(genus) = parts.next() else { return false };
    let Some(species) = parts.next() else { return false };
    if parts.next().is_some() {
        return false;
    }
    let mut g = genus.chars();
    let Some(first) = g.next() else { return false };
    if !first.is_ascii_uppercase() {
        return false;
    }
    if !g.all(|c| c.is_ascii_lowercase()) {
        return false;
    }
    !species.is_empty() && species.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

/// Compute rarity score for a detection.
///
/// Components:
///   - **Local rarity**: first-ever, first-of-season, first-of-week, first-of-day,
///     days since last detection, prior detection count.
///   - **Regional rarity**: inverted BirdNET meta-model location score (low score = rare).
///   - **Temporal rarity**: how unusual this hour-of-day is for the species.
///
/// Returns `None` for non-species labels (Perch's ambient-sound classes,
/// Bass-from-"Bass_drum", etc.). Without this guard, `species_local_history`
/// silently returns 0 for those, so every detection is flagged
/// `first_ever=true` → 999× retention → pinned forever on disk.
async fn compute_rarity(
    ctx: &PersistCtx,
    detected_at_ms: i64,
    scientific_name: &str,
) -> Option<RarityInfo> {
    if !looks_like_species_name(scientific_name) {
        return None;
    }

    let station_bytes = ctx.station_id.as_bytes().as_slice();
    let min_conf = f64::from(ctx.settings.load().display_min_confidence);

    // ── Local history ─────────────────────────────────────────
    // Keyed by scientific_name across all models — earlier same-species
    // detections from a different model still count.
    let (local_count, last_at) = ctx
        .db
        .species_local_history(scientific_name, station_bytes, detected_at_ms, min_conf)
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
        // Compare season-instance, not bare season. Without the year
        // component, a returning migrant (last seen in Spring 2026,
        // detected again in Spring 2027) would compare `Spring == Spring`
        // and never re-trigger `first_season`, costing it the 8× retention
        // multiplier it had the year before. The (year, season) tuple
        // distinguishes "this Spring" from "last Spring" while still
        // grouping all three Dec-Feb months into one winter instance.
        let first_season = season_instance(detection_date, ctx.station_latitude)
            != season_instance(last_date, ctx.station_latitude);

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
        .species_hour_fraction(scientific_name, hour_utc, detected_at_ms, min_conf)
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

/// Returns a (season-year, season) tuple uniquely identifying the *instance*
/// of meteorological season a date belongs to.
///
/// `meteorological_season` alone collapses every Spring (or Summer, …)
/// into the same 0..3 bucket regardless of year, so a returning migrant
/// — last seen in Spring 2026, detected again in Spring 2027 — sees
/// `Spring == Spring` and is denied `first_season`. By pairing the
/// season with a year, "Spring 2026" and "Spring 2027" become distinct
/// instances and the migrant earns the 8× retention multiplier again.
///
/// The year component is the year the season "owns": the Dec-Feb winter
/// (Northern) / summer (Southern) attaches all three months to the
/// December year, so a 2026-12-15 and 2027-01-15 detection share one
/// season-instance. Every other season uses the calendar year.
fn season_instance(date: NaiveDate, latitude: Option<f64>) -> (i32, u8) {
    let season = meteorological_season(date, latitude);
    // January / February belong to the previous calendar year's
    // Dec-starting season. Pulling them back by one year groups the
    // three months into a single (year, season) bucket.
    let year = if date.month() <= 2 {
        date.year() - 1
    } else {
        date.year()
    };
    (year, season)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_event(species: &str, confidence: f32) -> DetectionEvent {
        DetectionEvent {
            id: "test-id".into(),
            detected_at: "2026-04-22T12:00:00Z".into(),
            station_id: "station-1".into(),
            source_name: None,
            model: "birdnet".into(),
            model_version: "2.4".into(),
            species: SpeciesInfo {
                scientific_name: species.into(),
                common_name: species.into(),
                taxon_code: None,
            },
            confidence,
            alternatives: vec![],
            has_embedding: false,
            has_audio: false,
            individual: None,
            rarity: None,
            range_unverified: None,
            detection_url: None,
            peak_confidence: None,
            confirmed_count: None,
        }
    }

    #[test]
    fn presence_tracker_min_1_passes_immediately() {
        let mut tracker = PresenceTracker::new(1, 10);
        let event = dummy_event("Tyto alba", 0.8);
        let result = tracker.track("Tyto alba", 1000, event);
        assert!(result.is_some());
        // No peak_confidence or confirmed_count set when min_detections=1.
        let evt = result.unwrap();
        assert!(evt.peak_confidence.is_none());
        assert!(evt.confirmed_count.is_none());
    }

    #[test]
    fn presence_tracker_requires_n_detections() {
        let mut tracker = PresenceTracker::new(3, 10);

        // First detection: not enough.
        let r1 = tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.7));
        assert!(r1.is_none());

        // Second: still not enough.
        let r2 = tracker.track("Tyto alba", 60_000, dummy_event("Tyto alba", 0.9));
        assert!(r2.is_none());

        // Third: confirmed!
        let r3 = tracker.track("Tyto alba", 120_000, dummy_event("Tyto alba", 0.75));
        assert!(r3.is_some());
        let evt = r3.unwrap();
        assert_eq!(evt.confirmed_count, Some(3));
        assert_eq!(evt.peak_confidence, Some(0.9));
        // The event should be the one with peak confidence (0.9).
        assert!((evt.confidence - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn presence_tracker_cooldown_after_confirmation() {
        let mut tracker = PresenceTracker::new(2, 10);

        // Confirm species.
        tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.8));
        let confirmed = tracker.track("Tyto alba", 60_000, dummy_event("Tyto alba", 0.85));
        assert!(confirmed.is_some());

        // Within cooldown window (10 minutes = 600_000 ms): should be suppressed.
        let r = tracker.track("Tyto alba", 300_000, dummy_event("Tyto alba", 0.9));
        assert!(r.is_none());

        // After cooldown (11 minutes): should start accumulating again.
        let r = tracker.track("Tyto alba", 660_001, dummy_event("Tyto alba", 0.7));
        assert!(r.is_none()); // first detection, need 2
        let r = tracker.track("Tyto alba", 700_000, dummy_event("Tyto alba", 0.72));
        assert!(r.is_some()); // second detection, confirmed!
    }

    #[test]
    fn presence_tracker_prunes_old_entries() {
        let mut tracker = PresenceTracker::new(2, 10);
        let window_ms = 10 * 60_000;

        // Detection at t=0.
        tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.8));

        // Detection at t=window+1ms — the first one should be pruned.
        let r = tracker.track("Tyto alba", window_ms + 1, dummy_event("Tyto alba", 0.85));
        assert!(r.is_none(), "First detection expired, so only 1 in window");

        // Another detection within the new window.
        let r = tracker.track("Tyto alba", window_ms + 60_000, dummy_event("Tyto alba", 0.9));
        assert!(r.is_some(), "Two detections within current window");
    }

    #[test]
    fn presence_tracker_independent_species() {
        let mut tracker = PresenceTracker::new(2, 10);

        tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.8));
        tracker.track("Strix aluco", 0, dummy_event("Strix aluco", 0.85));

        // Second Barn Owl detection confirms Barn Owl only.
        let r = tracker.track("Tyto alba", 60_000, dummy_event("Tyto alba", 0.9));
        assert!(r.is_some());

        // Tawny Owl still needs one more.
        let r = tracker.track("Strix aluco", 60_000, dummy_event("Strix aluco", 0.88));
        assert!(r.is_some());
    }

    #[test]
    fn presence_tracker_update_config_clears_state() {
        let mut tracker = PresenceTracker::new(3, 10);

        // Accumulate 2 detections (1 short of threshold).
        tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.8));
        tracker.track("Tyto alba", 60_000, dummy_event("Tyto alba", 0.85));

        // Change config: now only 2 needed. But update_config clears state.
        tracker.update_config(2, 10, None);

        // Previous detections are gone, need to start fresh.
        let r = tracker.track("Tyto alba", 120_000, dummy_event("Tyto alba", 0.9));
        assert!(r.is_none(), "State was cleared on config change");

        let r = tracker.track("Tyto alba", 180_000, dummy_event("Tyto alba", 0.88));
        assert!(r.is_some(), "Two fresh detections after reconfig");
    }

    #[test]
    fn presence_tracker_immediate_threshold_bypasses() {
        let mut tracker = PresenceTracker::new(3, 10);
        tracker.immediate_threshold = Some(0.90);

        // Below threshold: needs 3 hits.
        let r = tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.85));
        assert!(r.is_none());

        // At threshold: immediate broadcast.
        let r = tracker.track("Tyto alba", 60_000, dummy_event("Tyto alba", 0.95));
        assert!(r.is_some());
        let evt = r.unwrap();
        // Immediate bypass doesn't set peak_confidence/confirmed_count.
        assert!(evt.peak_confidence.is_none());
        assert!(evt.confirmed_count.is_none());
    }

    #[test]
    fn presence_tracker_immediate_threshold_sets_cooldown() {
        let mut tracker = PresenceTracker::new(2, 10);
        tracker.immediate_threshold = Some(0.90);

        // High-confidence detection confirms immediately.
        let r = tracker.track("Tyto alba", 0, dummy_event("Tyto alba", 0.95));
        assert!(r.is_some());

        // Subsequent detection within cooldown window is suppressed.
        let r = tracker.track("Tyto alba", 60_000, dummy_event("Tyto alba", 0.7));
        assert!(r.is_none());

        // But another high-confidence one also gets suppressed by cooldown.
        let r = tracker.track("Tyto alba", 120_000, dummy_event("Tyto alba", 0.96));
        assert!(r.is_none(), "Cooldown applies even to high-confidence");
    }

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

    // ── season_instance: (year, season) tuple for cross-year disambiguation
    //
    // The bug this guards against: a Woodcock detected in Spring 2026 and
    // again in Spring 2027 was previously denied `first_season` because
    // `meteorological_season` returned the same bucket for both. With the
    // year component, "Spring 2026" and "Spring 2027" are distinct.

    #[test]
    fn season_instance_distinguishes_returning_spring_migrant() {
        let lat = Some(44.0); // Northern hemisphere
        let prev_spring = NaiveDate::from_ymd_opt(2026, 5, 16).unwrap();
        let next_spring = NaiveDate::from_ymd_opt(2027, 4, 15).unwrap();
        assert_ne!(
            season_instance(prev_spring, lat),
            season_instance(next_spring, lat),
            "Returning spring migrant should re-trigger first_season"
        );
    }

    #[test]
    fn season_instance_groups_winter_across_calendar_year() {
        // Dec 2026 and Jan/Feb 2027 are the same meteorological winter.
        // Detections across them should share a season-instance so a bird
        // singing all winter doesn't ratchet first_season at New Year's.
        let lat = Some(44.0); // Northern hemisphere
        let dec = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        let jan = NaiveDate::from_ymd_opt(2027, 1, 15).unwrap();
        let feb = NaiveDate::from_ymd_opt(2027, 2, 15).unwrap();
        assert_eq!(season_instance(dec, lat), season_instance(jan, lat));
        assert_eq!(season_instance(jan, lat), season_instance(feb, lat));
    }

    #[test]
    fn season_instance_keeps_adjacent_seasons_distinct() {
        // Within one year, Spring → Summer → Autumn must be three distinct
        // instances (no off-by-one mishaps).
        let lat = Some(44.0);
        let spring = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
        let summer = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let autumn = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_ne!(season_instance(spring, lat), season_instance(summer, lat));
        assert_ne!(season_instance(summer, lat), season_instance(autumn, lat));
        assert_ne!(season_instance(spring, lat), season_instance(autumn, lat));
    }

    #[test]
    fn season_instance_handles_southern_summer_year_wrap() {
        // Southern hemisphere: Dec-Feb is *summer* (the year-wrapping
        // season). Same grouping rule applies — Dec 2026 and Jan 2027
        // share a summer-instance.
        let lat = Some(-34.0);
        let dec = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        let jan = NaiveDate::from_ymd_opt(2027, 1, 15).unwrap();
        assert_eq!(season_instance(dec, lat), season_instance(jan, lat));
        // And a returning southern-summer migrant the year after must
        // earn a fresh first_season.
        let jan_next = NaiveDate::from_ymd_opt(2028, 1, 15).unwrap();
        assert_ne!(season_instance(jan, lat), season_instance(jan_next, lat));
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

    // ── Species-name gate ─────────────────────────────────────────

    #[test]
    fn species_name_accepts_real_binomials() {
        assert!(looks_like_species_name("Tyto alba"));
        assert!(looks_like_species_name("Sayornis phoebe"));
        assert!(looks_like_species_name("Vulpes vulpes"));
        assert!(looks_like_species_name("Geothlypis trichas"));
        // Hyphenated species epithet is legal Linnaean form.
        assert!(looks_like_species_name("Picoides arcticus-borealis"));
    }

    #[test]
    fn species_name_rejects_perch_ambient_labels() {
        // Bare common-name pseudo-species ("Animal", "Vehicle", "Music") arrive
        // as the raw label since they have no taxonomy match. All single token.
        assert!(!looks_like_species_name("Animal"));
        assert!(!looks_like_species_name("Vehicle"));
        assert!(!looks_like_species_name("Music"));
        assert!(!looks_like_species_name("Engine"));
        // Compound non-species ("Bass_drum") split on '_' produces just "Bass".
        assert!(!looks_like_species_name("Bass"));
        assert!(!looks_like_species_name("Acoustic"));
        // Empty / whitespace-only.
        assert!(!looks_like_species_name(""));
        assert!(!looks_like_species_name("   "));
        // Three-or-more-word labels (subspecies trinomials are uncommon in
        // these label sets; reject to stay conservative).
        assert!(!looks_like_species_name("Setophaga coronata audubonii"));
    }

    #[test]
    fn species_name_rejects_malformed_capitalisation() {
        // Lowercase genus — not a valid binomial.
        assert!(!looks_like_species_name("tyto alba"));
        // Uppercase species epithet.
        assert!(!looks_like_species_name("Tyto Alba"));
        // Numbers / punctuation in the words.
        assert!(!looks_like_species_name("Tyto alba2"));
        assert!(!looks_like_species_name("Tyto alb@"));
    }
}

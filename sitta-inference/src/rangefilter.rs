use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use birdnet_onnx::RangeFilter as OnnxRangeFilter;
use chrono::{Datelike, NaiveDate, Utc};

use crate::InferenceError;
use crate::model::{Classification, RangeStatus};

/// Allowed species set + per-species location scores for a single day.
type DayScores = (Arc<HashSet<String>>, Arc<HashMap<String, f32>>);

struct Cached {
    date: NaiveDate,
    // Lowercase scientific names expected at this location today.
    // Keyed by scientific name so the same filter works across models (BirdNET and Perch
    // have different label-index spaces but share scientific names via the taxonomy).
    allowed: Arc<HashSet<String>>,
    // Per-species location score from the meta-model (0.0–1.0).
    // Includes ALL species, not just those above threshold.
    scores: Arc<HashMap<String, f32>>,
}

/// Location + date filter backed by the BirdNET species-occurrence meta-model.
///
/// Wraps `birdnet_onnx::RangeFilter`. Location scores are computed once per
/// calendar day and cached; subsequent calls within the same day use the cached
/// set without touching the ONNX session.
///
/// The allowed set is keyed by **lowercase scientific name**, so the same
/// `RangeFilter` instance can be shared between BirdNET and Perch consumers
/// even though they have different label-index spaces. Species outside the
/// meta-model's label space (e.g. Perch-only species not in BirdNET's 6,522)
/// pass through unfiltered since there is no occurrence data to filter on.
///
/// Species in `force_allow` bypass geographic scoring entirely — they always
/// pass regardless of the meta-model's location score for that species.
pub struct RangeFilter {
    inner: OnnxRangeFilter,
    lat: f32,
    lon: f32,
    /// eBird species codes that always pass, e.g. `["guifow"]` for Helmeted Guineafowl.
    /// Checked against `Classification::species::taxon_code`; requires taxonomy to be loaded.
    force_allow: HashSet<String>,
    /// All species known to the BirdNET meta-model (lowercase scientific names).
    /// Used to distinguish "species below threshold" (drop) from "species not in
    /// meta-model at all" (pass through — e.g. Perch-only species).
    known_species: HashSet<String>,
    cache: Mutex<Option<Cached>>,
}

impl RangeFilter {
    /// Load the BirdNET meta-model from `meta_model_path`.
    ///
    /// `labels` must be the raw label slice from a BirdNET v2.4 classifier.
    /// The resulting filter can be applied to any classifier (BirdNET or Perch)
    /// as long as `Classification::species::scientific_name` is populated.
    pub fn load(
        meta_model_path: &Path,
        labels: &[String],
        lat: f32,
        lon: f32,
        threshold: f32,
        force_allow: HashSet<String>,
    ) -> Result<Self, InferenceError> {
        let inner = OnnxRangeFilter::builder()
            .model_path(meta_model_path.to_string_lossy().into_owned())
            .from_classifier_labels(labels)
            .threshold(threshold)
            .build()
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?;

        // Build the set of all species known to the meta-model so we can
        // distinguish "below threshold" (drop) from "not in model" (pass through).
        let known_species: HashSet<String> = labels
            .iter()
            .map(|label| {
                label
                    .split_once('_')
                    .map(|(sci, _)| sci)
                    .unwrap_or(label)
                    .to_lowercase()
            })
            .collect();

        tracing::info!(
            model = %meta_model_path.display(),
            lat,
            lon,
            threshold,
            force_allow = ?force_allow,
            known_species = known_species.len(),
            "Loaded BirdNET range filter (meta-model)"
        );

        Ok(Self {
            inner,
            lat,
            lon,
            force_allow,
            known_species,
            cache: Mutex::new(None),
        })
    }

    /// Filter `classifications` to species expected at this station's location today.
    ///
    /// A classification passes if either:
    /// - its scientific name matches a species in today's allowed set, or
    /// - its `taxon_code` is in the `force_allow` list.
    ///
    /// Matching is by **scientific name** so the filter is model-agnostic — it works
    /// for both BirdNET (6,522 species) and Perch (14,795 species) without separate
    /// filter instances or index remapping.
    ///
    /// Location scores are cached per calendar date. On the first call of each day
    /// the meta-model runs (~1 ms); subsequent calls are O(n) HashSet lookups.
    pub fn filter(
        &self,
        mut classifications: Vec<Classification>,
    ) -> Result<Vec<Classification>, InferenceError> {
        let today = Utc::now().date_naive();
        let allowed = self.allowed_for_today(today)?;

        classifications.retain_mut(|c| {
            // Primary check: scientific name in today's location-allowed set.
            let sci = c.species.scientific_name.to_lowercase();
            if allowed.contains(&sci) {
                c.range_status = RangeStatus::Allowed;
                return true;
            }
            // Secondary check: force_allow by taxon code (requires taxonomy).
            if let Some(code) = c.species.taxon_code.as_deref()
                && self.force_allow.contains(code)
            {
                c.range_status = RangeStatus::ForceAllowed;
                tracing::debug!(
                    species = %c.species.common_name,
                    taxon_code = code,
                    confidence = c.confidence,
                    "Detection passed via force_allow"
                );
                return true;
            }
            // Species not in the meta-model at all (e.g. Perch-only species outside
            // BirdNET's 6,522 label space) — pass through since we have no occurrence
            // data to filter on.
            if !self.known_species.contains(&sci) {
                c.range_status = RangeStatus::NotInMetaModel;
                tracing::debug!(
                    species = %c.species.common_name,
                    scientific_name = %c.species.scientific_name,
                    confidence = format_args!("{:.3}", c.confidence),
                    "Detection passed (species not in meta-model label space)"
                );
                return true;
            }
            tracing::debug!(
                species = %c.species.common_name,
                scientific_name = %c.species.scientific_name,
                confidence = format_args!("{:.3}", c.confidence),
                "Detection dropped by range filter (species not expected at this location)"
            );
            false
        });

        Ok(classifications)
    }

    /// Look up the BirdNET meta-model location score for a species on today's date.
    ///
    /// Returns the raw probability (0.0–1.0) if the species is known to the meta-model,
    /// or `None` if unknown (e.g. a Perch-only species with no BirdNET label).
    pub fn score_for(&self, scientific_name: &str) -> Option<f32> {
        let today = Utc::now().date_naive();
        let (_, scores) = self.allowed_and_scores_for_today(today).ok()?;
        scores.get(&scientific_name.to_lowercase()).copied()
    }

    fn allowed_for_today(
        &self,
        today: NaiveDate,
    ) -> Result<Arc<HashSet<String>>, InferenceError> {
        self.allowed_and_scores_for_today(today)
            .map(|(allowed, _)| allowed)
    }

    fn allowed_and_scores_for_today(
        &self,
        today: NaiveDate,
    ) -> Result<DayScores, InferenceError> {
        // Fast path: cache hit — clone the Arcs (pointer copy).
        {
            let guard = self.cache.lock().expect("range filter cache poisoned");
            if let Some(c) = guard.as_ref()
                && c.date == today
            {
                return Ok((Arc::clone(&c.allowed), Arc::clone(&c.scores)));
            }
        }

        // Cache miss: run meta-model inference, build scientific-name set, update cache.
        let month = today.month();
        let day = today.day();
        let raw_scores = self
            .inner
            .predict(self.lat, self.lon, month, day)
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        // BirdNET label format: "Scientific Name_Common Name".
        // Extract the scientific name (everything before the first '_') and normalise.
        let mut allowed_set = HashSet::new();
        let mut score_map = HashMap::with_capacity(raw_scores.len());

        for s in &raw_scores {
            let sci = s
                .species
                .split_once('_')
                .map(|(sci, _)| sci)
                .unwrap_or(&s.species)
                .to_lowercase();
            score_map.insert(sci.clone(), s.score);
            allowed_set.insert(sci);
        }

        let allowed = Arc::new(allowed_set);
        let scores = Arc::new(score_map);

        tracing::info!(
            date = %today,
            allowed_species = allowed.len(),
            force_allow = self.force_allow.len(),
            "Range filter: updated location scores for today"
        );

        let mut guard = self.cache.lock().expect("range filter cache poisoned");
        *guard = Some(Cached {
            date: today,
            allowed: Arc::clone(&allowed),
            scores: Arc::clone(&scores),
        });

        Ok((allowed, scores))
    }
}

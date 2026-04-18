use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use birdnet_onnx::RangeFilter as OnnxRangeFilter;
use chrono::{Datelike, NaiveDate, Utc};

use crate::InferenceError;
use crate::model::Classification;

struct Cached {
    date: NaiveDate,
    // Lowercase scientific names expected at this location today.
    // Keyed by scientific name so the same filter works across models (BirdNET and Perch
    // have different label-index spaces but share scientific names via the taxonomy).
    allowed: Arc<HashSet<String>>,
}

/// Location + date filter backed by the BirdNET species-occurrence meta-model.
///
/// Wraps `birdnet_onnx::RangeFilter`. Location scores are computed once per
/// calendar day and cached; subsequent calls within the same day use the cached
/// set without touching the ONNX session.
///
/// The allowed set is keyed by **lowercase scientific name**, so the same
/// `RangeFilter` instance can be shared between BirdNET and Perch consumers
/// even though they have different label-index spaces.
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

        tracing::info!(
            model = %meta_model_path.display(),
            lat,
            lon,
            threshold,
            force_allow = ?force_allow,
            "Loaded BirdNET range filter (meta-model)"
        );

        Ok(Self {
            inner,
            lat,
            lon,
            force_allow,
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

        classifications.retain(|c| {
            // Primary check: scientific name in today's location-allowed set.
            let sci = c.species.scientific_name.to_lowercase();
            if allowed.contains(&sci) {
                return true;
            }
            // Secondary check: force_allow by taxon code (requires taxonomy).
            if let Some(code) = c.species.taxon_code.as_deref()
                && self.force_allow.contains(code)
            {
                tracing::debug!(
                    species = %c.species.common_name,
                    taxon_code = code,
                    confidence = c.confidence,
                    "Detection passed via force_allow"
                );
                return true;
            }
            false
        });

        Ok(classifications)
    }

    fn allowed_for_today(
        &self,
        today: NaiveDate,
    ) -> Result<Arc<HashSet<String>>, InferenceError> {
        // Fast path: cache hit — clone the Arc (pointer copy).
        {
            let guard = self.cache.lock().expect("range filter cache poisoned");
            if let Some(c) = guard.as_ref()
                && c.date == today
            {
                return Ok(Arc::clone(&c.allowed));
            }
        }

        // Cache miss: run meta-model inference, build scientific-name set, update cache.
        let month = today.month();
        let day = today.day();
        let scores = self
            .inner
            .predict(self.lat, self.lon, month, day)
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        // BirdNET label format: "Scientific Name_Common Name".
        // Extract the scientific name (everything before the first '_') and normalise.
        let allowed: Arc<HashSet<String>> = Arc::new(
            scores
                .iter()
                .map(|s| {
                    s.species
                        .split_once('_')
                        .map(|(sci, _)| sci)
                        .unwrap_or(&s.species)
                        .to_lowercase()
                })
                .collect(),
        );

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
        });

        Ok(allowed)
    }
}

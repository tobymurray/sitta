use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use birdnet_onnx::RangeFilter as OnnxRangeFilter;
use chrono::{Datelike, NaiveDate, Utc};

use crate::InferenceError;
use crate::model::Classification;

struct Cached {
    date: NaiveDate,
    // Arc so callers can hold a ref without re-locking the mutex each inference.
    allowed: Arc<HashSet<usize>>,
}

/// Location + date filter backed by the BirdNET species-occurrence meta-model.
///
/// Wraps `birdnet_onnx::RangeFilter`. Location scores are computed once per
/// calendar day and cached; subsequent calls within the same day use the cached
/// set without touching the ONNX session.
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
    /// `labels` must be the raw label slice from the paired `BirdNet` classifier —
    /// the meta-model output dimension must match the label count.
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
    /// - its `label_index` is in the meta-model's allowed set for today's date, or
    /// - its `taxon_code` is in the `force_allow` list.
    ///
    /// Location scores are cached per calendar date. On the first call of each day
    /// the meta-model runs (CPU-bound, ~1 ms); subsequent calls are O(n) HashSet
    /// lookups against the cached allowed-index set.
    pub fn filter(
        &self,
        mut classifications: Vec<Classification>,
    ) -> Result<Vec<Classification>, InferenceError> {
        let today = Utc::now().date_naive();
        let allowed = self.allowed_for_today(today)?;

        classifications.retain(|c| {
            if allowed.contains(&c.label_index) {
                return true;
            }
            // Check force_allow by taxon code (requires taxonomy to be loaded).
            if let Some(code) = c.species.taxon_code.as_deref() {
                if self.force_allow.contains(code) {
                    tracing::debug!(
                        species = %c.species.common_name,
                        taxon_code = code,
                        confidence = c.confidence,
                        "Detection passed via force_allow"
                    );
                    return true;
                }
            }
            false
        });

        Ok(classifications)
    }

    fn allowed_for_today(
        &self,
        today: NaiveDate,
    ) -> Result<Arc<HashSet<usize>>, InferenceError> {
        // Fast path: cache hit — clone the Arc (pointer copy).
        {
            let guard = self.cache.lock().expect("range filter cache poisoned");
            if let Some(c) = guard.as_ref() {
                if c.date == today {
                    return Ok(Arc::clone(&c.allowed));
                }
            }
        }

        // Cache miss: run meta-model inference, then update cache.
        let month = today.month();
        let day = today.day();
        let scores = self
            .inner
            .predict(self.lat, self.lon, month, day)
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        let allowed: Arc<HashSet<usize>> =
            Arc::new(scores.iter().map(|s| s.index).collect());

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

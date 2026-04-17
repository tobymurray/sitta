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
pub struct RangeFilter {
    inner: OnnxRangeFilter,
    lat: f32,
    lon: f32,
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
            "Loaded BirdNET range filter (meta-model)"
        );

        Ok(Self {
            inner,
            lat,
            lon,
            cache: Mutex::new(None),
        })
    }

    /// Filter `classifications` to species expected at this station's location today.
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
        classifications.retain(|c| allowed.contains(&c.label_index));
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

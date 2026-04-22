use std::path::Path;
use std::sync::Arc;

use birdnet_onnx::{Classifier as OnnxClassifier, InferenceOptions, ModelType};
use sitta_taxonomy::EbirdTaxonomy;

use crate::InferenceError;
use crate::model::{Classification, Classifier, RangeStatus, Species};

/// BirdNET species classifier via birdnet-onnx (ONNX Runtime).
///
/// Supports BirdNET v2.4, v3.0, Perch v2, and BSG Finland models.
/// Thread-safe via internal `Arc` in birdnet-onnx — no Mutex needed.
pub struct BirdNet {
    inner: OnnxClassifier,
    taxonomy: Option<Arc<EbirdTaxonomy>>,
}

impl BirdNet {
    /// Load a BirdNET-family model from an ONNX file and labels file.
    pub fn load(
        model_path: &Path,
        labels_path: &Path,
        min_confidence: f32,
        top_k: usize,
    ) -> Result<Self, InferenceError> {
        Self::load_with_taxonomy(model_path, labels_path, min_confidence, top_k, None)
    }

    /// Load a model and attach an eBird taxonomy for common-name resolution.
    ///
    /// Required for Perch v2, whose labels are bare scientific names (`Tyto_alba`).
    /// Also enriches BirdNET detections with eBird species codes.
    pub fn load_with_taxonomy(
        model_path: &Path,
        labels_path: &Path,
        min_confidence: f32,
        top_k: usize,
        taxonomy: Option<Arc<EbirdTaxonomy>>,
    ) -> Result<Self, InferenceError> {
        let inner = OnnxClassifier::builder()
            .model_path(model_path.to_string_lossy().into_owned())
            .labels_path(labels_path.to_string_lossy().into_owned())
            .top_k(top_k)
            .min_confidence(min_confidence)
            .build()
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?;

        let config = inner.config();
        tracing::info!(
            model = %model_path.display(),
            labels = %labels_path.display(),
            model_type = ?config.model_type,
            sample_rate = config.sample_rate,
            sample_count = config.sample_count,
            num_species = config.num_species,
            min_confidence,
            top_k,
            taxonomy = taxonomy.is_some(),
            "Loaded BirdNET model via birdnet-onnx"
        );

        Ok(Self { inner, taxonomy })
    }

    /// Raw label strings as loaded by birdnet-onnx, needed to build a `RangeFilter`.
    pub fn labels(&self) -> &[String] {
        self.inner.labels()
    }

    /// Parse a label string from the model's labels file into a `Species`.
    ///
    /// Two label formats are handled:
    /// - Perch: `"Tyto_alba"` — underscore-joined scientific name, no common name.
    ///   The taxonomy is required to resolve the common name.
    /// - BirdNET: `"Tyto alba_Barn Owl"` — scientific name, underscore, common name.
    ///   The taxonomy optionally enriches with a species code.
    fn parse_species(&self, label: &str) -> Species {
        let taxonomy = self.taxonomy.as_deref();

        // Try the whole label as a (possibly underscore-joined) scientific name.
        // This handles Perch labels like "Tyto_alba".
        if let Some(entry) = taxonomy.and_then(|t| t.lookup(label)) {
            return Species {
                scientific_name: entry.scientific_name.clone(),
                common_name: entry.common_name.clone(),
                taxon_code: Some(entry.species_code.clone()),
            };
        }

        // Fall back to BirdNET label format: "Scientific Name_Common Name".
        let (scientific, common) = label
            .split_once('_')
            .unwrap_or((label, "Unknown"));

        // Enrich with taxonomy species code if available.
        let taxon_code = taxonomy
            .and_then(|t| t.lookup(scientific))
            .map(|e| e.species_code.clone());

        Species {
            scientific_name: scientific.to_string(),
            common_name: common.to_string(),
            taxon_code,
        }
    }
}

impl Classifier for BirdNet {
    fn classify(&self, audio: &[f32]) -> Result<Vec<Classification>, InferenceError> {
        let expected = self.inner.config().sample_count;
        if audio.len() != expected {
            return Err(InferenceError::InvalidInput(format!(
                "expected {expected} samples, got {}",
                audio.len()
            )));
        }

        let result = self
            .inner
            .predict(audio, &InferenceOptions::default())
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        let classifications = result
            .predictions
            .iter()
            .map(|p| Classification {
                label_index: p.index,
                species: self.parse_species(&p.species),
                confidence: p.confidence,
                range_status: RangeStatus::default(),
            })
            .collect();

        Ok(classifications)
    }

    fn classify_with_embeddings(
        &self,
        audio: &[f32],
    ) -> Result<(Vec<Classification>, Option<Vec<f32>>), InferenceError> {
        let expected = self.inner.config().sample_count;
        if audio.len() != expected {
            return Err(InferenceError::InvalidInput(format!(
                "expected {expected} samples, got {}",
                audio.len()
            )));
        }

        let result = self
            .inner
            .predict(audio, &InferenceOptions::default())
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        let classifications = result
            .predictions
            .iter()
            .map(|p| Classification {
                label_index: p.index,
                species: self.parse_species(&p.species),
                confidence: p.confidence,
                range_status: RangeStatus::default(),
            })
            .collect();

        Ok((classifications, result.embeddings))
    }

    fn name(&self) -> &str {
        match self.inner.config().model_type {
            ModelType::BirdNetV24 => "BirdNET v2.4",
            ModelType::BirdNetV30 => "BirdNET v3.0",
            ModelType::PerchV2 => "Perch v2",
            ModelType::BsgFinland => "BSG Finland",
        }
    }

    fn sample_rate(&self) -> u32 {
        self.inner.config().sample_rate
    }

    fn window_samples(&self) -> usize {
        self.inner.config().sample_count
    }

    fn raw_labels(&self) -> &[String] {
        self.inner.labels()
    }

    fn embedding_dim(&self) -> Option<usize> {
        match self.inner.config().model_type {
            ModelType::PerchV2 => Some(1536),
            ModelType::BirdNetV30 => Some(1024),
            _ => None,
        }
    }
}

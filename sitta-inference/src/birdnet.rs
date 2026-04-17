use std::path::Path;

use birdnet_onnx::{Classifier as OnnxClassifier, InferenceOptions, ModelType};

use crate::InferenceError;
use crate::model::{Classification, Classifier, Species};

/// BirdNET species classifier via birdnet-onnx (ONNX Runtime).
///
/// Supports BirdNET v2.4, v3.0, Perch v2, and BSG Finland models.
/// Thread-safe via internal `Arc` in birdnet-onnx — no Mutex needed.
pub struct BirdNet {
    inner: OnnxClassifier,
}

impl BirdNet {
    /// Load a BirdNET-family model from an ONNX file and labels file.
    ///
    /// Model type (v2.4, v3.0, Perch, BSG) is auto-detected from the model file.
    /// The labels file has one entry per line in `ScientificName_CommonName` format.
    pub fn load(
        model_path: &Path,
        labels_path: &Path,
        min_confidence: f32,
        top_k: usize,
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
            "Loaded BirdNET model via birdnet-onnx"
        );

        Ok(Self { inner })
    }

    fn parse_species(species_str: &str) -> Species {
        let (scientific, common) = species_str
            .split_once('_')
            .unwrap_or((species_str, "Unknown"));
        Species {
            scientific_name: scientific.to_string(),
            common_name: common.to_string(),
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
                species: Self::parse_species(&p.species),
                confidence: p.confidence,
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
                species: Self::parse_species(&p.species),
                confidence: p.confidence,
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
}

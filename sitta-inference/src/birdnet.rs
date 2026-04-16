use std::path::Path;

use tract_onnx::prelude::*;
use tract_onnx::tract_hir::tract_ndarray;

use crate::model::{Classification, Classifier, Species};
use crate::InferenceError;

const SAMPLE_RATE: u32 = 48_000;
const WINDOW_SECONDS: u32 = 3;
const WINDOW_SAMPLES: usize = (SAMPLE_RATE * WINDOW_SECONDS) as usize; // 144,000

type Model = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// BirdNET v2.4 species classifier.
///
/// Loads an ONNX-converted BirdNET model and a labels file. Audio input is
/// raw waveform at 48 kHz -- the model contains its own spectrogram layer.
pub struct BirdNet {
    model: Model,
    labels: Vec<Species>,
    min_confidence: f32,
    sigmoid_sensitivity: f32,
}

impl BirdNet {
    /// Load BirdNET from an ONNX model file and a labels file.
    ///
    /// The labels file has one entry per line in `ScientificName_CommonName` format.
    /// The ONNX model can be produced from the BirdNET Keras/SavedModel via `tf2onnx`.
    pub fn load(
        model_path: &Path,
        labels_path: &Path,
        min_confidence: f32,
        sigmoid_sensitivity: f32,
    ) -> Result<Self, InferenceError> {
        tracing::info!(
            model = %model_path.display(),
            labels = %labels_path.display(),
            min_confidence,
            sigmoid_sensitivity,
            "Loading BirdNET model"
        );

        let labels = Self::load_labels(labels_path)?;
        tracing::info!(species_count = labels.len(), "Loaded BirdNET labels");

        let model = tract_onnx::onnx()
            .model_for_path(model_path)
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?
            .with_input_fact(0, f32::fact([1, WINDOW_SAMPLES as i64]).into())
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?
            .into_optimized()
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?
            .into_runnable()
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?;

        tracing::info!("BirdNET model loaded and optimised");

        Ok(Self {
            model,
            labels,
            min_confidence,
            sigmoid_sensitivity,
        })
    }

    fn load_labels(path: &Path) -> Result<Vec<Species>, InferenceError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| InferenceError::LabelsLoad(e.to_string()))?;

        let labels: Vec<Species> = content
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (scientific, common) = line.split_once('_').unwrap_or((line, "Unknown"));
                Species {
                    scientific_name: scientific.to_string(),
                    common_name: common.to_string(),
                }
            })
            .collect();

        if labels.is_empty() {
            return Err(InferenceError::LabelsLoad("labels file is empty".into()));
        }

        Ok(labels)
    }

    fn sigmoid(&self, x: f32) -> f32 {
        1.0 / (1.0 + (-self.sigmoid_sensitivity * x).exp())
    }
}

impl Classifier for BirdNet {
    fn classify(&self, audio: &[f32]) -> Result<Vec<Classification>, InferenceError> {
        if audio.len() != WINDOW_SAMPLES {
            return Err(InferenceError::InvalidInput(format!(
                "expected {} samples, got {}",
                WINDOW_SAMPLES,
                audio.len()
            )));
        }

        let input =
            tract_ndarray::Array2::from_shape_vec((1, WINDOW_SAMPLES), audio.to_vec())
                .map_err(|e| InferenceError::Inference(e.to_string()))?;

        let result = self
            .model
            .run(tvec!(input.into_tvalue()))
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        let output = result[0]
            .to_array_view::<f32>()
            .map_err(|e| InferenceError::Inference(e.to_string()))?;

        let mut classifications: Vec<Classification> = output
            .iter()
            .enumerate()
            .filter_map(|(i, &logit)| {
                let confidence = self.sigmoid(logit);
                if confidence >= self.min_confidence && i < self.labels.len() {
                    Some(Classification {
                        label_index: i,
                        species: self.labels[i].clone(),
                        confidence,
                    })
                } else {
                    None
                }
            })
            .collect();

        classifications.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        Ok(classifications)
    }

    fn name(&self) -> &str {
        "BirdNET v2.4"
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn window_samples(&self) -> usize {
        WINDOW_SAMPLES
    }
}

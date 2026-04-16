//! Inference engine for running bioacoustic classification models.
//!
//! Provides a [`model::Classifier`] trait that abstracts over different model
//! backends (BirdNET, Google Perch, etc.). Each classifier takes raw audio
//! samples and returns species classifications with confidence scores.

pub mod birdnet;
pub mod model;

#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    #[error("failed to load model: {0}")]
    ModelLoad(String),
    #[error("failed to load labels: {0}")]
    LabelsLoad(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("inference failed: {0}")]
    Inference(String),
}

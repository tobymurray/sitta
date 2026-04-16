/// A species classification result from an inference model.
#[derive(Debug, Clone)]
pub struct Classification {
    /// Index into the model's label set.
    pub label_index: usize,
    /// Species identification.
    pub species: Species,
    /// Confidence score in [0.0, 1.0] (post-sigmoid for BirdNET).
    pub confidence: f32,
}

/// A species identifier with scientific and common names.
#[derive(Debug, Clone)]
pub struct Species {
    pub scientific_name: String,
    pub common_name: String,
}

/// Trait for models that produce species classifications from audio.
///
/// Both BirdNET and Google Perch can implement this trait. Perch can
/// additionally produce embeddings for individual identification, but
/// species classification is the shared capability.
pub trait Classifier: Send + Sync {
    /// Run classification on raw audio samples.
    ///
    /// `audio` must contain exactly [`window_samples()`](Self::window_samples)
    /// f32 samples at [`sample_rate()`](Self::sample_rate) Hz.
    fn classify(&self, audio: &[f32]) -> Result<Vec<Classification>, crate::InferenceError>;

    /// Human-readable model name (e.g., "BirdNET v2.4").
    fn name(&self) -> &str;

    /// Required sample rate in Hz.
    fn sample_rate(&self) -> u32;

    /// Required number of samples per inference window.
    fn window_samples(&self) -> usize;
}

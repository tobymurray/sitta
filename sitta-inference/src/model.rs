/// How a classification relates to the geographic/seasonal range filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RangeStatus {
    /// No range filter was applied (filter disabled or not configured).
    #[default]
    Unfiltered,
    /// Species is in the meta-model and passed the location/date threshold.
    Allowed,
    /// Species passed via the `force_allow` bypass list.
    ForceAllowed,
    /// Species is NOT in the meta-model label space (e.g. Perch-only species).
    NotInMetaModel,
}

/// A species classification result from an inference model.
#[derive(Debug, Clone)]
pub struct Classification {
    /// Index into the model's label set.
    pub label_index: usize,
    /// Species identification.
    pub species: Species,
    /// Confidence score in [0.0, 1.0] (post-sigmoid for BirdNET).
    pub confidence: f32,
    /// How this detection relates to the range filter. Set by `RangeFilter::filter()`.
    pub range_status: RangeStatus,
}

/// A species identifier with scientific and common names.
#[derive(Debug, Clone)]
pub struct Species {
    pub scientific_name: String,
    pub common_name: String,
    /// eBird species code (e.g., "barowl1"), present when a taxonomy was loaded.
    pub taxon_code: Option<String>,
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

    /// Run classification and optionally return embedding vectors.
    ///
    /// Models that support embeddings (e.g., BirdNET v3.0, Perch v2)
    /// return `Some(Vec<f32>)`. Models without embedding support return `None`.
    fn classify_with_embeddings(
        &self,
        audio: &[f32],
    ) -> Result<(Vec<Classification>, Option<Vec<f32>>), crate::InferenceError> {
        Ok((self.classify(audio)?, None))
    }

    /// Raw label strings from the model's label file, indexed by position.
    ///
    /// Used at startup to seed the labels table. Default returns empty
    /// for models that don't expose their label set.
    fn raw_labels(&self) -> &[String] {
        &[]
    }

    /// Embedding vector dimensionality, if the model produces embeddings.
    fn embedding_dim(&self) -> Option<usize> {
        None
    }
}

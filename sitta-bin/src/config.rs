use serde::Deserialize;
use sitta_audio::source::SourceConfig;

/// Top-level application configuration, loaded from config.toml.
#[derive(Debug, Deserialize)]
pub struct Config {
    pub station: StationConfig,
    pub audio: AudioConfig,
    #[serde(default)]
    pub inference: InferenceConfig,
}

#[derive(Debug, Deserialize)]
pub struct StationConfig {
    /// Unique station identifier (e.g., "station_01").
    pub id: String,
    /// Human-readable station name (e.g., "North Paddock").
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AudioConfig {
    /// Duration of each audio chunk in seconds. Defaults to 3 (matches BirdNET window).
    #[serde(default = "default_chunk_seconds")]
    pub chunk_seconds: u32,
    /// Audio sources to capture from.
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Deserialize)]
pub struct InferenceConfig {
    #[serde(default)]
    pub birdnet: Option<BirdNetConfig>,
    #[serde(default)]
    pub perch: Option<PerchConfig>,
}

#[derive(Debug, Deserialize)]
pub struct BirdNetConfig {
    /// Path to the ONNX model file.
    pub model_path: String,
    /// Path to the labels text file.
    pub labels_path: String,
    /// Minimum confidence threshold. Default: 0.25.
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    /// Number of top predictions to return. Default: 10.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

#[derive(Debug, Deserialize)]
pub struct PerchConfig {
    /// Path to the Perch ONNX model file.
    pub model_path: String,
    /// Path to the labels CSV file.
    pub labels_path: String,
    /// Minimum confidence threshold. Default: 0.25.
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    /// Number of top predictions to return. Default: 10.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            birdnet: None,
            perch: None,
        }
    }
}

fn default_chunk_seconds() -> u32 {
    3
}
fn default_min_confidence() -> f32 {
    0.25
}
fn default_top_k() -> usize {
    10
}

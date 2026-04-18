use serde::Deserialize;
use sitta_audio::source::SourceConfig;

/// Top-level application configuration, loaded from config.toml.
#[derive(Debug, Deserialize)]
pub struct Config {
    pub station: StationConfig,
    pub audio: AudioConfig,
    #[serde(default)]
    pub store: StoreConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub inference: InferenceConfig,
    #[serde(default)]
    pub taxonomy: Option<TaxonomyConfig>,
}

#[derive(Debug, Deserialize)]
pub struct StationConfig {
    /// Unique station identifier (e.g., "station_01").
    pub id: String,
    /// Human-readable station name (e.g., "North Paddock").
    pub name: String,
    /// Station latitude in decimal degrees (-90 to 90). Required for the range filter.
    pub latitude: Option<f32>,
    /// Station longitude in decimal degrees (-180 to 180). Required for the range filter.
    pub longitude: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub struct AudioConfig {
    /// Duration of each audio chunk in seconds. Defaults to 3 (matches BirdNET window).
    #[serde(default = "default_chunk_seconds")]
    pub chunk_seconds: u32,
    /// Audio sources to capture from.
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Default, Deserialize)]
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
    /// Optional BirdNET meta-model for geographic/seasonal filtering.
    /// Path to birdnet-v24-meta.onnx (install with: birda models install birdnet-v24).
    /// Requires [station] latitude and longitude to be set.
    pub meta_model_path: Option<String>,
    /// Minimum species occurrence score from the meta-model (0.0–1.0). Default: 0.01.
    #[serde(default = "default_meta_threshold")]
    pub meta_threshold: f32,
    /// eBird species codes that always pass the geographic filter regardless of
    /// the meta-model score. Use for domestic or feral animals known to be present.
    /// Requires [taxonomy] to be configured (species codes come from taxon_code).
    /// Example: ["helgui1"] for Helmeted Guineafowl (Domestic type), ["redjun1"] for Domestic Chicken.
    #[serde(default)]
    pub force_allow: Vec<String>,
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
    /// Cosine similarity threshold for individual matching. Default: 0.85.
    #[serde(default = "default_individual_threshold")]
    pub individual_threshold: f32,
}

/// eBird taxonomy configuration for common-name and species-code resolution.
#[derive(Debug, Deserialize)]
pub struct TaxonomyConfig {
    /// Path to the eBird taxonomy CSV file.
    /// Download: curl -o ebird-taxonomy.csv "https://api.ebird.org/v2/ref/taxonomy/ebird?fmt=csv"
    pub ebird_path: String,
}

/// Persistence layer configuration.
#[derive(Debug, Deserialize)]
pub struct StoreConfig {
    /// Path to the SQLite database file.
    #[serde(default = "default_store_path")]
    pub path: String,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: default_store_path(),
        }
    }
}

/// HTTP API configuration.
#[derive(Debug, Deserialize)]
pub struct ApiConfig {
    /// Socket address to bind (e.g., "0.0.0.0:8080").
    #[serde(default = "default_api_bind")]
    pub bind: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: default_api_bind(),
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
fn default_meta_threshold() -> f32 {
    0.01
}
fn default_store_path() -> String {
    "./sitta.db".into()
}
fn default_api_bind() -> String {
    "0.0.0.0:8080".into()
}
fn default_individual_threshold() -> f32 {
    0.85
}

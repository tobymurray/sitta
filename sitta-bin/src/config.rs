use serde::Deserialize;
use sitta_audio::source::SourceConfig;

/// Top-level application configuration, loaded from config.toml.
#[derive(Debug, Deserialize)]
pub struct Config {
    pub station: StationConfig,
    pub audio: AudioConfig,
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

fn default_chunk_seconds() -> u32 {
    3
}

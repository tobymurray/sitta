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
    pub snippets: SnippetConfig,
    #[serde(default)]
    pub inference: InferenceConfig,
    #[serde(default)]
    pub mqtt: Option<MqttConfig>,
    #[serde(default)]
    pub taxonomy: Option<TaxonomyConfig>,
    #[serde(default)]
    pub presence: PresenceConfig,
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
    /// IANA timezone (e.g., "America/Toronto"). Derived from longitude if not set.
    #[serde(default)]
    pub timezone: Option<String>,
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
    /// Stride in seconds for inference windows. Controls overlap.
    /// stride = chunk_seconds means no overlap. stride < chunk_seconds means
    /// overlapping windows (e.g. 1.0 with 3s chunks = 2s overlap).
    /// Default: 1.0 (2s overlap, matching BirdNET-Go behaviour).
    #[serde(default = "default_birdnet_stride")]
    pub stride_seconds: f32,
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
    /// Cosine similarity threshold for matching against enrolled individuals. Default: 0.85.
    #[serde(default = "default_individual_threshold")]
    pub individual_threshold: f32,
    /// Minimum cosine similarity to merge a candidate into an existing cluster.
    /// Lower = more aggressive grouping. Default: 0.70.
    #[serde(default = "default_cluster_merge_threshold")]
    pub cluster_merge_threshold: f32,
    /// Minimum cluster size before suggesting enrollment. Default: 5.
    #[serde(default = "default_min_cluster_size")]
    pub min_cluster_size: u32,
    /// Minimum distinct calendar days of detections before suggesting enrollment. Default: 2.
    #[serde(default = "default_min_distinct_days")]
    pub min_distinct_days: u32,
    /// Days to keep unclustered candidates before pruning. Default: 30.
    #[serde(default = "default_candidate_retention_days")]
    pub candidate_retention_days: u32,
    /// Per-species overrides for min_cluster_size.
    /// Keys are scientific names (e.g., "Turdus migratorius").
    #[serde(default)]
    #[allow(dead_code)] // Deserialized from config; per-species filtering planned.
    pub species_overrides: std::collections::HashMap<String, SpeciesOverride>,
}

/// Per-species overrides for clustering thresholds.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Deserialized from config; will be used when per-species API filtering is added.
pub struct SpeciesOverride {
    /// Override min_cluster_size for this species.
    pub min_cluster_size: Option<u32>,
    /// Override min_distinct_days for this species.
    pub min_distinct_days: Option<u32>,
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

/// Audio snippet saving configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct SnippetConfig {
    /// Whether to save audio clips of detections. Default: true.
    #[serde(default = "default_snippet_enabled")]
    pub enabled: bool,
    /// Base directory for clip storage. Default: "./clips".
    #[serde(default = "default_snippet_dir")]
    pub clip_dir: String,
    /// Maximum retention age in days. 0 = unlimited. Default: 30.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    /// Maximum total disk usage in MB. 0 = unlimited. Default: 2048.
    #[serde(default = "default_max_disk_mb")]
    pub max_disk_mb: u64,
    /// Multiplier applied to `retention_days` for `first_ever` detections.
    /// Default 999 (effectively forever — set lower if you'd rather these
    /// age out eventually).
    #[serde(default = "default_first_ever_multiplier")]
    pub first_ever_multiplier: u32,
    /// Multiplier applied to `retention_days` for `first_season` detections. Default 8.
    #[serde(default = "default_first_season_multiplier")]
    pub first_season_multiplier: u32,
    /// Multiplier applied to `retention_days` for `first_week` detections. Default 4.
    #[serde(default = "default_first_week_multiplier")]
    pub first_week_multiplier: u32,
    /// Multiplier applied to `retention_days` for `first_day` detections. Default 2.
    #[serde(default = "default_first_day_multiplier")]
    pub first_day_multiplier: u32,
    /// Multiplier applied to `retention_days` for detections with rarity score >= 0.6. Default 2.
    #[serde(default = "default_high_score_multiplier")]
    pub high_score_multiplier: u32,
    /// Per-(species, calendar-day) quota: keep this many most-recent clips
    /// per species per UTC day. The keep set is the union of "most recent
    /// N" (this knob) and "top M by confidence" (see
    /// [`Self::per_species_per_day_top_confidence`]), so a species×day
    /// bucket retains up to N+M unique clips. The quota applies to every
    /// day in the retention window — the most-recent clips of *today* are
    /// always preserved, so a noisy phoebe day never wipes out the day's
    /// representative recordings. Reviewed-correct, first_ever,
    /// first_season, and first_week are exempt. Default 10. Set both
    /// quota knobs to 0 to disable per-day trimming entirely.
    #[serde(default = "default_per_species_per_day_recent")]
    pub per_species_per_day_recent: u32,
    /// Per-(species, calendar-day) quota: keep the top M clips by confidence
    /// per species per UTC day. Pairs with
    /// [`Self::per_species_per_day_recent`] (the keep set is the union).
    /// Default 10.
    #[serde(default = "default_per_species_per_day_top_confidence")]
    pub per_species_per_day_top_confidence: u32,
}

impl Default for SnippetConfig {
    fn default() -> Self {
        Self {
            enabled: default_snippet_enabled(),
            clip_dir: default_snippet_dir(),
            retention_days: default_retention_days(),
            max_disk_mb: default_max_disk_mb(),
            first_ever_multiplier: default_first_ever_multiplier(),
            first_season_multiplier: default_first_season_multiplier(),
            first_week_multiplier: default_first_week_multiplier(),
            first_day_multiplier: default_first_day_multiplier(),
            high_score_multiplier: default_high_score_multiplier(),
            per_species_per_day_recent: default_per_species_per_day_recent(),
            per_species_per_day_top_confidence: default_per_species_per_day_top_confidence(),
        }
    }
}

/// HTTP API configuration.
#[derive(Debug, Deserialize)]
pub struct ApiConfig {
    /// Socket address to bind (e.g., "0.0.0.0:8080").
    #[serde(default = "default_api_bind")]
    pub bind: String,
    /// External base URL for detection links in MQTT events (e.g., "http://192.168.1.132:8080").
    /// Defaults to `http://{bind}` if not set.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Minimum confidence for displaying detections in the UI and SSE feed.
    /// Detections below this are still captured in the database. Default: 0.65.
    #[serde(default = "default_display_min_confidence")]
    pub display_min_confidence: f32,
    /// Show detections whose species is not in the BirdNET range model.
    /// Default: true.
    #[serde(default = "default_show_range_unverified")]
    pub show_range_unverified: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: default_api_bind(),
            base_url: None,
            display_min_confidence: default_display_min_confidence(),
            show_range_unverified: true,
        }
    }
}

fn default_show_range_unverified() -> bool { true }



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
fn default_display_min_confidence() -> f32 {
    0.65
}
fn default_individual_threshold() -> f32 {
    0.85
}
fn default_snippet_enabled() -> bool {
    true
}
fn default_snippet_dir() -> String {
    "./clips".into()
}
fn default_retention_days() -> u32 {
    30
}
fn default_max_disk_mb() -> u64 {
    2048
}
fn default_first_ever_multiplier() -> u32 {
    999
}
fn default_first_season_multiplier() -> u32 {
    8
}
fn default_first_week_multiplier() -> u32 {
    4
}
fn default_first_day_multiplier() -> u32 {
    2
}
fn default_high_score_multiplier() -> u32 {
    2
}
fn default_per_species_per_day_recent() -> u32 {
    10
}
fn default_per_species_per_day_top_confidence() -> u32 {
    10
}
fn default_birdnet_stride() -> f32 {
    1.0
}
fn default_cluster_merge_threshold() -> f32 {
    0.70
}
fn default_min_cluster_size() -> u32 {
    5
}
fn default_min_distinct_days() -> u32 {
    2
}
fn default_candidate_retention_days() -> u32 {
    30
}

/// Presence confirmation: require repeated detections before alerting.
///
/// A single 3-second window claiming a species at moderate confidence is
/// weak evidence. Requiring N detections within T minutes dramatically
/// cuts false positives — especially important for rare-bird alerts where
/// a false alarm wastes real effort.
#[derive(Debug, Clone, Deserialize)]
pub struct PresenceConfig {
    /// Number of detections of the same species required within the window
    /// before broadcasting a confirmed-presence event. Default: 2.
    /// Set to 1 to disable (every detection broadcasts immediately).
    #[serde(default = "default_presence_min_detections")]
    pub min_detections: u32,
    /// Sliding window in minutes. Detections older than this are pruned
    /// from the accumulator. Default: 10.
    #[serde(default = "default_presence_window_minutes")]
    pub window_minutes: u32,
    /// Confidence threshold that bypasses the repeat-detection requirement.
    /// A single detection at or above this confidence broadcasts immediately
    /// without waiting for additional hits. Useful for high-confidence
    /// detections of species that vocalize once and leave.
    /// Default: None (disabled — all detections require N hits).
    #[serde(default)]
    pub immediate_threshold: Option<f32>,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            min_detections: default_presence_min_detections(),
            window_minutes: default_presence_window_minutes(),
            immediate_threshold: None,
        }
    }
}

fn default_presence_min_detections() -> u32 {
    2
}
fn default_presence_window_minutes() -> u32 {
    10
}

/// MQTT publishing configuration. When absent, MQTT is disabled.
#[derive(Debug, Clone, Deserialize)]
pub struct MqttConfig {
    /// Broker hostname or IP.
    pub host: String,
    /// Broker port. Default: 1883.
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    /// Username for broker authentication.
    pub username: Option<String>,
    /// Password for broker authentication.
    pub password: Option<String>,
    /// MQTT client ID. Default: "sitta-{station_id}".
    pub client_id: Option<String>,
    /// Minimum confidence for first-of-day messages. Default: 0.75.
    #[serde(default = "default_first_of_day_confidence")]
    pub first_of_day_min_confidence: f32,
    /// Enable Home Assistant MQTT auto-discovery. Default: true.
    #[serde(default = "default_ha_discovery")]
    pub homeassistant_discovery: bool,
    /// Home Assistant discovery prefix. Default: "homeassistant".
    #[serde(default = "default_ha_prefix")]
    pub homeassistant_prefix: String,
}

fn default_mqtt_port() -> u16 {
    1883
}
fn default_first_of_day_confidence() -> f32 {
    0.75
}
fn default_ha_discovery() -> bool {
    true
}
fn default_ha_prefix() -> String {
    "homeassistant".into()
}

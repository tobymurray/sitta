//! Runtime-changeable settings with disk persistence.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Settings that can be changed at runtime without restarting.
/// Stored in an `ArcSwap` for lock-free reads on the inference hot path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub station_name: String,
    pub station_latitude: Option<f64>,
    pub station_longitude: Option<f64>,
    /// IANA timezone (e.g., "America/Toronto"). Derived from lat/lon if not set.
    pub timezone: String,
    /// Base URL for custom species images. If set, the UI tries
    /// `{url}/{Scientific_name}.jpg` before falling back to Wikipedia.
    /// Example: "http://localhost:8080/images" or "https://my-cdn.com/birds"
    #[serde(default)]
    pub species_image_url: Option<String>,
    /// Minimum confidence for displaying detections in the UI and SSE feed.
    /// Detections below this are still captured in the database.
    pub display_min_confidence: f32,
    pub birdnet_min_confidence: Option<f32>,
    pub birdnet_top_k: Option<usize>,
    pub birdnet_meta_threshold: Option<f32>,
    pub birdnet_force_allow: Option<Vec<String>>,
    pub perch_min_confidence: Option<f32>,
    pub perch_top_k: Option<usize>,
    /// Show detections whose species is not in the BirdNET range model
    /// (Perch-only species that bypass the geographic filter). Default: true.
    #[serde(default = "default_show_range_unverified")]
    pub show_range_unverified: bool,
    /// Number of detections of the same species required within the window
    /// before broadcasting. 1 = immediate (no confirmation). Default: 2.
    #[serde(default = "default_presence_min_detections")]
    pub presence_min_detections: u32,
    /// Presence confirmation window in minutes. Default: 10.
    #[serde(default = "default_presence_window_minutes")]
    pub presence_window_minutes: u32,
    /// Confidence threshold that bypasses presence confirmation.
    /// A single detection at or above this confidence broadcasts immediately.
    /// None = disabled (all detections require N hits). Example: 0.90.
    #[serde(default)]
    pub presence_immediate_threshold: Option<f32>,
}

fn default_show_range_unverified() -> bool { true }
fn default_presence_min_detections() -> u32 { 2 }
fn default_presence_window_minutes() -> u32 { 10 }

/// Partial update for PUT /api/v1/settings.
/// All fields are optional — only present fields are applied.
#[derive(Debug, Deserialize)]
pub struct SettingsUpdate {
    pub station_name: Option<String>,
    pub station_latitude: Option<f64>,
    pub station_longitude: Option<f64>,
    pub timezone: Option<String>,
    pub species_image_url: Option<String>,
    pub display_min_confidence: Option<f32>,
    pub birdnet_min_confidence: Option<f32>,
    pub birdnet_top_k: Option<usize>,
    pub birdnet_meta_threshold: Option<f32>,
    pub birdnet_force_allow: Option<Vec<String>>,
    pub perch_min_confidence: Option<f32>,
    pub perch_top_k: Option<usize>,
    pub show_range_unverified: Option<bool>,
    pub presence_min_detections: Option<u32>,
    pub presence_window_minutes: Option<u32>,
    pub presence_immediate_threshold: Option<f32>,
}

/// Read-only config snapshot for values that require a restart to change.
#[derive(Debug, Clone, Serialize)]
pub struct InitialConfig {
    pub station_id: String,
    pub mqtt_host: Option<String>,
    pub mqtt_port: Option<u16>,
    pub birdnet_model_path: Option<String>,
    pub birdnet_labels_path: Option<String>,
    pub birdnet_meta_model_path: Option<String>,
    pub perch_model_path: Option<String>,
    pub perch_labels_path: Option<String>,
    pub store_path: String,
    pub api_bind: String,
    /// Minimum cluster size for candidate enrollment suggestions.
    #[serde(skip_serializing)]
    pub min_cluster_size: i64,
    /// Minimum distinct days for candidate enrollment suggestions.
    #[serde(skip_serializing)]
    pub min_distinct_days: i64,
}

/// Full response for GET /api/v1/settings.
#[derive(Serialize)]
pub struct SettingsResponse {
    #[serde(flatten)]
    pub runtime: RuntimeSettings,
    #[serde(rename = "_initial")]
    pub initial: InitialConfig,
    #[serde(rename = "_restart_required")]
    pub restart_required: Vec<&'static str>,
}

pub const RESTART_REQUIRED_FIELDS: &[&str] = &[
    "station_id",
    "birdnet_model_path",
    "birdnet_labels_path",
    "birdnet_meta_model_path",
    "perch_model_path",
    "perch_labels_path",
    "store_path",
    "api_bind",
];

/// Apply a partial update to the current settings. Returns the merged result
/// and a list of field names that were changed.
pub fn apply_update(current: &RuntimeSettings, update: &SettingsUpdate) -> (RuntimeSettings, Vec<&'static str>) {
    let mut merged = current.clone();
    let mut changed = Vec::new();

    if let Some(ref v) = update.station_name
        && *v != merged.station_name
    {
        merged.station_name = v.clone();
        changed.push("station_name");
    }
    if let Some(v) = update.station_latitude
        && merged.station_latitude != Some(v)
    {
        merged.station_latitude = Some(v);
        changed.push("station_latitude");
    }
    if let Some(v) = update.station_longitude
        && merged.station_longitude != Some(v)
    {
        merged.station_longitude = Some(v);
        changed.push("station_longitude");
    }
    if let Some(ref v) = update.timezone
        && *v != merged.timezone
    {
        merged.timezone = v.clone();
        changed.push("timezone");
    }
    if let Some(ref v) = update.species_image_url
        && merged.species_image_url.as_ref() != Some(v)
    {
        merged.species_image_url = Some(v.clone());
        changed.push("species_image_url");
    }
    if let Some(v) = update.display_min_confidence
        && (merged.display_min_confidence - v).abs() > f32::EPSILON
    {
        merged.display_min_confidence = v;
        changed.push("display_min_confidence");
    }
    if let Some(v) = update.birdnet_min_confidence
        && merged.birdnet_min_confidence != Some(v)
    {
        merged.birdnet_min_confidence = Some(v);
        changed.push("birdnet_min_confidence");
    }
    if let Some(v) = update.birdnet_top_k
        && merged.birdnet_top_k != Some(v)
    {
        merged.birdnet_top_k = Some(v);
        changed.push("birdnet_top_k");
    }
    if let Some(v) = update.birdnet_meta_threshold
        && merged.birdnet_meta_threshold != Some(v)
    {
        merged.birdnet_meta_threshold = Some(v);
        changed.push("birdnet_meta_threshold");
    }
    if let Some(ref v) = update.birdnet_force_allow
        && merged.birdnet_force_allow.as_ref() != Some(v)
    {
        merged.birdnet_force_allow = Some(v.clone());
        changed.push("birdnet_force_allow");
    }
    if let Some(v) = update.perch_min_confidence
        && merged.perch_min_confidence != Some(v)
    {
        merged.perch_min_confidence = Some(v);
        changed.push("perch_min_confidence");
    }
    if let Some(v) = update.perch_top_k
        && merged.perch_top_k != Some(v)
    {
        merged.perch_top_k = Some(v);
        changed.push("perch_top_k");
    }
    if let Some(v) = update.show_range_unverified
        && v != merged.show_range_unverified
    {
        merged.show_range_unverified = v;
        changed.push("show_range_unverified");
    }
    if let Some(v) = update.presence_min_detections
        && v != merged.presence_min_detections
    {
        merged.presence_min_detections = v;
        changed.push("presence_min_detections");
    }
    if let Some(v) = update.presence_window_minutes
        && v != merged.presence_window_minutes
    {
        merged.presence_window_minutes = v;
        changed.push("presence_window_minutes");
    }
    if let Some(v) = update.presence_immediate_threshold
        && merged.presence_immediate_threshold != Some(v)
    {
        merged.presence_immediate_threshold = Some(v);
        changed.push("presence_immediate_threshold");
    }

    (merged, changed)
}

/// Persist runtime settings back to the TOML config file, preserving
/// comments and formatting via `toml_edit`.
pub fn persist_to_toml(path: &Path, settings: &RuntimeSettings) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config: {e}"))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("failed to parse config: {e}"))?;

    // Station
    if let Some(station) = doc.get_mut("station").and_then(|v| v.as_table_mut()) {
        station["name"] = toml_edit::value(&settings.station_name);
        if let Some(lat) = settings.station_latitude {
            station["latitude"] = toml_edit::value(round4(lat));
        }
        if let Some(lon) = settings.station_longitude {
            station["longitude"] = toml_edit::value(round4(lon));
        }
        if !settings.timezone.is_empty() {
            station["timezone"] = toml_edit::value(&settings.timezone);
        }
    }

    // Display settings — stored under [api]
    if let Some(api) = doc.get_mut("api").and_then(|v| v.as_table_mut()) {
        api["display_min_confidence"] = toml_edit::value(f64::from(settings.display_min_confidence));
        api["show_range_unverified"] = toml_edit::value(settings.show_range_unverified);
    }

    // Presence confirmation — stored under [presence]
    {
        let table = doc.entry("presence").or_insert_with(|| {
            toml_edit::Item::Table(toml_edit::Table::new())
        });
        if let Some(t) = table.as_table_mut() {
            t["min_detections"] = toml_edit::value(i64::from(settings.presence_min_detections));
            t["window_minutes"] = toml_edit::value(i64::from(settings.presence_window_minutes));
            if let Some(v) = settings.presence_immediate_threshold {
                t["immediate_threshold"] = toml_edit::value(f64::from(v));
            } else {
                t.remove("immediate_threshold");
            }
        }
    }

    // BirdNET inference
    if let Some(birdnet) = doc
        .get_mut("inference")
        .and_then(|v| v.as_table_mut())
        .and_then(|t| t.get_mut("birdnet"))
        .and_then(|v| v.as_table_mut())
    {
        if let Some(v) = settings.birdnet_min_confidence {
            birdnet["min_confidence"] = toml_edit::value(f64::from(v));
        }
        if let Some(v) = settings.birdnet_top_k {
            birdnet["top_k"] = toml_edit::value(v as i64);
        }
        if let Some(v) = settings.birdnet_meta_threshold {
            birdnet["meta_threshold"] = toml_edit::value(f64::from(v));
        }
        if let Some(ref v) = settings.birdnet_force_allow {
            let arr = v.iter().map(|s| toml_edit::Value::from(s.as_str())).collect::<toml_edit::Array>();
            birdnet["force_allow"] = toml_edit::value(arr);
        }
    }

    // Perch inference
    if let Some(perch) = doc
        .get_mut("inference")
        .and_then(|v| v.as_table_mut())
        .and_then(|t| t.get_mut("perch"))
        .and_then(|v| v.as_table_mut())
    {
        if let Some(v) = settings.perch_min_confidence {
            perch["min_confidence"] = toml_edit::value(f64::from(v));
        }
        if let Some(v) = settings.perch_top_k {
            perch["top_k"] = toml_edit::value(v as i64);
        }
    }

    std::fs::write(path, doc.to_string())
        .map_err(|e| format!("failed to write config: {e}"))?;
    Ok(())
}

/// Persist audio sources to the TOML config file, replacing the [[audio.sources]] array.
pub fn persist_sources_to_toml(
    path: &Path,
    sources: &[sitta_audio::source::SourceConfig],
) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config: {e}"))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("failed to parse config: {e}"))?;

    if let Some(audio) = doc.get_mut("audio").and_then(|v| v.as_table_mut()) {
        // Rebuild the sources array from scratch.
        let mut arr = toml_edit::ArrayOfTables::new();
        for source in sources {
            let mut table = toml_edit::Table::new();
            match source {
                sitta_audio::source::SourceConfig::Rtsp(r) => {
                    table["type"] = toml_edit::value("rtsp");
                    table["name"] = toml_edit::value(&r.name);
                    table["url"] = toml_edit::value(&r.url);
                    table["transport"] = toml_edit::value(r.transport.as_str());
                    table["sample_rate"] = toml_edit::value(r.sample_rate as i64);
                    table["channels"] = toml_edit::value(r.channels as i64);
                }
                sitta_audio::source::SourceConfig::Local(l) => {
                    table["type"] = toml_edit::value("local");
                    table["name"] = toml_edit::value(&l.name);
                    table["device"] = toml_edit::value(&l.device);
                }
                sitta_audio::source::SourceConfig::Remote(r) => {
                    table["type"] = toml_edit::value("remote");
                    table["name"] = toml_edit::value(&r.name);
                    table["url"] = toml_edit::value(&r.url);
                }
            }
            arr.push(table);
        }
        audio.insert("sources", toml_edit::Item::ArrayOfTables(arr));
    }

    std::fs::write(path, doc.to_string())
        .map_err(|e| format!("failed to write config: {e}"))?;
    Ok(())
}

/// MQTT configuration for the API layer (read/write via dedicated endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_first_of_day_confidence")]
    pub first_of_day_min_confidence: f32,
    #[serde(default = "default_ha_discovery")]
    pub homeassistant_discovery: bool,
    #[serde(default = "default_ha_prefix")]
    pub homeassistant_prefix: String,
}

fn default_mqtt_port() -> u16 { 1883 }
fn default_first_of_day_confidence() -> f32 { 0.75 }
fn default_ha_discovery() -> bool { true }
fn default_ha_prefix() -> String { "homeassistant".into() }

/// Read the current [mqtt] section from config.toml.
pub fn read_mqtt_from_toml(path: &Path) -> MqttSettings {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return MqttSettings {
            enabled: false,
            host: String::new(),
            port: 1883,
            username: None,
            password: None,
            first_of_day_min_confidence: 0.75,
            homeassistant_discovery: true,
            homeassistant_prefix: "homeassistant".into(),
        },
    };
    let doc = match content.parse::<toml_edit::DocumentMut>() {
        Ok(d) => d,
        Err(_) => return MqttSettings {
            enabled: false, host: String::new(), port: 1883, username: None,
            password: None, first_of_day_min_confidence: 0.75,
            homeassistant_discovery: true, homeassistant_prefix: "homeassistant".into(),
        },
    };

    let Some(mqtt) = doc.get("mqtt").and_then(|v| v.as_table()) else {
        return MqttSettings {
            enabled: false, host: String::new(), port: 1883, username: None,
            password: None, first_of_day_min_confidence: 0.75,
            homeassistant_discovery: true, homeassistant_prefix: "homeassistant".into(),
        };
    };

    MqttSettings {
        enabled: true,
        host: mqtt.get("host").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        port: mqtt.get("port").and_then(|v| v.as_integer()).unwrap_or(1883) as u16,
        username: mqtt.get("username").and_then(|v| v.as_str()).map(String::from),
        password: mqtt.get("password").and_then(|v| v.as_str()).map(String::from),
        first_of_day_min_confidence: mqtt.get("first_of_day_min_confidence")
            .and_then(|v| v.as_float()).unwrap_or(0.75) as f32,
        homeassistant_discovery: mqtt.get("homeassistant_discovery")
            .and_then(|v| v.as_bool()).unwrap_or(true),
        homeassistant_prefix: mqtt.get("homeassistant_prefix")
            .and_then(|v| v.as_str()).unwrap_or("homeassistant").to_string(),
    }
}

/// Write the [mqtt] section to config.toml. If enabled=false, remove the section.
pub fn persist_mqtt_to_toml(path: &Path, mqtt: &MqttSettings) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config: {e}"))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("failed to parse config: {e}"))?;

    if !mqtt.enabled || mqtt.host.is_empty() {
        doc.remove("mqtt");
    } else {
        let mut table = toml_edit::Table::new();
        table["host"] = toml_edit::value(&mqtt.host);
        table["port"] = toml_edit::value(i64::from(mqtt.port));
        if let Some(ref u) = mqtt.username {
            table["username"] = toml_edit::value(u);
        }
        if let Some(ref p) = mqtt.password {
            table["password"] = toml_edit::value(p);
        }
        table["first_of_day_min_confidence"] = toml_edit::value(f64::from(mqtt.first_of_day_min_confidence));
        table["homeassistant_discovery"] = toml_edit::value(mqtt.homeassistant_discovery);
        table["homeassistant_prefix"] = toml_edit::value(&mqtt.homeassistant_prefix);
        doc["mqtt"] = toml_edit::Item::Table(table);
    }

    std::fs::write(path, doc.to_string())
        .map_err(|e| format!("failed to write config: {e}"))?;
    Ok(())
}

/// Round to 4 decimal places (~11m precision, sufficient for station location).
pub fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Derive an IANA timezone from latitude/longitude using a simple offset heuristic.
/// Returns a fixed-offset timezone string like "Etc/GMT+5". For proper timezone
/// resolution (daylight saving, political boundaries), a timezone database lookup
/// would be needed, but this is a reasonable default.
pub fn timezone_from_coords(lat: f64, lon: f64) -> String {
    // Rough timezone from longitude: every 15 degrees = 1 hour offset.
    let _ = lat; // latitude doesn't affect timezone offset significantly
    let offset_hours = (lon / 15.0).round() as i32;
    // Etc/GMT signs are inverted: Etc/GMT-5 means UTC+5
    if offset_hours >= 0 {
        format!("Etc/GMT-{offset_hours}")
    } else {
        format!("Etc/GMT+{}", -offset_hours)
    }
}

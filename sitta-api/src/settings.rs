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
    /// Minimum confidence for displaying detections in the UI and SSE feed.
    /// Detections below this are still captured in the database.
    pub display_min_confidence: f32,
    pub birdnet_min_confidence: Option<f32>,
    pub birdnet_top_k: Option<usize>,
    pub birdnet_meta_threshold: Option<f32>,
    pub birdnet_force_allow: Option<Vec<String>>,
    pub perch_min_confidence: Option<f32>,
    pub perch_top_k: Option<usize>,
}

/// Partial update for PUT /api/v1/settings.
/// All fields are optional — only present fields are applied.
#[derive(Debug, Deserialize)]
pub struct SettingsUpdate {
    pub station_name: Option<String>,
    pub station_latitude: Option<f64>,
    pub station_longitude: Option<f64>,
    pub display_min_confidence: Option<f32>,
    pub birdnet_min_confidence: Option<f32>,
    pub birdnet_top_k: Option<usize>,
    pub birdnet_meta_threshold: Option<f32>,
    pub birdnet_force_allow: Option<Vec<String>>,
    pub perch_min_confidence: Option<f32>,
    pub perch_top_k: Option<usize>,
}

/// Read-only config snapshot for values that require a restart to change.
#[derive(Debug, Clone, Serialize)]
pub struct InitialConfig {
    pub station_id: String,
    pub birdnet_model_path: Option<String>,
    pub birdnet_labels_path: Option<String>,
    pub birdnet_meta_model_path: Option<String>,
    pub perch_model_path: Option<String>,
    pub perch_labels_path: Option<String>,
    pub store_path: String,
    pub api_bind: String,
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
            station["latitude"] = toml_edit::value(lat);
        }
        if let Some(lon) = settings.station_longitude {
            station["longitude"] = toml_edit::value(lon);
        }
    }

    // Display threshold — stored under [api]
    if let Some(api) = doc.get_mut("api").and_then(|v| v.as_table_mut()) {
        api["display_min_confidence"] = toml_edit::value(f64::from(settings.display_min_confidence));
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

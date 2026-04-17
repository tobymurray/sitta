//! Plain structs for database rows and insert parameters.

use uuid::Uuid;

/// Helper to convert a `Uuid` to the 16-byte slice sqlx binds as BLOB.
pub fn uuid_bytes(id: &Uuid) -> &[u8] {
    id.as_bytes().as_slice()
}

/// Helper to convert a `Vec<u8>` from a BLOB column back to `Uuid`.
pub fn uuid_from_blob(blob: Vec<u8>) -> Result<Uuid, crate::StoreError> {
    let bytes: [u8; 16] = blob
        .try_into()
        .map_err(|v: Vec<u8>| crate::StoreError::InvalidUuid(v.len()))?;
    Ok(Uuid::from_bytes(bytes))
}

/// Parameters for seeding a station.
pub struct NewStation<'a> {
    pub id: &'a Uuid,
    pub name: &'a str,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Parameters for seeding an audio source.
pub struct NewAudioSource<'a> {
    pub id: &'a Uuid,
    pub station_id: &'a Uuid,
    pub name: &'a str,
    pub source_type: &'a str,
    pub uri: Option<&'a str>,
    pub sample_rate: i64,
    pub channels: i64,
}

/// Parameters for seeding a model. Returns the assigned INTEGER PK.
pub struct NewModel<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub sample_rate: i64,
    pub window_samples: i64,
    pub has_embeddings: bool,
    pub embedding_dim: Option<i64>,
}

/// A single label entry for bulk seeding.
pub struct NewLabel<'a> {
    pub model_id: i64,
    pub label_index: i64,
    pub scientific_name: Option<&'a str>,
    pub common_name: &'a str,
    pub label_type: &'a str,
    pub taxon_code: Option<&'a str>,
}

/// Parameters for inserting a detection.
pub struct NewDetection<'a> {
    pub id: &'a Uuid,
    pub station_id: &'a Uuid,
    pub source_id: Option<&'a Uuid>,
    pub model_id: i64,
    pub label_id: i64,
    pub detected_at: i64,
    pub confidence: f64,
    pub snippet_path: Option<&'a str>,
    pub snippet_duration_ms: Option<i64>,
    pub snippet_sample_rate: Option<i64>,
    pub metadata: Option<&'a str>,
}

/// A secondary prediction for a detection.
pub struct NewPrediction {
    pub rank: i64,
    pub label_id: i64,
    pub confidence: f64,
}

// ── Read types (returned by query methods) ──────────────────────

/// A detection row with joined label/model/source info.
pub struct DetectionRow {
    pub id: Vec<u8>,
    pub detected_at: i64,
    pub confidence: f64,
    pub snippet_path: Option<String>,
    pub metadata: Option<String>,
    pub scientific_name: Option<String>,
    pub common_name: String,
    pub taxon_code: Option<String>,
    pub model_name: String,
    pub model_version: String,
    pub source_name: Option<String>,
}

/// A secondary prediction row with label info.
pub struct PredictionRow {
    pub rank: i64,
    pub confidence: f64,
    pub scientific_name: Option<String>,
    pub common_name: String,
}

/// Aggregated species summary for a date range.
pub struct SpeciesSummaryRow {
    pub scientific_name: Option<String>,
    pub common_name: String,
    pub taxon_code: Option<String>,
    pub detection_count: i64,
    pub last_detected_at: i64,
    pub avg_confidence: f64,
}

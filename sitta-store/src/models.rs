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

/// Parameters for enrolling a new individual.
pub struct NewIndividual<'a> {
    pub id: &'a Uuid,
    pub scientific_name: &'a str,
    pub label: &'a str,
    pub reference_embedding: Option<&'a [u8]>,
    pub reference_embedding_dim: Option<i64>,
    pub enrolled_at: i64,
    pub notes: Option<&'a str>,
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
    pub has_embedding: bool,
}

/// A secondary prediction row with label info.
pub struct PredictionRow {
    pub rank: i64,
    pub confidence: f64,
    pub scientific_name: Option<String>,
    pub common_name: String,
}

/// An enrolled individual row.
pub struct IndividualRow {
    pub id: Vec<u8>,
    pub scientific_name: String,
    pub label: String,
    pub reference_embedding: Option<Vec<u8>>,
    pub reference_embedding_dim: Option<i64>,
    pub enrolled_at: i64,
    pub notes: Option<String>,
}

/// A detection review row.
pub struct ReviewRow {
    pub detection_id: Vec<u8>,
    pub status: String,
    pub reviewed_at: i64,
    pub comment: Option<String>,
}

/// Hourly detection count for a single species in a single hour bucket.
pub struct HourlyActivityRow {
    pub common_name: String,
    pub scientific_name: Option<String>,
    pub taxon_code: Option<String>,
    pub hour_bucket: i64,
    pub count: i64,
}

/// Parameters for creating a new candidate cluster.
pub struct NewCluster<'a> {
    pub scientific_name: &'a str,
    pub centroid: &'a [u8],
    pub centroid_dim: i64,
    pub member_count: i64,
    pub distinct_days: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// A candidate embedding awaiting clustering.
pub struct CandidateRow {
    pub detection_id: Vec<u8>,
    pub scientific_name: String,
    pub embedding: Vec<u8>,
    pub cluster_id: Option<i64>,
    pub created_at: i64,
}

/// A discovered cluster of candidate embeddings.
pub struct ClusterRow {
    pub id: i64,
    pub scientific_name: String,
    pub centroid: Vec<u8>,
    pub centroid_dim: i64,
    pub member_count: i64,
    pub distinct_days: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub status: String,
    pub individual_id: Option<Vec<u8>>,
}

/// Hourly detection count for a species across all time (UTC hour bucket).
pub struct SpeciesHourlyProfileRow {
    pub hour_utc: i64,
    pub count: i64,
}

/// A notable (high-rarity) detection for a species.
pub struct NotableDetectionRow {
    pub detection_id: Vec<u8>,
    pub detected_at: i64,
    pub confidence: f64,
    pub score: f64,
    pub first_ever: bool,
    pub first_season: bool,
    pub first_week: bool,
    pub first_day: bool,
}

/// Monthly detection count for a species (calendar month 1-12).
pub struct SpeciesMonthlyRow {
    pub month: i64,
    pub count: i64,
}

/// Aggregate stats for a single species across all detections.
pub struct SpeciesStatsRow {
    pub common_name: String,
    pub total: i64,
    pub first_detected_at: i64,
    pub last_detected_at: i64,
    pub avg_confidence: f64,
    pub distinct_days: i64,
}

/// Rarity score breakdown for a detection.
pub struct RarityRow {
    pub detection_id: Vec<u8>,
    pub score: f64,
    pub first_ever: bool,
    pub first_season: bool,
    pub first_week: bool,
    pub first_day: bool,
    pub days_since_last: Option<i64>,
    pub local_count: i64,
    pub range_score: Option<f64>,
    pub temporal_score: f64,
}

/// Parameters for inserting a rarity score.
pub struct NewRarity<'a> {
    pub detection_id: &'a uuid::Uuid,
    pub score: f64,
    pub first_ever: bool,
    pub first_season: bool,
    pub first_week: bool,
    pub first_day: bool,
    pub days_since_last: Option<i64>,
    pub local_count: i64,
    pub range_score: Option<f64>,
    pub temporal_score: f64,
}

// ── Session / effort tracking ──────────────────────────────────

/// Parameters for starting a new source session.
pub struct NewSession<'a> {
    pub id: &'a Uuid,
    pub source_id: &'a Uuid,
    pub started_at: i64,
}

/// A source session row with joined source name.
pub struct SessionRow {
    pub id: Vec<u8>,
    pub source_id: Vec<u8>,
    pub source_name: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub end_reason: Option<String>,
    pub chunks_received: i64,
}

/// Per-source effort summary for a time range.
pub struct SourceEffortRow {
    pub source_name: String,
    pub total_seconds: f64,
    pub session_count: i64,
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

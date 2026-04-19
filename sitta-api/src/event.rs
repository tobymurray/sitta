//! Detection event type shared between inference producers and API consumers.

use serde::Serialize;

/// A detection event for live streaming (SSE) and API responses.
///
/// Constructed in the inference pipeline after a successful database insert,
/// then broadcast to all connected clients. Also used as the JSON shape for
/// REST responses.
#[derive(Debug, Clone, Serialize)]
pub struct DetectionEvent {
    /// UUIDv7 detection ID (hyphenated string).
    pub id: String,
    /// ISO 8601 timestamp of the detection.
    pub detected_at: String,
    /// Station identifier from config.
    pub station_id: String,
    /// Audio source name (e.g., "north_paddock"), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    /// Short model identifier (e.g., "birdnet").
    pub model: String,
    /// Model version (e.g., "2.4").
    pub model_version: String,
    /// Primary species classification.
    pub species: SpeciesInfo,
    /// Confidence of the primary classification, [0.0, 1.0].
    pub confidence: f32,
    /// Ranked alternative predictions (rank 1 = second-best).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Alternative>,
    /// Whether an embedding vector was stored for this detection.
    pub has_embedding: bool,
    /// Whether an audio clip is being saved for this detection.
    pub has_audio: bool,
    /// Individual match info, if this detection matched a known individual.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub individual: Option<IndividualInfo>,
    /// Rarity scoring breakdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rarity: Option<RarityInfo>,
}

/// Species identification within a detection event.
#[derive(Debug, Clone, Serialize)]
pub struct SpeciesInfo {
    pub scientific_name: String,
    pub common_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taxon_code: Option<String>,
}

/// Individual match info within a detection event.
#[derive(Debug, Clone, Serialize)]
pub struct IndividualInfo {
    pub individual_id: String,
    pub label: String,
    pub similarity: f32,
}

/// Rarity scoring breakdown for a detection.
#[derive(Debug, Clone, Serialize)]
pub struct RarityInfo {
    /// Composite rarity score, 0.0 (common) to 1.0 (extremely rare).
    pub score: f32,
    /// First-ever detection of this species at this station.
    pub first_ever: bool,
    /// First detection of this species this meteorological season.
    pub first_season: bool,
    /// First detection of this species this ISO week.
    pub first_week: bool,
    /// First detection of this species today.
    pub first_day: bool,
    /// Days since the species was last detected (None if first_ever).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_since_last: Option<i64>,
    /// Total prior detections of this species at this station.
    pub local_count: i64,
    /// BirdNET meta-model location score (0.0–1.0). None if range filter is not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_score: Option<f32>,
    /// How unusual the detection hour is for this species (0.0 = typical, 1.0 = very unusual).
    pub temporal_score: f32,
}

/// A secondary prediction within a detection event.
#[derive(Debug, Clone, Serialize)]
pub struct Alternative {
    pub rank: u32,
    pub scientific_name: String,
    pub common_name: String,
    pub confidence: f32,
}

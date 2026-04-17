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
}

/// Species identification within a detection event.
#[derive(Debug, Clone, Serialize)]
pub struct SpeciesInfo {
    pub scientific_name: String,
    pub common_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taxon_code: Option<String>,
}

/// A secondary prediction within a detection event.
#[derive(Debug, Clone, Serialize)]
pub struct Alternative {
    pub rank: u32,
    pub scientific_name: String,
    pub common_name: String,
    pub confidence: f32,
}

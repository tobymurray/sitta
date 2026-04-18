use std::collections::HashMap;
use std::sync::Arc;

use sitta_api::event::{Alternative, DetectionEvent, IndividualInfo, SpeciesInfo};
use sitta_audio::chunk::AudioChunk;
use sitta_inference::model::Classification;
use sitta_store::db::Database;
use sitta_store::matcher::IndividualMatcher;
use sitta_store::models::{NewDetection, NewPrediction};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::seed::parse_model_name;

/// Shared context for persisting detections from any consumer.
#[derive(Clone)]
pub struct PersistCtx {
    pub db: Database,
    /// (model_db_id, label_index) → label_db_id
    pub label_cache: Arc<HashMap<(i64, i64), i64>>,
    /// classifier display name → model_db_id
    pub model_ids: Arc<HashMap<String, i64>>,
    /// source display name → source UUID
    pub source_ids: Arc<HashMap<String, Uuid>>,
    pub station_id: Uuid,
    /// Broadcast channel for live detection events (SSE, MQTT, etc.).
    pub detection_tx: broadcast::Sender<DetectionEvent>,
    /// Individual matcher for Perch embeddings. None if no Perch configured.
    pub matcher: Option<Arc<IndividualMatcher>>,
}

/// Persist a detection, its secondary predictions, optional embedding,
/// and broadcast a live event to SSE subscribers.
pub async fn persist_detections(
    ctx: &PersistCtx,
    model_id: i64,
    classifier_name: &str,
    chunk: &AudioChunk,
    detections: &[Classification],
    embeddings: Option<&Vec<f32>>,
) {
    let top = match detections.first() {
        Some(d) => d,
        None => return,
    };
    let Some(&label_id) = ctx.label_cache.get(&(model_id, top.label_index as i64)) else {
        tracing::warn!(model_id, label_index = top.label_index, "Label not in cache");
        return;
    };

    let detection_id = Uuid::now_v7();
    let detected_at = chunk.captured_at.timestamp_millis();
    let source_id = ctx.source_ids.get(&chunk.source_name);

    if let Err(e) = ctx
        .db
        .insert_detection(&NewDetection {
            id: &detection_id,
            station_id: &ctx.station_id,
            source_id,
            model_id,
            label_id,
            detected_at,
            confidence: f64::from(top.confidence),
            snippet_path: None,
            snippet_duration_ms: None,
            snippet_sample_rate: None,
            metadata: None,
        })
        .await
    {
        tracing::error!(error = %e, "Failed to persist detection");
        return;
    }

    // Secondary predictions (rank 1+).
    let predictions: Vec<NewPrediction> = detections
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(r, p)| {
            let label_id = *ctx.label_cache.get(&(model_id, p.label_index as i64))?;
            Some(NewPrediction {
                rank: r as i64,
                label_id,
                confidence: f64::from(p.confidence),
            })
        })
        .collect();
    if let Err(e) = ctx.db.insert_predictions(&detection_id, &predictions).await {
        tracing::error!(error = %e, "Failed to persist predictions");
    }

    // Embedding (Perch path).
    let has_embedding = embeddings.is_some();
    if let Some(emb) = embeddings
        && let Err(e) = ctx.db.insert_embedding(&detection_id, emb).await
    {
        tracing::error!(error = %e, "Failed to persist embedding");
    }

    // Individual matching.
    let individual_match = if let Some(emb) = embeddings
        && let Some(matcher) = &ctx.matcher
    {
        matcher.find_match(&top.species.scientific_name, emb)
    } else {
        None
    };

    if let Some(ref m) = individual_match {
        let match_id = Uuid::now_v7();
        let now_ms = chunk.captured_at.timestamp_millis();
        if let Err(e) = ctx
            .db
            .insert_individual_match(&match_id, &m.individual_id, &detection_id, f64::from(m.similarity), now_ms)
            .await
        {
            tracing::error!(error = %e, "Failed to persist individual match");
        } else {
            tracing::info!(
                individual = %m.individual_label,
                similarity = format_args!("{:.3}", m.similarity),
                species = %top.species.common_name,
                "Individual matched"
            );
        }
    }

    // Broadcast to live subscribers (SSE, MQTT).
    let (model_name, model_version) = parse_model_name(classifier_name);
    let alternatives: Vec<Alternative> = detections
        .iter()
        .enumerate()
        .skip(1)
        .map(|(r, c)| Alternative {
            rank: r as u32,
            scientific_name: c.species.scientific_name.clone(),
            common_name: c.species.common_name.clone(),
            confidence: c.confidence,
        })
        .collect();

    let event = DetectionEvent {
        id: detection_id.to_string(),
        detected_at: chunk.captured_at.to_rfc3339(),
        station_id: ctx.station_id.to_string(),
        source_name: Some(chunk.source_name.clone()),
        model: model_name.to_string(),
        model_version: model_version.to_string(),
        species: SpeciesInfo {
            scientific_name: top.species.scientific_name.clone(),
            common_name: top.species.common_name.clone(),
            taxon_code: top.species.taxon_code.clone(),
        },
        confidence: top.confidence,
        alternatives,
        has_embedding,
        individual: individual_match.map(|m| IndividualInfo {
            individual_id: m.individual_id.to_string(),
            label: m.individual_label,
            similarity: m.similarity,
        }),
    };

    let _ = ctx.detection_tx.send(event);
}

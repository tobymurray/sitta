use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use sitta_api::event::{Alternative, DetectionEvent, IndividualInfo, SpeciesInfo};
use sitta_api::settings::RuntimeSettings;
use sitta_audio::chunk::AudioChunk;
use sitta_inference::model::Classification;
use sitta_store::db::Database;
use sitta_store::matcher::IndividualMatcher;
use sitta_store::models::{NewDetection, NewIndividual, NewPrediction};
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
    /// Runtime settings (for display_min_confidence threshold).
    pub settings: Arc<ArcSwap<RuntimeSettings>>,
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

    // Individual matching / auto-clustering.
    let individual_match = if let Some(emb) = embeddings
        && let Some(matcher) = &ctx.matcher
    {
        match matcher.find_match(&top.species.scientific_name, emb) {
            Some(m) => {
                // Known individual — record the match.
                let match_id = Uuid::now_v7();
                let now_ms = chunk.captured_at.timestamp_millis();
                if let Err(e) = ctx
                    .db
                    .insert_individual_match(&match_id, &m.individual_id, &detection_id, f64::from(m.similarity), now_ms)
                    .await
                {
                    tracing::error!(error = %e, "Failed to persist individual match");
                }
                Some(m)
            }
            None => {
                // New individual — auto-enroll from this detection's embedding.
                auto_enroll(ctx, &top.species, emb, &detection_id, chunk).await
            }
        }
    } else {
        None
    };

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

    // Only broadcast to live UI if above the display threshold.
    let display_threshold = ctx.settings.load().display_min_confidence;
    if top.confidence >= display_threshold {
        let _ = ctx.detection_tx.send(event);
    }
}

/// Auto-enroll a new individual when no match is found.
/// Creates an individual with an auto-generated label and reloads the matcher.
async fn auto_enroll(
    ctx: &PersistCtx,
    species: &sitta_inference::model::Species,
    embedding: &[f32],
    detection_id: &Uuid,
    chunk: &AudioChunk,
) -> Option<sitta_store::matcher::MatchResult> {
    let matcher = ctx.matcher.as_ref()?;

    // Count existing individuals for this species to generate a label.
    let existing = matcher.count_for_species(&species.scientific_name);
    let label = format!("{} #{}", species.common_name, existing + 1);

    let individual_id = Uuid::now_v7();
    let now_ms = chunk.captured_at.timestamp_millis();
    let emb_bytes: &[u8] = bytemuck::cast_slice(embedding);

    if let Err(e) = ctx
        .db
        .insert_individual(&NewIndividual {
            id: &individual_id,
            scientific_name: &species.scientific_name,
            label: &label,
            reference_embedding: Some(emb_bytes),
            reference_embedding_dim: Some(embedding.len() as i64),
            enrolled_at: now_ms,
            notes: Some("auto-enrolled"),
        })
        .await
    {
        tracing::error!(error = %e, "Failed to auto-enroll individual");
        return None;
    }

    // Record the match (similarity 1.0 — the reference IS this embedding).
    let match_id = Uuid::now_v7();
    let _ = ctx
        .db
        .insert_individual_match(&match_id, &individual_id, detection_id, 1.0, now_ms)
        .await;

    // Reload matcher cache so subsequent detections can match against this individual.
    if let Err(e) = matcher.reload().await {
        tracing::warn!(error = %e, "Failed to reload matcher after auto-enrollment");
    }

    tracing::info!(
        individual = %label,
        species = %species.common_name,
        "New individual auto-enrolled"
    );

    Some(sitta_store::matcher::MatchResult {
        individual_id,
        individual_label: label,
        similarity: 1.0,
    })
}


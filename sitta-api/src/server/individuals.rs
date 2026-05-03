//! Individual enrolment + candidate cluster management.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::{Deserialize, Serialize};

use sitta_store::models::uuid_from_blob;

use crate::server::{millis_to_rfc3339, ApiError, ApiState};

pub(super) async fn list_individuals(
    State(state): State<ApiState>,
    Query(params): Query<IndividualParams>,
) -> Result<Json<Vec<IndividualSummary>>, ApiError> {
    let rows = state
        .core
        .db
        .list_individuals(params.species.as_deref())
        .await?;

    let individuals = rows
        .into_iter()
        .filter_map(|r| {
            Some(IndividualSummary {
                id: uuid_from_blob(r.id).ok()?.to_string(),
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                label: r.label,
                enrolled_at: millis_to_rfc3339(r.enrolled_at)?,
                notes: r.notes,
            })
        })
        .collect();

    Ok(Json(individuals))
}

#[derive(Deserialize)]
pub(super) struct IndividualParams {
    species: Option<String>,
}

pub(super) async fn get_individual(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<IndividualSummary>, ApiError> {
    let uuid = id
        .parse::<uuid::Uuid>()
        .map_err(|_| ApiError::bad_request("invalid id"))?;
    let row = state
        .core
        .db
        .get_individual(uuid.as_bytes().as_slice())
        .await?
        .ok_or(ApiError::not_found("not found"))?;

    Ok(Json(IndividualSummary {
        id: uuid.to_string(),
        scientific_name: row.scientific_name,
        common_name: row.common_name,
        label: row.label,
        enrolled_at: millis_to_rfc3339(row.enrolled_at).unwrap_or_default(),
        notes: row.notes,
    }))
}

pub(super) async fn delete_all_individuals(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let deleted = state.core.db.delete_all_individuals().await?;

    // Reload matcher to clear the in-memory cache.
    if let Some(matcher) = &state.inference.matcher
        && let Err(e) = matcher.reload().await
    {
        tracing::warn!(error = %e, "Failed to reload matcher after bulk delete");
    }

    tracing::info!(deleted, "Deleted all individuals");
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

pub(super) async fn enroll_individual(
    State(state): State<ApiState>,
    Json(req): Json<EnrollRequest>,
) -> Result<Json<IndividualSummary>, ApiError> {
    let det_uuid = req
        .detection_id
        .parse::<uuid::Uuid>()
        .map_err(|_| ApiError::bad_request("invalid detection_id"))?;

    // Fetch the detection to get the species.
    let det = state
        .core
        .db
        .get_detection(det_uuid.as_bytes().as_slice())
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("detection not found"))?;

    // Fetch the embedding for this detection.
    let emb_blob = state
        .core
        .db
        .get_embedding_for_detection(det_uuid.as_bytes().as_slice())
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::bad_request(
            "detection has no embedding (only Perch detections have embeddings)",
        ))?;

    let individual_id = uuid::Uuid::now_v7();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let dim = (emb_blob.len() / 4) as i64;
    let scientific_name = det.scientific_name.unwrap_or_default();
    let common_name = Some(det.common_name);

    state
        .core
        .db
        .insert_individual(&sitta_store::models::NewIndividual {
            id: &individual_id,
            scientific_name: &scientific_name,
            label: &req.label,
            reference_embedding: Some(&emb_blob),
            reference_embedding_dim: Some(dim),
            enrolled_at: now_ms,
            notes: req.notes.as_deref(),
        })
        .await
        .map_err(ApiError::internal)?;

    // Reload the matcher cache so future detections see this individual.
    if let Some(matcher) = &state.inference.matcher
        && let Err(e) = matcher.reload().await
    {
        tracing::warn!(error = %e, "Failed to reload matcher after enrollment");
    }

    tracing::info!(
        individual = %individual_id,
        label = %req.label,
        species = %scientific_name,
        "Individual enrolled"
    );

    Ok(Json(IndividualSummary {
        id: individual_id.to_string(),
        scientific_name,
        common_name,
        label: req.label,
        enrolled_at: millis_to_rfc3339(now_ms).unwrap_or_default(),
        notes: req.notes,
    }))
}

#[derive(Deserialize)]
pub(super) struct EnrollRequest {
    detection_id: String,
    label: String,
    notes: Option<String>,
}

#[derive(Serialize)]
pub(super) struct IndividualSummary {
    id: String,
    scientific_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    common_name: Option<String>,
    label: String,
    enrolled_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

// ── Candidate clusters ─────────────────────────────────────────

pub(super) async fn list_candidate_clusters(
    State(state): State<ApiState>,
) -> Result<Json<Vec<CandidateClusterSummary>>, ApiError> {
    let min_members = state.core.initial_config.min_cluster_size;
    let min_days = state.core.initial_config.min_distinct_days;

    let rows = state
        .core
        .db
        .ready_clusters(min_members, min_days)
        .await?;

    let mut clusters: Vec<CandidateClusterSummary> = rows
        .into_iter()
        .map(|r| CandidateClusterSummary {
            id: r.id,
            scientific_name: r.scientific_name,
            common_name: None,
            member_count: r.member_count,
            distinct_days: r.distinct_days,
            first_seen_at: millis_to_rfc3339(r.first_seen_at).unwrap_or_default(),
            last_seen_at: millis_to_rfc3339(r.last_seen_at).unwrap_or_default(),
        })
        .collect();

    // Resolve common names from the labels table.
    for c in &mut clusters {
        if let Ok(name) = state.core.db.common_name_for(&c.scientific_name).await {
            c.common_name = name;
        }
    }

    Ok(Json(clusters))
}

pub(super) async fn enroll_cluster(
    State(state): State<ApiState>,
    Path(cluster_id): Path<i64>,
    Json(req): Json<ClusterEnrollRequest>,
) -> Result<Json<IndividualSummary>, ApiError> {
    let cluster = state
        .core
        .db
        .get_cluster(cluster_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("cluster not found"))?;

    if cluster.status != "pending" {
        return Err(ApiError::conflict(format!(
            "cluster is already {}",
            cluster.status
        )));
    }

    // Create individual from cluster centroid.
    let individual_id = uuid::Uuid::now_v7();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let dim = cluster.centroid_dim;

    state
        .core
        .db
        .insert_individual(&sitta_store::models::NewIndividual {
            id: &individual_id,
            scientific_name: &cluster.scientific_name,
            label: &req.label,
            reference_embedding: Some(&cluster.centroid),
            reference_embedding_dim: Some(dim),
            enrolled_at: now_ms,
            notes: req.notes.as_deref(),
        })
        .await
        .map_err(ApiError::internal)?;

    // Mark cluster as enrolled.
    state
        .core
        .db
        .enroll_cluster(cluster_id, &individual_id)
        .await
        .map_err(ApiError::internal)?;

    // Link cluster member detections to the new individual.
    let detection_ids = state
        .core
        .db
        .cluster_detection_ids(cluster_id)
        .await
        .map_err(ApiError::internal)?;
    for det_bytes in &detection_ids {
        if let Ok(det_uuid) = uuid_from_blob(det_bytes.clone()) {
            let match_id = uuid::Uuid::now_v7();
            // Use similarity 1.0 as a sentinel — these are founding members, not runtime matches.
            let _ = state
                .core
                .db
                .insert_individual_match(&match_id, &individual_id, &det_uuid, 1.0, now_ms)
                .await;
        }
    }

    // Reload matcher so future detections match against this individual.
    if let Some(matcher) = &state.inference.matcher
        && let Err(e) = matcher.reload().await
    {
        tracing::warn!(error = %e, "Failed to reload matcher after cluster enrollment");
    }

    tracing::info!(
        cluster_id,
        individual = %individual_id,
        label = %req.label,
        species = %cluster.scientific_name,
        members = detection_ids.len(),
        "Cluster enrolled as individual"
    );

    Ok(Json(IndividualSummary {
        id: individual_id.to_string(),
        scientific_name: cluster.scientific_name,
        common_name: None,
        label: req.label,
        enrolled_at: millis_to_rfc3339(now_ms).unwrap_or_default(),
        notes: req.notes,
    }))
}

pub(super) async fn dismiss_cluster(
    State(state): State<ApiState>,
    Path(cluster_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let cluster = state
        .core
        .db
        .get_cluster(cluster_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or(ApiError::not_found("cluster not found"))?;

    if cluster.status != "pending" {
        return Err(ApiError::conflict(format!(
            "cluster is already {}",
            cluster.status
        )));
    }

    state
        .core
        .db
        .dismiss_cluster(cluster_id)
        .await
        .map_err(ApiError::internal)?;

    tracing::info!(cluster_id, species = %cluster.scientific_name, "Cluster dismissed");

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub(super) struct CandidateClusterSummary {
    id: i64,
    scientific_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    common_name: Option<String>,
    member_count: i64,
    distinct_days: i64,
    first_seen_at: String,
    last_seen_at: String,
}

#[derive(Deserialize)]
pub(super) struct ClusterEnrollRequest {
    label: String,
    notes: Option<String>,
}

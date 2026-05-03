//! Detection insert, read, and correlation queries.

use std::collections::HashMap;

use uuid::Uuid;

use crate::models::{
    uuid_bytes, DetectionRow, NewDetection, NewPrediction, PredictionRow, RarityRow,
};

use super::Database;

/// Reconstruct a `RarityRow` from the columns produced by a
/// `LEFT JOIN detection_rarity`. Returns `None` when the detection had no
/// rarity row (the score column will be NULL).
#[allow(clippy::too_many_arguments)]
fn build_rarity_row(
    detection_id: &[u8],
    score: Option<f64>,
    first_ever: Option<bool>,
    first_season: Option<bool>,
    first_week: Option<bool>,
    first_day: Option<bool>,
    days_since_last: Option<i64>,
    local_count: Option<i64>,
    range_score: Option<f64>,
    temporal_score: Option<f64>,
) -> Option<RarityRow> {
    Some(RarityRow {
        detection_id: detection_id.to_vec(),
        score: score?,
        first_ever: first_ever.unwrap_or(false),
        first_season: first_season.unwrap_or(false),
        first_week: first_week.unwrap_or(false),
        first_day: first_day.unwrap_or(false),
        days_since_last,
        local_count: local_count.unwrap_or(0),
        range_score,
        temporal_score: temporal_score.unwrap_or(0.0),
    })
}

impl Database {
    /// Insert a single detection.
    pub async fn insert_detection(
        &self,
        det: &NewDetection<'_>,
    ) -> Result<(), crate::StoreError> {
        let id = uuid_bytes(det.id);
        let station_id = uuid_bytes(det.station_id);
        let source_id = det.source_id.map(uuid_bytes);
        sqlx::query!(
            "INSERT INTO detections (id, station_id, source_id, model_id, label_id, detected_at, confidence, snippet_path, snippet_duration_ms, snippet_sample_rate, metadata, range_status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            id,
            station_id,
            source_id,
            det.model_id,
            det.label_id,
            det.detected_at,
            det.confidence,
            det.snippet_path,
            det.snippet_duration_ms,
            det.snippet_sample_rate,
            det.metadata,
            det.range_status,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert secondary predictions for a detection.
    pub async fn insert_predictions(
        &self,
        detection_id: &Uuid,
        predictions: &[NewPrediction],
    ) -> Result<(), crate::StoreError> {
        if predictions.is_empty() {
            return Ok(());
        }
        let det_id = uuid_bytes(detection_id);
        let mut tx = self.pool.begin().await?;
        for pred in predictions {
            sqlx::query!(
                "INSERT INTO detection_predictions (detection_id, rank, label_id, confidence)
                 VALUES ($1, $2, $3, $4)",
                det_id,
                pred.rank,
                pred.label_id,
                pred.confidence,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Insert an embedding for a detection. The f32 slice is stored as
    /// little-endian bytes in the BLOB column.
    pub async fn insert_embedding(
        &self,
        detection_id: &Uuid,
        embedding: &[f32],
    ) -> Result<(), crate::StoreError> {
        let det_id = uuid_bytes(detection_id);
        let bytes: &[u8] = bytemuck::cast_slice(embedding);
        let dim = embedding.len() as i64;
        sqlx::query!(
            "INSERT INTO embeddings (detection_id, embedding, embedding_dim)
             VALUES ($1, $2, $3)",
            det_id,
            bytes,
            dim,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Recent detections with joined label/model/source/rarity/individual info.
    /// Deduplicated: when multiple models detect the same species within a
    /// 5-second window, only the highest-confidence detection is returned.
    /// The `model_name` field contains all confirming models (comma-separated).
    ///
    /// When `rare_only` is true, the SQL filter retains only rows that meet
    /// the rare predicate (any first_* flag, or composite score ≥ 0.6) so
    /// the `has_more` calculation reflects post-filter counts.
    #[allow(clippy::too_many_arguments)]
    pub async fn recent_detections(
        &self,
        since: i64,
        until: i64,
        limit: i64,
        offset: i64,
        species: Option<&str>,
        min_confidence: Option<f64>,
        rare_only: bool,
    ) -> Result<Vec<DetectionRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        // Fetch more rows than requested to have headroom for deduplication.
        // Rarity-filtered queries can lose proportionally more rows to dedup,
        // so widen the headroom in that case.
        let fetch_limit = if rare_only { limit * 6 } else { limit * 3 };
        let rows = sqlx::query!(
            r#"SELECT d.id, d.detected_at, d.confidence AS "confidence!: f64",
                      d.snippet_path, d.metadata,
                      l.scientific_name, l.common_name AS "common_name!",
                      l.taxon_code,
                      m.name AS "model_name!", m.version AS "model_version!",
                      s.name AS "source_name?",
                      (EXISTS (SELECT 1 FROM embeddings e WHERE e.detection_id = d.id)) AS "has_embedding!: bool",
                      d.range_status,
                      im.individual_id AS "individual_id?: Vec<u8>",
                      ind.label AS "individual_label?",
                      im.similarity AS "individual_similarity?: f64",
                      r.score AS "rarity_score?: f64",
                      r.first_ever AS "first_ever?: bool",
                      r.first_season AS "first_season?: bool",
                      r.first_week AS "first_week?: bool",
                      r.first_day AS "first_day?: bool",
                      r.days_since_last AS "days_since_last?: i64",
                      r.local_count AS "local_count?: i64",
                      r.range_score AS "range_score?: f64",
                      r.temporal_score AS "temporal_score?: f64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               JOIN models m ON m.id = d.model_id
               LEFT JOIN audio_sources s ON s.id = d.source_id
               LEFT JOIN individual_matches im ON im.detection_id = d.id
                   AND im.id = (
                       SELECT m2.id FROM individual_matches m2
                       WHERE m2.detection_id = d.id
                       ORDER BY m2.similarity DESC LIMIT 1
                   )
               LEFT JOIN individuals ind ON ind.id = im.individual_id
               LEFT JOIN detection_rarity r ON r.detection_id = d.id
               WHERE d.detected_at >= $1 AND d.detected_at <= $2
                 AND d.confidence >= $6
                 AND ($5 IS NULL OR l.scientific_name = $5)
                 AND ($7 = 0 OR (r.first_ever OR r.first_season OR r.first_week OR r.first_day OR r.score >= 0.6))
               ORDER BY d.detected_at DESC
               LIMIT $3 OFFSET $4"#,
            since,
            until,
            fetch_limit,
            offset,
            species,
            conf_floor,
            rare_only,
        )
        .fetch_all(&self.pool)
        .await?;

        // Deduplicate: group by (species, 5-second time bucket).
        // Keep the highest-confidence detection per group, merge model names.
        let mut seen: HashMap<(String, i64), usize> = HashMap::new();
        let mut result: Vec<DetectionRow> = Vec::new();

        for r in rows {
            let sci = r.scientific_name.clone().unwrap_or_default();
            let bucket = r.detected_at / 5000;
            let key = (sci, bucket);
            let rarity = build_rarity_row(
                &r.id,
                r.rarity_score,
                r.first_ever,
                r.first_season,
                r.first_week,
                r.first_day,
                r.days_since_last,
                r.local_count,
                r.range_score,
                r.temporal_score,
            );

            if let Some(&idx) = seen.get(&key) {
                let existing = &mut result[idx];
                if !existing.model_name.contains(&r.model_name) {
                    existing.model_name = format!("{}, {}", existing.model_name, r.model_name);
                }
                if r.confidence > existing.confidence {
                    existing.confidence = r.confidence;
                    existing.id = r.id;
                    existing.detected_at = r.detected_at;
                    existing.snippet_path = r.snippet_path;
                    existing.individual_id = r.individual_id;
                    existing.individual_label = r.individual_label;
                    existing.individual_similarity = r.individual_similarity;
                    existing.rarity = rarity;
                }
                if r.has_embedding {
                    existing.has_embedding = true;
                }
            } else {
                seen.insert(key, result.len());
                result.push(DetectionRow {
                    id: r.id,
                    detected_at: r.detected_at,
                    confidence: r.confidence,
                    snippet_path: r.snippet_path,
                    metadata: r.metadata,
                    scientific_name: r.scientific_name,
                    common_name: r.common_name,
                    taxon_code: r.taxon_code,
                    model_name: r.model_name,
                    model_version: r.model_version,
                    source_name: r.source_name,
                    has_embedding: r.has_embedding,
                    range_status: r.range_status.clone(),
                    individual_id: r.individual_id,
                    individual_label: r.individual_label,
                    individual_similarity: r.individual_similarity,
                    rarity,
                });
            }
        }

        result.truncate(limit as usize);
        Ok(result)
    }

    /// Single detection by ID with joined info.
    pub async fn get_detection(
        &self,
        id: &[u8],
    ) -> Result<Option<DetectionRow>, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT d.id, d.detected_at, d.confidence AS "confidence!: f64",
                      d.snippet_path, d.metadata,
                      l.scientific_name, l.common_name AS "common_name!",
                      l.taxon_code,
                      m.name AS "model_name!", m.version AS "model_version!",
                      s.name AS "source_name?",
                      (EXISTS (SELECT 1 FROM embeddings e WHERE e.detection_id = d.id)) AS "has_embedding!: bool",
                      d.range_status,
                      im.individual_id AS "individual_id?: Vec<u8>",
                      ind.label AS "individual_label?",
                      im.similarity AS "individual_similarity?: f64",
                      r.score AS "rarity_score?: f64",
                      r.first_ever AS "first_ever?: bool",
                      r.first_season AS "first_season?: bool",
                      r.first_week AS "first_week?: bool",
                      r.first_day AS "first_day?: bool",
                      r.days_since_last AS "days_since_last?: i64",
                      r.local_count AS "local_count?: i64",
                      r.range_score AS "range_score?: f64",
                      r.temporal_score AS "temporal_score?: f64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               JOIN models m ON m.id = d.model_id
               LEFT JOIN audio_sources s ON s.id = d.source_id
               LEFT JOIN individual_matches im ON im.detection_id = d.id
                   AND im.id = (
                       SELECT m2.id FROM individual_matches m2
                       WHERE m2.detection_id = d.id
                       ORDER BY m2.similarity DESC LIMIT 1
                   )
               LEFT JOIN individuals ind ON ind.id = im.individual_id
               LEFT JOIN detection_rarity r ON r.detection_id = d.id
               WHERE d.id = $1"#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let rarity = build_rarity_row(
                &r.id,
                r.rarity_score,
                r.first_ever,
                r.first_season,
                r.first_week,
                r.first_day,
                r.days_since_last,
                r.local_count,
                r.range_score,
                r.temporal_score,
            );
            DetectionRow {
                id: r.id,
                detected_at: r.detected_at,
                confidence: r.confidence,
                snippet_path: r.snippet_path,
                metadata: r.metadata,
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                taxon_code: r.taxon_code,
                model_name: r.model_name,
                model_version: r.model_version,
                source_name: r.source_name,
                has_embedding: r.has_embedding,
                range_status: r.range_status,
                individual_id: r.individual_id,
                individual_label: r.individual_label,
                individual_similarity: r.individual_similarity,
                rarity,
            }
        }))
    }

    /// Secondary predictions for a detection.
    pub async fn get_predictions(
        &self,
        detection_id: &[u8],
    ) -> Result<Vec<PredictionRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT p.rank, p.confidence AS "confidence!: f64",
                      l.scientific_name, l.common_name AS "common_name!"
               FROM detection_predictions p
               JOIN labels l ON l.id = p.label_id
               WHERE p.detection_id = $1
               ORDER BY p.rank"#,
            detection_id,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| PredictionRow {
                rank: r.rank,
                confidence: r.confidence,
                scientific_name: r.scientific_name,
                common_name: r.common_name,
            })
            .collect())
    }

    /// Find detections from other models within ±5 seconds of a given timestamp.
    /// Used to show what other models detected for the same audio moment.
    pub async fn correlated_detections(
        &self,
        detection_id: &[u8],
        timestamp_ms: i64,
        limit: i64,
    ) -> Result<Vec<DetectionRow>, crate::StoreError> {
        let window_start = timestamp_ms - 5000;
        let window_end = timestamp_ms + 5000;
        let rows = sqlx::query!(
            r#"SELECT d.id, d.detected_at, d.confidence AS "confidence!: f64",
                      d.snippet_path, d.metadata,
                      l.scientific_name, l.common_name AS "common_name!",
                      l.taxon_code,
                      m.name AS "model_name!", m.version AS "model_version!",
                      s.name AS "source_name?",
                      (EXISTS (SELECT 1 FROM embeddings e WHERE e.detection_id = d.id)) AS "has_embedding!: bool",
                      d.range_status
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               JOIN models m ON m.id = d.model_id
               LEFT JOIN audio_sources s ON s.id = d.source_id
               WHERE d.detected_at >= $1 AND d.detected_at <= $2
                 AND d.id != $3
               ORDER BY ABS(d.detected_at - $4) ASC
               LIMIT $5"#,
            window_start,
            window_end,
            detection_id,
            timestamp_ms,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| DetectionRow {
                id: r.id,
                detected_at: r.detected_at,
                confidence: r.confidence,
                snippet_path: r.snippet_path,
                metadata: r.metadata,
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                taxon_code: r.taxon_code,
                model_name: r.model_name,
                model_version: r.model_version,
                source_name: r.source_name,
                has_embedding: r.has_embedding,
                range_status: r.range_status,
                // Correlated detections render as a side panel showing
                // species + confidence; individual + rarity data isn't
                // surfaced here (the user clicks through to the detail
                // page for that).
                individual_id: None,
                individual_label: None,
                individual_similarity: None,
                rarity: None,
            })
            .collect())
    }

    /// Delete a detection and all associated data.
    ///
    /// Returns the snippet_path (if any) so the caller can clean up the audio file.
    /// Manually deletes from `detection_rarity` (no cascade FK), then deletes the
    /// detection row which cascades to predictions, embeddings, reviews, matches,
    /// and candidate_embeddings.
    pub async fn delete_detection(
        &self,
        id: &[u8],
    ) -> Result<Option<String>, crate::StoreError> {
        let mut tx = self.pool.begin().await?;

        // Grab snippet_path before deleting.
        let snippet_path = sqlx::query_scalar!(
            "SELECT snippet_path FROM detections WHERE id = $1",
            id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .flatten();

        // detection_rarity has no ON DELETE CASCADE.
        sqlx::query!("DELETE FROM detection_rarity WHERE detection_id = $1", id)
            .execute(&mut *tx)
            .await?;

        // This cascades to: detection_predictions, embeddings,
        // individual_matches, detection_reviews, candidate_embeddings.
        sqlx::query!("DELETE FROM detections WHERE id = $1", id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(snippet_path)
    }

    /// Total detection count (for status endpoint).
    pub async fn detection_count(&self) -> Result<i64, crate::StoreError> {
        let row = sqlx::query!(r#"SELECT COUNT(*) AS "count!" FROM detections"#)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.count)
    }
}

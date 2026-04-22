//! Audio snippet path management for the retention worker.

use crate::models::DetectionRow;

use super::Database;

impl Database {
    /// Update the snippet path for a detection after async clip saving.
    pub async fn update_snippet_path(
        &self,
        detection_id: &[u8],
        path: &str,
        duration_ms: i64,
        sample_rate: i64,
    ) -> Result<(), crate::StoreError> {
        sqlx::query!(
            "UPDATE detections SET snippet_path = $1, snippet_duration_ms = $2, snippet_sample_rate = $3 WHERE id = $4",
            path,
            duration_ms,
            sample_rate,
            detection_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clear the snippet path for a detection (after retention cleanup).
    pub async fn clear_snippet_path(
        &self,
        detection_id: &[u8],
    ) -> Result<(), crate::StoreError> {
        sqlx::query!(
            "UPDATE detections SET snippet_path = NULL, snippet_duration_ms = NULL, snippet_sample_rate = NULL WHERE id = $1",
            detection_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch detections with non-NULL snippet_path ordered by detected_at ASC.
    /// Used by the retention worker.
    pub async fn detections_with_snippets(
        &self,
        limit: i64,
    ) -> Result<Vec<DetectionRow>, crate::StoreError> {
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
               WHERE d.snippet_path IS NOT NULL
               ORDER BY d.detected_at ASC
               LIMIT $1"#,
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
            })
            .collect())
    }
}

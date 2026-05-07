//! Rarity scoring: insert, read, and helper queries for computing rarity.

use crate::models::{uuid_bytes, NewRarity, NotableDetectionRow, RarityRow};

use super::Database;

impl Database {
    /// Insert a rarity score for a detection.
    pub async fn insert_rarity(&self, r: &NewRarity<'_>) -> Result<(), crate::StoreError> {
        let det_id = uuid_bytes(r.detection_id);
        sqlx::query!(
            "INSERT INTO detection_rarity
                (detection_id, score, first_ever, first_season, first_week, first_day,
                 days_since_last, local_count, range_score, temporal_score)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            det_id,
            r.score,
            r.first_ever,
            r.first_season,
            r.first_week,
            r.first_day,
            r.days_since_last,
            r.local_count,
            r.range_score,
            r.temporal_score,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch rarity for a single detection.
    pub async fn get_rarity(
        &self,
        detection_id: &[u8],
    ) -> Result<Option<RarityRow>, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT detection_id, score AS "score!: f64",
                      first_ever AS "first_ever!: bool",
                      first_season AS "first_season!: bool",
                      first_week AS "first_week!: bool",
                      first_day AS "first_day!: bool",
                      days_since_last,
                      local_count AS "local_count!: i64",
                      range_score,
                      temporal_score AS "temporal_score!: f64"
               FROM detection_rarity
               WHERE detection_id = $1"#,
            detection_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| RarityRow {
            detection_id: r.detection_id,
            score: r.score,
            first_ever: r.first_ever,
            first_season: r.first_season,
            first_week: r.first_week,
            first_day: r.first_day,
            days_since_last: r.days_since_last,
            local_count: r.local_count,
            range_score: r.range_score,
            temporal_score: r.temporal_score,
        }))
    }

    /// Recent notable detections (highest rarity scores) for a species.
    pub async fn notable_detections(
        &self,
        scientific_name: &str,
        limit: i64,
        min_confidence: f64,
    ) -> Result<Vec<NotableDetectionRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT r.detection_id, d.detected_at, d.confidence AS "confidence!: f64",
                      r.score AS "score!: f64",
                      r.first_ever AS "first_ever!: bool",
                      r.first_season AS "first_season!: bool",
                      r.first_week AS "first_week!: bool",
                      r.first_day AS "first_day!: bool"
               FROM detection_rarity r
               JOIN detections d ON d.id = r.detection_id
               JOIN labels l ON l.id = d.label_id
               WHERE l.scientific_name = $1
                 AND d.confidence >= $2
                 AND r.score > 0.3
               ORDER BY r.score DESC, d.detected_at DESC
               LIMIT $3"#,
            scientific_name,
            min_confidence,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| NotableDetectionRow {
                detection_id: r.detection_id,
                detected_at: r.detected_at,
                confidence: r.confidence,
                score: r.score,
                first_ever: r.first_ever,
                first_season: r.first_season,
                first_week: r.first_week,
                first_day: r.first_day,
            })
            .collect())
    }

    // ── Helper queries used to compute rarity at detection time ────

    /// Count prior detections + most recent timestamp for a species at this station.
    /// Returns (count, last_detected_at_ms) or (0, None) if never seen.
    ///
    /// Keyed by scientific name, not label_id, because labels are
    /// per-(model, label_index) — BirdNET's "Turdus migratorius" and
    /// Perch's are different label_ids. Filtering by label_id partitioned
    /// rarity history by model, so the first detection per model per day
    /// got `first_day = true` (and similar) even when the other model had
    /// found that species all morning.
    pub async fn species_local_history(
        &self,
        scientific_name: &str,
        station_id: &[u8],
        before_ms: i64,
        min_confidence: f64,
    ) -> Result<(i64, Option<i64>), crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT COUNT(*) AS "count!: i64",
                      MAX(d.detected_at) AS "last_at: i64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE l.scientific_name = $1
                 AND d.station_id = $2
                 AND d.detected_at < $3
                 AND d.confidence >= $4"#,
            scientific_name,
            station_id,
            before_ms,
            min_confidence,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok((row.count, row.last_at))
    }

    /// Fraction of a species' detections that occurred in a given UTC hour (0-23).
    /// Returns 0.0 if the species has no prior detections.
    /// Keyed by scientific name (see `species_local_history` for why).
    pub async fn species_hour_fraction(
        &self,
        scientific_name: &str,
        hour_utc: i64,
        before_ms: i64,
        min_confidence: f64,
    ) -> Result<f64, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT
                 COALESCE(SUM(CASE WHEN CAST((d.detected_at / 3600000) % 24 AS INTEGER) = $2
                                   THEN 1 ELSE 0 END), 0) AS "hour_count!: i64",
                 COUNT(*) AS "total!: i64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE l.scientific_name = $1
                 AND d.detected_at < $3
                 AND d.confidence >= $4"#,
            scientific_name,
            hour_utc,
            before_ms,
            min_confidence,
        )
        .fetch_one(&self.pool)
        .await?;

        if row.total == 0 {
            return Ok(0.0);
        }
        Ok(row.hour_count as f64 / row.total as f64)
    }
}

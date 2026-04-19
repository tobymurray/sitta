//! Species summary, hourly activity, and per-species analytics.

use crate::models::{
    HourlyActivityRow, SpeciesHourlyProfileRow, SpeciesMonthlyRow, SpeciesStatsRow,
    SpeciesSummaryRow,
};

use super::Database;

impl Database {
    /// Species summary aggregated over a date range.
    pub async fn species_summary(
        &self,
        since: i64,
        until: i64,
        min_confidence: Option<f64>,
    ) -> Result<Vec<SpeciesSummaryRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        let rows = sqlx::query!(
            r#"SELECT l.scientific_name, l.common_name AS "common_name!",
                      l.taxon_code,
                      COUNT(*) AS "detection_count!: i64",
                      MAX(d.detected_at) AS "last_detected_at!: i64",
                      AVG(d.confidence) AS "avg_confidence!: f64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE d.detected_at >= $1 AND d.detected_at <= $2
                 AND d.confidence >= $3
                 AND l.label_type = 'species'
               GROUP BY l.scientific_name, l.common_name, l.taxon_code
               ORDER BY COUNT(*) DESC"#,
            since,
            until,
            conf_floor,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SpeciesSummaryRow {
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                taxon_code: r.taxon_code,
                detection_count: r.detection_count,
                last_detected_at: r.last_detected_at,
                avg_confidence: r.avg_confidence,
            })
            .collect())
    }

    /// Hourly detection counts per species over a date range.
    pub async fn hourly_activity(
        &self,
        since: i64,
        until: i64,
        min_confidence: Option<f64>,
    ) -> Result<Vec<HourlyActivityRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        let rows = sqlx::query!(
            r#"SELECT l.common_name AS "common_name!",
                      l.scientific_name,
                      l.taxon_code,
                      CAST((d.detected_at - $1) / 3600000 AS INTEGER) AS "hour_bucket!: i64",
                      COUNT(*) AS "count!: i64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE d.detected_at >= $1 AND d.detected_at < $2
                 AND d.confidence >= $3
                 AND l.label_type = 'species'
               GROUP BY l.scientific_name, l.common_name, l.taxon_code,
                        CAST((d.detected_at - $1) / 3600000 AS INTEGER)
               ORDER BY COUNT(*) DESC"#,
            since,
            until,
            conf_floor,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| HourlyActivityRow {
                common_name: r.common_name,
                scientific_name: r.scientific_name,
                taxon_code: r.taxon_code,
                hour_bucket: r.hour_bucket,
                count: r.count,
            })
            .collect())
    }

    /// Hourly detection profile for a species across all time (24 UTC hours).
    pub async fn species_hourly_profile(
        &self,
        scientific_name: &str,
        min_confidence: Option<f64>,
    ) -> Result<Vec<SpeciesHourlyProfileRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        let rows = sqlx::query!(
            r#"SELECT CAST((d.detected_at / 3600000) % 24 AS INTEGER) AS "hour_utc!: i64",
                      COUNT(*) AS "count!: i64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE l.scientific_name = $1
                 AND d.confidence >= $2
                 AND l.label_type = 'species'
               GROUP BY CAST((d.detected_at / 3600000) % 24 AS INTEGER)
               ORDER BY 1"#,
            scientific_name,
            conf_floor,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SpeciesHourlyProfileRow {
                hour_utc: r.hour_utc,
                count: r.count,
            })
            .collect())
    }

    /// Monthly detection distribution for a species across all time (12 calendar months).
    pub async fn species_monthly_distribution(
        &self,
        scientific_name: &str,
        min_confidence: Option<f64>,
    ) -> Result<Vec<SpeciesMonthlyRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        // SQLite: extract month from Unix-ms timestamp.
        // strftime('%m', ..., 'unixepoch') gives zero-padded month string.
        let rows = sqlx::query!(
            r#"SELECT CAST(strftime('%m', d.detected_at / 1000, 'unixepoch') AS INTEGER) AS "month!: i64",
                      COUNT(*) AS "count!: i64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE l.scientific_name = $1
                 AND d.confidence >= $2
                 AND l.label_type = 'species'
               GROUP BY CAST(strftime('%m', d.detected_at / 1000, 'unixepoch') AS INTEGER)
               ORDER BY 1"#,
            scientific_name,
            conf_floor,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SpeciesMonthlyRow {
                month: r.month,
                count: r.count,
            })
            .collect())
    }

    /// Aggregate stats for a species across all detections.
    pub async fn species_stats(
        &self,
        scientific_name: &str,
        min_confidence: Option<f64>,
    ) -> Result<Option<SpeciesStatsRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        let row = sqlx::query!(
            r#"SELECT l.common_name AS "common_name!",
                      COUNT(*) AS "total!: i64",
                      MIN(d.detected_at) AS "first_detected_at!: i64",
                      MAX(d.detected_at) AS "last_detected_at!: i64",
                      AVG(d.confidence) AS "avg_confidence!: f64",
                      COUNT(DISTINCT CAST(d.detected_at / 86400000 AS INTEGER)) AS "distinct_days!: i64"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               WHERE l.scientific_name = $1
                 AND d.confidence >= $2
                 AND l.label_type = 'species'"#,
            scientific_name,
            conf_floor,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|r| {
            if r.total == 0 {
                return None;
            }
            Some(SpeciesStatsRow {
                common_name: r.common_name,
                total: r.total,
                first_detected_at: r.first_detected_at,
                last_detected_at: r.last_detected_at,
                avg_confidence: r.avg_confidence,
                distinct_days: r.distinct_days,
            })
        }))
    }
}

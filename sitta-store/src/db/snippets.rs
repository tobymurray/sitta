//! Audio snippet path management for the retention worker.

use super::Database;

/// One day's worth of detection counts split by whether a clip was saved.
#[derive(Debug, Clone)]
pub struct DailyAudioHealth {
    pub day: String,
    pub total: i64,
    pub with_clip: i64,
}

/// Aggregate counts of detections with and without a saved clip.
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioHealthTotals {
    pub total: i64,
    pub with_clip: i64,
}

/// Earliest and latest `detected_at` (Unix ms) among detections that have
/// `snippet_path IS NULL`. Lets the diagnostics page answer
/// "is missing-audio still happening now?" without scrolling a chart.
///
/// `recent_count` is the count of clipless detections younger than the
/// caller-supplied cutoff (typically 15 minutes ago). The UI uses this
/// directly: > 0 means *new* clipless rows are still being created, so
/// the gap is active. == 0 means the writer is keeping up; the
/// `first_ms` / `last_ms` window describes a historical gap.
#[derive(Debug, Clone, Copy, Default)]
pub struct CliplessRange {
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub count: i64,
    pub recent_count: i64,
}

/// Per-tier clip counts. Each clip is counted in exactly one tier; tiers
/// are tested in protection order (most protective first), matching the
/// retention worker's `tier()` function.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClipTierBreakdown {
    pub first_ever: i64,
    pub first_season: i64,
    pub first_week: i64,
    pub first_day: i64,
    pub high_score: i64,
    pub common: i64,
    pub reviewed_correct: i64,
}

/// One row of the "top species by clip count" diagnostic.
#[derive(Debug, Clone)]
pub struct SpeciesClipCount {
    pub scientific_name: String,
    pub common_name: String,
    pub clip_count: i64,
}

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

    /// Fetch retention candidates: detections with a saved snippet, joined
    /// to their rarity row and a reviewed-correct flag. The retention worker
    /// loops over the result directly without further per-row queries.
    pub async fn retention_candidates(
        &self,
        limit: i64,
    ) -> Result<Vec<crate::models::RetentionCandidateRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT d.id, d.detected_at,
                      d.confidence AS "confidence!: f64",
                      d.snippet_path AS "snippet_path!",
                      l.scientific_name AS "scientific_name?",
                      r.score AS "rarity_score?: f64",
                      r.first_ever AS "first_ever?: bool",
                      r.first_season AS "first_season?: bool",
                      r.first_week AS "first_week?: bool",
                      r.first_day AS "first_day?: bool",
                      r.days_since_last AS "days_since_last?: i64",
                      r.local_count AS "local_count?: i64",
                      r.range_score AS "range_score?: f64",
                      r.temporal_score AS "temporal_score?: f64",
                      (EXISTS (SELECT 1 FROM detection_reviews v
                               WHERE v.detection_id = d.id
                                 AND v.status = 'correct')) AS "reviewed_correct!: bool"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               LEFT JOIN detection_rarity r ON r.detection_id = d.id
               WHERE d.snippet_path IS NOT NULL
               ORDER BY d.detected_at ASC
               LIMIT $1"#,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let rarity = r.rarity_score.map(|score| crate::models::RarityRow {
                    detection_id: r.id.clone(),
                    score,
                    first_ever: r.first_ever.unwrap_or(false),
                    first_season: r.first_season.unwrap_or(false),
                    first_week: r.first_week.unwrap_or(false),
                    first_day: r.first_day.unwrap_or(false),
                    days_since_last: r.days_since_last,
                    local_count: r.local_count.unwrap_or(0),
                    range_score: r.range_score,
                    temporal_score: r.temporal_score.unwrap_or(0.0),
                });
                crate::models::RetentionCandidateRow {
                    id: r.id,
                    detected_at: r.detected_at,
                    confidence: r.confidence,
                    snippet_path: r.snippet_path,
                    scientific_name: r.scientific_name.unwrap_or_default(),
                    rarity,
                    reviewed_correct: r.reviewed_correct,
                }
            })
            .collect())
    }

    /// Daily breakdown of detections with vs. without a saved snippet,
    /// for detections at or after `since_ms`. Most recent day first.
    pub async fn daily_audio_health(
        &self,
        since_ms: i64,
    ) -> Result<Vec<DailyAudioHealth>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT
                strftime('%Y-%m-%d', detected_at / 1000, 'unixepoch') AS "day!: String",
                COUNT(*) AS "total!: i64",
                SUM(CASE WHEN snippet_path IS NOT NULL THEN 1 ELSE 0 END) AS "with_clip!: i64"
              FROM detections
              WHERE detected_at >= $1
              GROUP BY strftime('%Y-%m-%d', detected_at / 1000, 'unixepoch')
              ORDER BY 1 DESC"#,
            since_ms,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| DailyAudioHealth {
                day: r.day,
                total: r.total,
                with_clip: r.with_clip,
            })
            .collect())
    }

    /// Bucket every saved clip by its rarity tier. Each clip falls into
    /// exactly one bucket, tested in protection order (highest first), and
    /// matching the retention worker's `tier()` function. `reviewed_correct`
    /// is reported separately so the diagnostics page can call it out as
    /// "always preserved" regardless of tier.
    pub async fn clip_tier_breakdown(&self) -> Result<ClipTierBreakdown, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT
                SUM(CASE WHEN reviewed_correct THEN 1 ELSE 0 END) AS "reviewed_correct!: i64",
                SUM(CASE WHEN NOT reviewed_correct AND first_ever THEN 1 ELSE 0 END) AS "first_ever!: i64",
                SUM(CASE WHEN NOT reviewed_correct AND NOT first_ever AND first_season THEN 1 ELSE 0 END) AS "first_season!: i64",
                SUM(CASE WHEN NOT reviewed_correct AND NOT first_ever AND NOT first_season AND first_week THEN 1 ELSE 0 END) AS "first_week!: i64",
                SUM(CASE WHEN NOT reviewed_correct AND NOT first_ever AND NOT first_season AND NOT first_week AND first_day THEN 1 ELSE 0 END) AS "first_day!: i64",
                SUM(CASE WHEN NOT reviewed_correct AND NOT first_ever AND NOT first_season AND NOT first_week AND NOT first_day AND high_score THEN 1 ELSE 0 END) AS "high_score!: i64",
                SUM(CASE WHEN NOT reviewed_correct AND NOT first_ever AND NOT first_season AND NOT first_week AND NOT first_day AND NOT high_score THEN 1 ELSE 0 END) AS "common!: i64"
            FROM (
                SELECT
                    COALESCE(r.first_ever, 0) AS first_ever,
                    COALESCE(r.first_season, 0) AS first_season,
                    COALESCE(r.first_week, 0) AS first_week,
                    COALESCE(r.first_day, 0) AS first_day,
                    CASE WHEN COALESCE(r.score, 0) >= 0.6 THEN 1 ELSE 0 END AS high_score,
                    EXISTS (SELECT 1 FROM detection_reviews v WHERE v.detection_id = d.id AND v.status = 'correct') AS reviewed_correct
                FROM detections d
                LEFT JOIN detection_rarity r ON r.detection_id = d.id
                WHERE d.snippet_path IS NOT NULL
            )"#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(ClipTierBreakdown {
            first_ever: row.first_ever,
            first_season: row.first_season,
            first_week: row.first_week,
            first_day: row.first_day,
            high_score: row.high_score,
            common: row.common,
            reviewed_correct: row.reviewed_correct,
        })
    }

    /// Top species by saved-clip count. Used by the diagnostics page to
    /// surface which species are dominating the clip pool.
    pub async fn top_species_by_clip_count(
        &self,
        limit: i64,
    ) -> Result<Vec<SpeciesClipCount>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT
                l.scientific_name AS "scientific_name!: String",
                MIN(l.common_name) AS "common_name!: String",
                COUNT(*) AS "clip_count!: i64"
              FROM detections d
              JOIN labels l ON l.id = d.label_id
              WHERE d.snippet_path IS NOT NULL
                AND l.scientific_name IS NOT NULL
              GROUP BY l.scientific_name
              ORDER BY COUNT(*) DESC
              LIMIT $1"#,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SpeciesClipCount {
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                clip_count: r.clip_count,
            })
            .collect())
    }

    /// All-time totals of detections vs. detections with a saved snippet.
    pub async fn audio_health_totals(&self) -> Result<AudioHealthTotals, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT
                COUNT(*) AS "total!: i64",
                SUM(CASE WHEN snippet_path IS NOT NULL THEN 1 ELSE 0 END) AS "with_clip!: i64"
              FROM detections"#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(AudioHealthTotals {
            total: row.total,
            with_clip: row.with_clip,
        })
    }

    /// First / last `detected_at` (Unix ms) for detections that have no
    /// saved clip, plus the total count and a "recent" subset count
    /// (rows with detected_at >= `recent_cutoff_ms`). The recent count
    /// is the most useful diagnostic signal: > 0 means new clipless
    /// rows are still appearing.
    pub async fn clipless_range(
        &self,
        recent_cutoff_ms: i64,
    ) -> Result<CliplessRange, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT
                COUNT(*) AS "count!: i64",
                MIN(detected_at) AS "first_ms?: i64",
                MAX(detected_at) AS "last_ms?: i64",
                SUM(CASE WHEN detected_at >= $1 THEN 1 ELSE 0 END) AS "recent_count!: i64"
              FROM detections
              WHERE snippet_path IS NULL"#,
            recent_cutoff_ms,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(CliplessRange {
            first_ms: row.first_ms,
            last_ms: row.last_ms,
            count: row.count,
            recent_count: row.recent_count,
        })
    }
}

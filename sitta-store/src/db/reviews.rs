//! Detection review CRUD (correct / false_positive).

use crate::models::ReviewRow;

use super::Database;

impl Database {
    /// Insert or replace a review for a detection.
    pub async fn upsert_review(
        &self,
        detection_id: &[u8],
        status: &str,
        reviewed_at: i64,
        comment: Option<&str>,
    ) -> Result<(), crate::StoreError> {
        sqlx::query!(
            "INSERT INTO detection_reviews (detection_id, status, reviewed_at, comment)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(detection_id) DO UPDATE SET status = $2, reviewed_at = $3, comment = $4",
            detection_id,
            status,
            reviewed_at,
            comment,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch the review for a detection.
    pub async fn get_review(
        &self,
        detection_id: &[u8],
    ) -> Result<Option<ReviewRow>, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT detection_id, status, reviewed_at, comment
               FROM detection_reviews WHERE detection_id = $1"#,
            detection_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ReviewRow {
            detection_id: r.detection_id,
            status: r.status,
            reviewed_at: r.reviewed_at,
            comment: r.comment,
        }))
    }

    /// Delete a review (un-review a detection).
    pub async fn delete_review(
        &self,
        detection_id: &[u8],
    ) -> Result<bool, crate::StoreError> {
        let result = sqlx::query!(
            "DELETE FROM detection_reviews WHERE detection_id = $1",
            detection_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Check whether a detection is reviewed as "correct" (used by retention).
    pub async fn is_review_correct(
        &self,
        detection_id: &[u8],
    ) -> Result<bool, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT COUNT(*) AS "count!" FROM detection_reviews
               WHERE detection_id = $1 AND status = 'correct'"#,
            detection_id,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.count > 0)
    }
}

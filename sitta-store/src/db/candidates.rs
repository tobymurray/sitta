//! Candidate embedding pool for unmatched detections.

use uuid::Uuid;

use crate::models::{uuid_bytes, CandidateRow};

use super::Database;

impl Database {
    /// Insert an unmatched embedding into the candidate pool.
    pub async fn insert_candidate(
        &self,
        detection_id: &Uuid,
        scientific_name: &str,
        embedding: &[u8],
        created_at: i64,
    ) -> Result<(), crate::StoreError> {
        let det_bytes = uuid_bytes(detection_id);
        sqlx::query!(
            "INSERT OR IGNORE INTO candidate_embeddings (detection_id, scientific_name, embedding, created_at)
             VALUES ($1, $2, $3, $4)",
            det_bytes,
            scientific_name,
            embedding,
            created_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load all unclustered candidates for a species.
    pub async fn unclustered_candidates(
        &self,
        scientific_name: &str,
    ) -> Result<Vec<CandidateRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT detection_id, scientific_name, embedding,
                      cluster_id AS "cluster_id?", created_at
               FROM candidate_embeddings
               WHERE scientific_name = $1 AND cluster_id IS NULL
               ORDER BY created_at ASC"#,
            scientific_name,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| CandidateRow {
                detection_id: r.detection_id,
                scientific_name: r.scientific_name,
                embedding: r.embedding,
                cluster_id: r.cluster_id,
                created_at: r.created_at,
            })
            .collect())
    }

    /// List distinct species that have unclustered candidates.
    pub async fn species_with_unclustered(&self) -> Result<Vec<String>, crate::StoreError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT scientific_name FROM candidate_embeddings WHERE cluster_id IS NULL"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.scientific_name).collect())
    }

    /// Prune old unclustered candidates older than `before_ms` (Unix milliseconds).
    pub async fn prune_old_candidates(&self, before_ms: i64) -> Result<u64, crate::StoreError> {
        let result = sqlx::query!(
            "DELETE FROM candidate_embeddings WHERE cluster_id IS NULL AND created_at < $1",
            before_ms,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

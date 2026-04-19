//! Candidate cluster management: create, update, promote, dismiss.

use uuid::Uuid;

use crate::models::{uuid_bytes, ClusterRow, NewCluster};

use super::Database;

impl Database {
    /// Assign a candidate to a cluster.
    pub async fn assign_candidate_to_cluster(
        &self,
        detection_id: &[u8],
        cluster_id: i64,
    ) -> Result<(), crate::StoreError> {
        sqlx::query!(
            "UPDATE candidate_embeddings SET cluster_id = $1 WHERE detection_id = $2",
            cluster_id,
            detection_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Create a new cluster and return its auto-generated ID.
    pub async fn insert_cluster(&self, c: &NewCluster<'_>) -> Result<i64, crate::StoreError> {
        let result = sqlx::query!(
            "INSERT INTO candidate_clusters (scientific_name, centroid, centroid_dim, member_count, distinct_days, first_seen_at, last_seen_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            c.scientific_name,
            c.centroid,
            c.centroid_dim,
            c.member_count,
            c.distinct_days,
            c.first_seen_at,
            c.last_seen_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    /// Update a cluster's centroid, counts, and time range.
    pub async fn update_cluster(
        &self,
        cluster_id: i64,
        centroid: &[u8],
        member_count: i64,
        distinct_days: i64,
        first_seen_at: i64,
        last_seen_at: i64,
    ) -> Result<(), crate::StoreError> {
        sqlx::query!(
            "UPDATE candidate_clusters
             SET centroid = $1, member_count = $2, distinct_days = $3,
                 first_seen_at = $4, last_seen_at = $5
             WHERE id = $6",
            centroid,
            member_count,
            distinct_days,
            first_seen_at,
            last_seen_at,
            cluster_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load all pending clusters for a species (for the clustering pass).
    pub async fn pending_clusters(
        &self,
        scientific_name: &str,
    ) -> Result<Vec<ClusterRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", scientific_name, centroid, centroid_dim,
                      member_count, distinct_days, first_seen_at, last_seen_at,
                      status, individual_id
               FROM candidate_clusters
               WHERE scientific_name = $1 AND status = 'pending'
               ORDER BY member_count DESC"#,
            scientific_name,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ClusterRow {
                id: r.id,
                scientific_name: r.scientific_name,
                centroid: r.centroid,
                centroid_dim: r.centroid_dim,
                member_count: r.member_count,
                distinct_days: r.distinct_days,
                first_seen_at: r.first_seen_at,
                last_seen_at: r.last_seen_at,
                status: r.status,
                individual_id: r.individual_id,
            })
            .collect())
    }

    /// List all pending clusters that meet readiness criteria.
    pub async fn ready_clusters(
        &self,
        min_members: i64,
        min_days: i64,
    ) -> Result<Vec<ClusterRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", scientific_name, centroid, centroid_dim,
                      member_count, distinct_days, first_seen_at, last_seen_at,
                      status, individual_id
               FROM candidate_clusters
               WHERE status = 'pending'
                 AND member_count >= $1
                 AND distinct_days >= $2
               ORDER BY member_count DESC"#,
            min_members,
            min_days,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ClusterRow {
                id: r.id,
                scientific_name: r.scientific_name,
                centroid: r.centroid,
                centroid_dim: r.centroid_dim,
                member_count: r.member_count,
                distinct_days: r.distinct_days,
                first_seen_at: r.first_seen_at,
                last_seen_at: r.last_seen_at,
                status: r.status,
                individual_id: r.individual_id,
            })
            .collect())
    }

    /// Get a single cluster by ID.
    pub async fn get_cluster(&self, cluster_id: i64) -> Result<Option<ClusterRow>, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT id AS "id!", scientific_name, centroid, centroid_dim,
                      member_count, distinct_days, first_seen_at, last_seen_at,
                      status, individual_id
               FROM candidate_clusters WHERE id = $1"#,
            cluster_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ClusterRow {
            id: r.id,
            scientific_name: r.scientific_name,
            centroid: r.centroid,
            centroid_dim: r.centroid_dim,
            member_count: r.member_count,
            distinct_days: r.distinct_days,
            first_seen_at: r.first_seen_at,
            last_seen_at: r.last_seen_at,
            status: r.status,
            individual_id: r.individual_id,
        }))
    }

    /// Mark a cluster as enrolled, linking it to an individual.
    pub async fn enroll_cluster(
        &self,
        cluster_id: i64,
        individual_id: &Uuid,
    ) -> Result<(), crate::StoreError> {
        let ind_bytes = uuid_bytes(individual_id);
        sqlx::query!(
            "UPDATE candidate_clusters SET status = 'enrolled', individual_id = $1 WHERE id = $2",
            ind_bytes,
            cluster_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Dismiss a cluster (hide from suggestions).
    pub async fn dismiss_cluster(&self, cluster_id: i64) -> Result<(), crate::StoreError> {
        sqlx::query!(
            "UPDATE candidate_clusters SET status = 'dismissed' WHERE id = $1",
            cluster_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get detection IDs belonging to a cluster (for enrollment linking).
    pub async fn cluster_detection_ids(
        &self,
        cluster_id: i64,
    ) -> Result<Vec<Vec<u8>>, crate::StoreError> {
        let rows = sqlx::query!(
            "SELECT detection_id FROM candidate_embeddings WHERE cluster_id = $1",
            cluster_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.detection_id).collect())
    }

    /// Get all member timestamps for a cluster (for distinct_days computation).
    pub async fn cluster_member_timestamps(
        &self,
        cluster_id: i64,
    ) -> Result<Vec<i64>, crate::StoreError> {
        let rows = sqlx::query!(
            "SELECT created_at FROM candidate_embeddings WHERE cluster_id = $1",
            cluster_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.created_at).collect())
    }
}

//! Individual bird enrollment, matching, and embedding queries.

use uuid::Uuid;

use crate::models::{uuid_bytes, IndividualRow, NewIndividual};

use super::Database;

impl Database {
    /// Insert a new individual.
    pub async fn insert_individual(
        &self,
        ind: &NewIndividual<'_>,
    ) -> Result<(), crate::StoreError> {
        let id = uuid_bytes(ind.id);
        sqlx::query!(
            "INSERT INTO individuals (id, scientific_name, label, reference_embedding, reference_embedding_dim, enrolled_at, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            id,
            ind.scientific_name,
            ind.label,
            ind.reference_embedding,
            ind.reference_embedding_dim,
            ind.enrolled_at,
            ind.notes,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete all individuals and their matches. Returns the number of individuals deleted.
    pub async fn delete_all_individuals(&self) -> Result<u64, crate::StoreError> {
        let result = sqlx::query!("DELETE FROM individuals")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Fetch a single individual by ID.
    pub async fn get_individual(
        &self,
        id: &[u8],
    ) -> Result<Option<IndividualRow>, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT i.id, i.scientific_name, i.label,
                      i.reference_embedding, i.reference_embedding_dim,
                      i.enrolled_at, i.notes,
                      (SELECT l.common_name FROM labels l
                       WHERE l.scientific_name = i.scientific_name LIMIT 1) AS "common_name?"
               FROM individuals i WHERE i.id = $1"#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| IndividualRow {
            id: r.id,
            scientific_name: r.scientific_name,
            common_name: r.common_name,
            label: r.label,
            reference_embedding: r.reference_embedding,
            reference_embedding_dim: r.reference_embedding_dim,
            enrolled_at: r.enrolled_at,
            notes: r.notes,
        }))
    }

    /// List all individuals, optionally filtered by species.
    pub async fn list_individuals(
        &self,
        species: Option<&str>,
    ) -> Result<Vec<IndividualRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT i.id, i.scientific_name, i.label,
                      i.reference_embedding, i.reference_embedding_dim,
                      i.enrolled_at, i.notes,
                      (SELECT l.common_name FROM labels l
                       WHERE l.scientific_name = i.scientific_name LIMIT 1) AS "common_name?"
               FROM individuals i
               WHERE ($1 IS NULL OR i.scientific_name = $1)
               ORDER BY i.enrolled_at DESC"#,
            species,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| IndividualRow {
                id: r.id,
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                label: r.label,
                reference_embedding: r.reference_embedding,
                reference_embedding_dim: r.reference_embedding_dim,
                enrolled_at: r.enrolled_at,
                notes: r.notes,
            })
            .collect())
    }

    /// Load all individuals with non-NULL reference embeddings (for matcher cache).
    pub async fn load_reference_embeddings(
        &self,
    ) -> Result<Vec<IndividualRow>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT i.id, i.scientific_name, i.label,
                      i.reference_embedding, i.reference_embedding_dim,
                      i.enrolled_at, i.notes,
                      (SELECT l.common_name FROM labels l
                       WHERE l.scientific_name = i.scientific_name LIMIT 1) AS "common_name?"
               FROM individuals i
               WHERE i.reference_embedding IS NOT NULL"#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| IndividualRow {
                id: r.id,
                scientific_name: r.scientific_name,
                common_name: r.common_name,
                label: r.label,
                reference_embedding: r.reference_embedding,
                reference_embedding_dim: r.reference_embedding_dim,
                enrolled_at: r.enrolled_at,
                notes: r.notes,
            })
            .collect())
    }

    /// Fetch the embedding for a specific detection.
    pub async fn get_embedding_for_detection(
        &self,
        detection_id: &[u8],
    ) -> Result<Option<Vec<u8>>, crate::StoreError> {
        let row = sqlx::query!(
            "SELECT embedding FROM embeddings WHERE detection_id = $1",
            detection_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.embedding))
    }

    /// Insert an individual match record.
    pub async fn insert_individual_match(
        &self,
        id: &Uuid,
        individual_id: &Uuid,
        detection_id: &Uuid,
        similarity: f64,
        matched_at: i64,
    ) -> Result<(), crate::StoreError> {
        let id_bytes = uuid_bytes(id);
        let ind_bytes = uuid_bytes(individual_id);
        let det_bytes = uuid_bytes(detection_id);
        sqlx::query!(
            "INSERT INTO individual_matches (id, individual_id, detection_id, similarity, matched_at)
             VALUES ($1, $2, $3, $4, $5)",
            id_bytes,
            ind_bytes,
            det_bytes,
            similarity,
            matched_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

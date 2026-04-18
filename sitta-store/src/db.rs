use std::collections::HashMap;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{
    uuid_bytes, CandidateRow, ClusterRow, DetectionRow, HourlyActivityRow, IndividualRow,
    NewAudioSource, NewDetection, NewIndividual, NewLabel, NewModel, NewPrediction, NewStation,
    PredictionRow, ReviewRow, SpeciesSummaryRow,
};

/// Connection to the Sitta SQLite database.
///
/// Wraps a [`SqlitePool`] that manages connection lifecycle, write
/// serialization, and concurrent reads under WAL mode. `Clone` is cheap
/// (the pool is `Arc`-backed) — pass it freely across async tasks.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open (or create) the database at `path`, run connection PRAGMAs,
    /// enable WAL mode, and apply any pending migrations.
    pub async fn open(path: &Path) -> Result<Self, crate::StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000")
            .pragma("synchronous", "NORMAL")
            .pragma("cache_size", "-8000")
            .pragma("temp_store", "MEMORY")
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(crate::StoreError::Open)?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(crate::StoreError::Migrate)?;

        Ok(Self { pool })
    }

    /// Access the underlying pool for queries.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Shut down the pool, waiting for in-flight queries to complete.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    // ── Seeding (called once at startup) ────────────────────────

    /// Insert or replace a station from config.
    pub async fn upsert_station(&self, station: &NewStation<'_>) -> Result<(), crate::StoreError> {
        let id = uuid_bytes(station.id);
        sqlx::query!(
            "INSERT INTO stations (id, name, latitude, longitude) VALUES ($1, $2, $3, $4)
             ON CONFLICT(id) DO UPDATE SET name = $2, latitude = $3, longitude = $4",
            id,
            station.name,
            station.latitude,
            station.longitude,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert or replace an audio source from config.
    pub async fn upsert_audio_source(
        &self,
        source: &NewAudioSource<'_>,
    ) -> Result<(), crate::StoreError> {
        let id = uuid_bytes(source.id);
        let station_id = uuid_bytes(source.station_id);
        sqlx::query!(
            "INSERT INTO audio_sources (id, station_id, name, source_type, uri, sample_rate, channels)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT(id) DO UPDATE SET name = $3, source_type = $4, uri = $5, sample_rate = $6, channels = $7",
            id,
            station_id,
            source.name,
            source.source_type,
            source.uri,
            source.sample_rate,
            source.channels,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert a model (or no-op if it already exists). Returns the INTEGER PK.
    pub async fn upsert_model(&self, model: &NewModel<'_>) -> Result<i64, crate::StoreError> {
        let has_embeddings = model.has_embeddings as i64;
        sqlx::query!(
            "INSERT INTO models (name, version, sample_rate, window_samples, has_embeddings, embedding_dim)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(name, version) DO NOTHING",
            model.name,
            model.version,
            model.sample_rate,
            model.window_samples,
            has_embeddings,
            model.embedding_dim,
        )
        .execute(&self.pool)
        .await?;

        // Fetch the id (whether just inserted or already existed).
        let row = sqlx::query!(
            r#"SELECT id AS "id!" FROM models WHERE name = $1 AND version = $2"#,
            model.name,
            model.version,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.id)
    }

    /// Bulk-insert a model's label set. Existing labels are skipped.
    pub async fn seed_labels(&self, labels: &[NewLabel<'_>]) -> Result<(), crate::StoreError> {
        let mut tx = self.pool.begin().await?;
        for label in labels {
            sqlx::query!(
                "INSERT OR IGNORE INTO labels (model_id, label_index, scientific_name, common_name, label_type, taxon_code)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                label.model_id,
                label.label_index,
                label.scientific_name,
                label.common_name,
                label.label_type,
                label.taxon_code,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Load the full (model_id, label_index) → label.id mapping for
    /// in-memory caching. Called once at startup.
    pub async fn load_label_id_cache(
        &self,
    ) -> Result<HashMap<(i64, i64), i64>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", model_id, label_index FROM labels"#
        )
        .fetch_all(&self.pool)
        .await?;

        let mut cache = HashMap::with_capacity(rows.len());
        for row in rows {
            cache.insert((row.model_id, row.label_index), row.id);
        }
        Ok(cache)
    }

    // ── Detection writes (called on every inference result) ─────

    /// Insert a single detection.
    pub async fn insert_detection(
        &self,
        det: &NewDetection<'_>,
    ) -> Result<(), crate::StoreError> {
        let id = uuid_bytes(det.id);
        let station_id = uuid_bytes(det.station_id);
        let source_id = det.source_id.map(uuid_bytes);
        sqlx::query!(
            "INSERT INTO detections (id, station_id, source_id, model_id, label_id, detected_at, confidence, snippet_path, snippet_duration_ms, snippet_sample_rate, metadata)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
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

    // ── Read queries (for API endpoints) ────────────────────────

    /// Recent detections with joined label/model/source info.
    /// Optional species filter by scientific name.
    pub async fn recent_detections(
        &self,
        since: i64,
        until: i64,
        limit: i64,
        offset: i64,
        species: Option<&str>,
        min_confidence: Option<f64>,
    ) -> Result<Vec<DetectionRow>, crate::StoreError> {
        let conf_floor = min_confidence.unwrap_or(0.0);
        let rows = sqlx::query!(
            r#"SELECT d.id, d.detected_at, d.confidence AS "confidence!: f64",
                      d.snippet_path, d.metadata,
                      l.scientific_name, l.common_name AS "common_name!",
                      l.taxon_code,
                      m.name AS "model_name!", m.version AS "model_version!",
                      s.name AS "source_name?",
                      (EXISTS (SELECT 1 FROM embeddings e WHERE e.detection_id = d.id)) AS "has_embedding!: bool"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               JOIN models m ON m.id = d.model_id
               LEFT JOIN audio_sources s ON s.id = d.source_id
               WHERE d.detected_at >= $1 AND d.detected_at <= $2
                 AND d.confidence >= $6
                 AND ($5 IS NULL OR l.scientific_name = $5)
               ORDER BY d.detected_at DESC
               LIMIT $3 OFFSET $4"#,
            since,
            until,
            limit,
            offset,
            species,
            conf_floor,
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
            })
            .collect())
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
                      (EXISTS (SELECT 1 FROM embeddings e WHERE e.detection_id = d.id)) AS "has_embedding!: bool"
               FROM detections d
               JOIN labels l ON l.id = d.label_id
               JOIN models m ON m.id = d.model_id
               LEFT JOIN audio_sources s ON s.id = d.source_id
               WHERE d.id = $1"#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| DetectionRow {
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
    ///
    /// `since` is the epoch-ms start of the window (e.g. local midnight);
    /// the hour bucket is computed as `(detected_at - since) / 3_600_000`.
    /// Callers should set `until = since + 86_400_000` for a full day.
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

    /// Total detection count (for status endpoint).
    pub async fn detection_count(&self) -> Result<i64, crate::StoreError> {
        let row = sqlx::query!(r#"SELECT COUNT(*) AS "count!" FROM detections"#)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.count)
    }

    // ── Individual queries ──────────────────────────────────────

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

    /// Fetch a single individual by ID.
    pub async fn get_individual(
        &self,
        id: &[u8],
    ) -> Result<Option<IndividualRow>, crate::StoreError> {
        let row = sqlx::query!(
            r#"SELECT id, scientific_name, label,
                      reference_embedding, reference_embedding_dim,
                      enrolled_at, notes
               FROM individuals WHERE id = $1"#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| IndividualRow {
            id: r.id,
            scientific_name: r.scientific_name,
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
            r#"SELECT id, scientific_name, label,
                      reference_embedding, reference_embedding_dim,
                      enrolled_at, notes
               FROM individuals
               WHERE ($1 IS NULL OR scientific_name = $1)
               ORDER BY enrolled_at DESC"#,
            species,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| IndividualRow {
                id: r.id,
                scientific_name: r.scientific_name,
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
            r#"SELECT id, scientific_name, label,
                      reference_embedding, reference_embedding_dim,
                      enrolled_at, notes
               FROM individuals
               WHERE reference_embedding IS NOT NULL"#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| IndividualRow {
                id: r.id,
                scientific_name: r.scientific_name,
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

    // ── Snippet update ──────────────────────────────────────────

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
                      (EXISTS (SELECT 1 FROM embeddings e WHERE e.detection_id = d.id)) AS "has_embedding!: bool"
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
            })
            .collect())
    }

    // ── Detection reviews ──────────────────────────────────────

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

    // ── Individual queries ──────────────────────────────────────

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

    // ── Candidate clustering queries ───────────────────────────

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
    pub async fn insert_cluster(
        &self,
        scientific_name: &str,
        centroid: &[u8],
        centroid_dim: i64,
        member_count: i64,
        distinct_days: i64,
        first_seen_at: i64,
        last_seen_at: i64,
    ) -> Result<i64, crate::StoreError> {
        let result = sqlx::query!(
            "INSERT INTO candidate_clusters (scientific_name, centroid, centroid_dim, member_count, distinct_days, first_seen_at, last_seen_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            scientific_name,
            centroid,
            centroid_dim,
            member_count,
            distinct_days,
            first_seen_at,
            last_seen_at,
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

    /// List all pending clusters that meet readiness criteria, sorted by member_count desc.
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

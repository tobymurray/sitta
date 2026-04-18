use std::collections::HashMap;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{
    uuid_bytes, DetectionRow, IndividualRow, NewAudioSource, NewDetection, NewIndividual, NewLabel,
    NewModel, NewPrediction, NewStation, PredictionRow, SpeciesSummaryRow,
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
            "INSERT OR REPLACE INTO stations (id, name, latitude, longitude) VALUES ($1, $2, $3, $4)",
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
            "INSERT OR REPLACE INTO audio_sources (id, station_id, name, source_type, uri, sample_rate, channels)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
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

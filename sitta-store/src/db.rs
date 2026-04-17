use std::collections::HashMap;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::models::{
    uuid_bytes, NewAudioSource, NewDetection, NewLabel, NewModel, NewPrediction, NewStation,
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
}

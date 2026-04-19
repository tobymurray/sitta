//! Startup seeding: stations, sources, models, labels.

use std::collections::HashMap;

use crate::models::{uuid_bytes, NewAudioSource, NewLabel, NewModel, NewStation};

use super::Database;

impl Database {
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
}

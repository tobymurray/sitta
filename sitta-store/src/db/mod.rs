//! Database access layer, split by domain.

mod candidates;
mod clustering;
mod detections;
mod individuals;
mod rarity;
mod reviews;
mod seeding;
mod sessions;
mod snippets;
mod species;

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

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
}

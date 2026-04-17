//! Local storage for detections and embedding vectors.

pub mod db;
pub mod models;

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to open database: {0}")]
    Open(sqlx::Error),

    #[error("migration failed: {0}")]
    Migrate(sqlx::migrate::MigrateError),

    #[error("query failed: {0}")]
    Query(#[from] sqlx::Error),

    #[error("invalid UUID blob: expected 16 bytes, got {0}")]
    InvalidUuid(usize),
}

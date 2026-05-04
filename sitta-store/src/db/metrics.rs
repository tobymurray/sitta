//! Lifetime metric counters that survive process restarts.
//!
//! The audio pipeline keeps in-memory atomics for `clips_saved`,
//! `clips_dropped`, etc., but with frequent rolling restarts during
//! development those reset to zero on every boot. We mirror the atomics
//! to a tiny key-value table so the diagnostics page can show counts
//! that span restarts.

use std::collections::HashMap;

use super::Database;

impl Database {
    /// Load every named lifetime counter into a map. Missing keys are
    /// silently absent — caller decides the default.
    pub async fn load_lifetime_metrics(&self) -> Result<HashMap<String, u64>, crate::StoreError> {
        let rows = sqlx::query!(
            r#"SELECT key, value AS "value!: i64" FROM lifetime_metrics"#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.key, r.value.max(0) as u64))
            .collect())
    }

    /// Upsert a lifetime counter to `value`. Used by the periodic flush
    /// task that mirrors in-memory atomics back to disk. Idempotent —
    /// writing the same value twice is a no-op.
    pub async fn set_lifetime_metric(
        &self,
        key: &str,
        value: u64,
    ) -> Result<(), crate::StoreError> {
        // SQLite stores INTEGER as i64; clamp to avoid overflow.
        let v = i64::try_from(value).unwrap_or(i64::MAX);
        sqlx::query!(
            "INSERT INTO lifetime_metrics (key, value) VALUES ($1, $2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            key,
            v,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

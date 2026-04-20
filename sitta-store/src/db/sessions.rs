//! Source session (effort tracking) queries.

use crate::models::{uuid_bytes, NewSession, SessionRow, SourceEffortRow};
use crate::StoreError;

use super::Database;

impl Database {
    /// Start a new source session.
    pub async fn start_session(&self, session: &NewSession<'_>) -> Result<(), StoreError> {
        let id = uuid_bytes(session.id);
        let source_id = uuid_bytes(session.source_id);
        sqlx::query!(
            "INSERT INTO source_sessions (id, source_id, started_at) VALUES (?, ?, ?)",
            id,
            source_id,
            session.started_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// End a session by setting ended_at and reason.
    pub async fn end_session(
        &self,
        session_id: &[u8],
        ended_at: i64,
        reason: &str,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            "UPDATE source_sessions SET ended_at = ?, end_reason = ? WHERE id = ?",
            ended_at,
            reason,
            session_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Add `count` to the chunk counter for an active session.
    pub async fn add_session_chunks(
        &self,
        session_id: &[u8],
        count: i64,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            "UPDATE source_sessions SET chunks_received = chunks_received + ? WHERE id = ?",
            count,
            session_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// End all sessions that are still active (ended_at IS NULL).
    pub async fn end_all_active_sessions(
        &self,
        ended_at: i64,
        reason: &str,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query!(
            "UPDATE source_sessions SET ended_at = ?, end_reason = ? WHERE ended_at IS NULL",
            ended_at,
            reason,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Get all currently active sessions.
    pub async fn active_sessions(&self) -> Result<Vec<SessionRow>, StoreError> {
        let rows = sqlx::query_as!(
            SessionRow,
            r#"SELECT
                s.id AS "id!",
                s.source_id AS "source_id!",
                a.name AS "source_name!",
                s.started_at AS "started_at!",
                s.ended_at,
                s.end_reason,
                s.chunks_received AS "chunks_received!"
            FROM source_sessions s
            JOIN audio_sources a ON a.id = s.source_id
            WHERE s.ended_at IS NULL
            ORDER BY s.started_at"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Per-source effort summary for a time range.
    ///
    /// For each source, computes total recording seconds and session count.
    /// Sessions that overlap the window are clamped to the window boundaries.
    pub async fn effort_summary(
        &self,
        since: i64,
        until: i64,
    ) -> Result<Vec<SourceEffortRow>, StoreError> {
        let rows = sqlx::query_as!(
            SourceEffortRow,
            r#"SELECT
                a.name AS source_name,
                CAST(SUM(
                    (MIN(COALESCE(s.ended_at, ?2), ?2) - MAX(s.started_at, ?1))
                ) AS REAL) / 1000.0 AS "total_seconds: f64",
                COUNT(*) AS session_count
            FROM source_sessions s
            JOIN audio_sources a ON a.id = s.source_id
            WHERE s.started_at < ?2
              AND (s.ended_at IS NULL OR s.ended_at > ?1)
            GROUP BY s.source_id
            ORDER BY source_name"#,
            since,
            until,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Total recording seconds across all sources in a time range.
    pub async fn total_effort_seconds(&self, since: i64, until: i64) -> Result<f64, StoreError> {
        let row = sqlx::query_scalar!(
            r#"SELECT CAST(COALESCE(SUM(
                MIN(COALESCE(s.ended_at, ?2), ?2) - MAX(s.started_at, ?1)
            ), 0) AS REAL) / 1000.0 AS "total: f64"
            FROM source_sessions s
            WHERE s.started_at < ?2
              AND (s.ended_at IS NULL OR s.ended_at > ?1)"#,
            since,
            until,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}

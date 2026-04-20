//! Automatic effort tracking: monitors the audio broadcast channel and
//! records source sessions (start/end times) in the database.
//!
//! A session starts when the first chunk from a source arrives (or after a
//! gap longer than `gap_timeout`). A session ends when no chunks have been
//! received for that duration, or on shutdown/removal.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use sitta_audio::chunk::AudioChunk;
use sitta_store::db::Database;
use sitta_store::models::NewSession;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// State for a single source's current session.
struct SourceState {
    /// Database session ID (UUIDv7).
    session_id: Uuid,
    /// Wall-clock time the session started (for DB).
    started_at_ms: i64,
    /// Monotonic instant of the last chunk received.
    last_chunk: Instant,
    /// Accumulated chunk count (batched to reduce DB writes).
    pending_chunks: i64,
}

/// Spawn the effort tracker as a background task.
///
/// `source_ids` maps source display names to their database UUIDs.
/// `gap_timeout` is how long to wait without chunks before closing a session.
pub fn spawn_effort_tracker(
    db: Database,
    source_ids: Arc<HashMap<String, Uuid>>,
    mut rx: broadcast::Receiver<Arc<AudioChunk>>,
    shutdown: CancellationToken,
    gap_timeout: Duration,
) {
    tokio::spawn(async move {
        // Close any sessions left open by a prior unclean shutdown.
        let now_ms = Utc::now().timestamp_millis();
        match db.end_all_active_sessions(now_ms, "startup_cleanup").await {
            Ok(n) if n > 0 => tracing::info!(closed = n, "Closed stale sessions from prior run"),
            Err(e) => tracing::error!(error = %e, "Failed to clean up stale sessions"),
            _ => {}
        }

        let mut sources: HashMap<String, SourceState> = HashMap::new();
        // Check for stale sessions every second.
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(chunk) => {
                            handle_chunk(&db, &source_ids, &mut sources, &chunk).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(dropped = n, "Effort tracker lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = tick.tick() => {
                    close_stale_sessions(&db, &mut sources, gap_timeout).await;
                }
                () = shutdown.cancelled() => break,
            }
        }

        // Shutdown: flush pending chunks and close all active sessions.
        let now_ms = Utc::now().timestamp_millis();
        for (name, mut state) in sources.drain() {
            flush_chunks(&db, &mut state).await;
            if let Err(e) = db
                .end_session(
                    state.session_id.as_bytes().as_slice(),
                    now_ms,
                    "shutdown",
                )
                .await
            {
                tracing::error!(source = %name, error = %e, "Failed to end session on shutdown");
            }
        }

        tracing::info!("Effort tracker stopped");
    });
}

async fn handle_chunk(
    db: &Database,
    source_ids: &HashMap<String, Uuid>,
    sources: &mut HashMap<String, SourceState>,
    chunk: &AudioChunk,
) {
    let now = Instant::now();

    if let Some(state) = sources.get_mut(&chunk.source_name) {
        // Existing session — update last-seen time.
        state.last_chunk = now;
        state.pending_chunks += 1;

        // Flush chunk count to DB periodically (every 10 chunks).
        if state.pending_chunks >= 10 {
            flush_chunks(db, state).await;
        }
    } else {
        // New session — look up source UUID and start.
        let Some(source_id) = source_ids.get(&chunk.source_name) else {
            return;
        };

        let session_id = Uuid::now_v7();
        let started_at = Utc::now().timestamp_millis();

        let session = NewSession {
            id: &session_id,
            source_id,
            started_at,
        };

        if let Err(e) = db.start_session(&session).await {
            tracing::error!(source = %chunk.source_name, error = %e, "Failed to start session");
            return;
        }

        tracing::info!(source = %chunk.source_name, "Recording session started");

        sources.insert(
            chunk.source_name.clone(),
            SourceState {
                session_id,
                started_at_ms: started_at,
                last_chunk: now,
                pending_chunks: 1,
            },
        );
    }
}

async fn close_stale_sessions(
    db: &Database,
    sources: &mut HashMap<String, SourceState>,
    gap_timeout: Duration,
) {
    let now = Instant::now();
    let now_ms = Utc::now().timestamp_millis();

    let stale: Vec<String> = sources
        .iter()
        .filter(|(_, state)| now.duration_since(state.last_chunk) > gap_timeout)
        .map(|(name, _)| name.clone())
        .collect();

    for name in stale {
        if let Some(mut state) = sources.remove(&name) {
            flush_chunks(db, &mut state).await;
            if let Err(e) = db
                .end_session(state.session_id.as_bytes().as_slice(), now_ms, "gap")
                .await
            {
                tracing::error!(source = %name, error = %e, "Failed to end stale session");
            }
            let duration_s = (now_ms - state.started_at_ms) as f64 / 1000.0;
            tracing::info!(source = %name, duration_s = format!("{duration_s:.0}"), "Recording session ended (gap)");
        }
    }
}

async fn flush_chunks(db: &Database, state: &mut SourceState) {
    if state.pending_chunks == 0 {
        return;
    }
    let id_bytes = state.session_id.as_bytes().as_slice();
    if let Err(e) = db
        .add_session_chunks(id_bytes, state.pending_chunks)
        .await
    {
        tracing::error!(error = %e, "Failed to flush session chunk count");
        return;
    }
    state.pending_chunks = 0;
}

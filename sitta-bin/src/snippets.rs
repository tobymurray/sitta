//! Asynchronous audio snippet writer and retention worker.
//!
//! The snippet writer receives [`SnippetJob`]s from the inference pipeline via
//! a bounded channel and writes WAV files to disk in a background task. Writes
//! are performed inside [`tokio::task::spawn_blocking`] to avoid blocking the
//! async runtime on slow I/O (SD cards).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sitta_api::server::SnippetMetrics;
use sitta_store::db::Database;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::SnippetConfig;

/// A request to save an audio clip to disk.
pub struct SnippetJob {
    pub detection_id: Uuid,
    pub detected_at: DateTime<Utc>,
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Handle for submitting snippet jobs to the background writer.
#[derive(Clone)]
pub struct SnippetWriter {
    tx: mpsc::Sender<SnippetJob>,
    pub metrics: Arc<SnippetMetrics>,
}

impl SnippetWriter {
    /// Submit a snippet job. Returns immediately. If the channel is full
    /// (SD card too slow), the job is dropped with a warning — never blocks.
    pub fn submit(&self, job: SnippetJob) {
        if self.tx.try_send(job).is_err() {
            self.metrics.clips_dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                total_dropped = self.metrics.clips_dropped.load(Ordering::Relaxed),
                "Snippet writer channel full, dropping clip"
            );
        }
    }
}

/// Spawn the background snippet writer task. Returns a [`SnippetWriter`] handle.
pub fn spawn_snippet_writer(
    config: SnippetConfig,
    db: Database,
    shutdown: CancellationToken,
) -> SnippetWriter {
    let (tx, rx) = mpsc::channel::<SnippetJob>(64);
    let metrics = Arc::new(SnippetMetrics::default());
    let writer_metrics = metrics.clone();

    tokio::spawn(writer_loop(config, db, rx, shutdown, writer_metrics));

    SnippetWriter { tx, metrics }
}

async fn writer_loop(
    config: SnippetConfig,
    db: Database,
    mut rx: mpsc::Receiver<SnippetJob>,
    shutdown: CancellationToken,
    metrics: Arc<SnippetMetrics>,
) {
    let clip_dir = PathBuf::from(&config.clip_dir);

    loop {
        tokio::select! {
            job = rx.recv() => {
                let Some(job) = job else { break };
                if let Err(e) = process_job(&clip_dir, &db, &job, &metrics).await {
                    tracing::error!(
                        detection_id = %job.detection_id,
                        error = %e,
                        "Failed to save audio clip"
                    );
                }
            }
            () = shutdown.cancelled() => {
                // Drain remaining jobs before exiting.
                while let Ok(job) = rx.try_recv() {
                    if let Err(e) = process_job(&clip_dir, &db, &job, &metrics).await {
                        tracing::error!(
                            detection_id = %job.detection_id,
                            error = %e,
                            "Failed to save audio clip during shutdown"
                        );
                    }
                }
                break;
            }
        }
    }
    tracing::info!("Snippet writer stopped");
}

async fn process_job(
    clip_dir: &Path,
    db: &Database,
    job: &SnippetJob,
    metrics: &SnippetMetrics,
) -> anyhow::Result<()> {
    let date_str = job.detected_at.format("%Y-%m-%d").to_string();
    let file_name = format!("{}.wav", job.detection_id);
    let rel_path = PathBuf::from(&date_str).join(&file_name);
    let full_path = clip_dir.join(&rel_path);

    let samples = job.samples.clone();
    let sample_rate = job.sample_rate;
    let channels = job.channels;

    // Write WAV in a blocking task (SD card I/O).
    let path_for_write = full_path.clone();
    tokio::task::spawn_blocking(move || {
        sitta_audio::wav::write_wav(&path_for_write, &samples, sample_rate, channels)
    })
    .await??;

    // Compute duration and file size.
    let duration_ms = (job.samples.len() as u64 * 1000) / (u64::from(job.sample_rate) * u64::from(job.channels));
    let file_size = tokio::fs::metadata(&full_path).await?.len();

    // Update the detection row with the snippet path.
    let det_id = job.detection_id.as_bytes().as_slice();
    let path_str = rel_path.to_string_lossy();
    db.update_snippet_path(
        det_id,
        &path_str,
        duration_ms as i64,
        i64::from(job.sample_rate),
    )
    .await?;

    metrics.clips_saved.fetch_add(1, Ordering::Relaxed);
    metrics.bytes_written.fetch_add(file_size, Ordering::Relaxed);

    tracing::debug!(
        detection_id = %job.detection_id,
        path = %rel_path.display(),
        duration_ms = duration_ms,
        size_bytes = file_size,
        "Audio clip saved"
    );

    Ok(())
}

/// Spawn the periodic retention worker.
pub fn spawn_retention_worker(
    config: SnippetConfig,
    db: Database,
    shutdown: CancellationToken,
) {
    tokio::spawn(retention_loop(config, db, shutdown));
}

async fn retention_loop(
    config: SnippetConfig,
    db: Database,
    shutdown: CancellationToken,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    interval.tick().await; // first tick is immediate — skip it
    let clip_dir = PathBuf::from(&config.clip_dir);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = run_retention(&config, &clip_dir, &db).await {
                    tracing::error!(error = %e, "Retention cleanup failed");
                }
            }
            () = shutdown.cancelled() => break,
        }
    }
}

async fn run_retention(
    config: &SnippetConfig,
    clip_dir: &Path,
    db: &Database,
) -> anyhow::Result<()> {
    let mut deleted = 0u64;

    // Age-based cleanup.
    if config.retention_days > 0 {
        let cutoff_ms = Utc::now().timestamp_millis()
            - i64::from(config.retention_days) * 24 * 60 * 60 * 1000;

        let rows = db.detections_with_snippets(10_000).await?;
        for row in &rows {
            if row.detected_at >= cutoff_ms {
                break; // Ordered ASC, so everything after this is newer.
            }
            if db.is_review_correct(&row.id).await? {
                continue; // Never delete clips reviewed as correct.
            }
            if let Some(ref path) = row.snippet_path {
                delete_clip(clip_dir, path).await;
                db.clear_snippet_path(&row.id).await?;
                deleted += 1;
            }
        }
    }

    // Size-based cleanup.
    if config.max_disk_mb > 0 {
        let max_bytes = config.max_disk_mb * 1024 * 1024;
        let total = dir_size(clip_dir).await;
        if total > max_bytes {
            let mut to_free = total - max_bytes;
            let rows = db.detections_with_snippets(10_000).await?;
            for row in &rows {
                if to_free == 0 {
                    break;
                }
                if db.is_review_correct(&row.id).await? {
                    continue;
                }
                if let Some(ref path) = row.snippet_path {
                    let full = clip_dir.join(path);
                    let size = tokio::fs::metadata(&full).await.map(|m| m.len()).unwrap_or(0);
                    delete_clip(clip_dir, path).await;
                    db.clear_snippet_path(&row.id).await?;
                    to_free = to_free.saturating_sub(size);
                    deleted += 1;
                }
            }
        }
    }

    if deleted > 0 {
        tracing::info!(deleted, "Retention cleanup complete");
        // Clean up empty date directories.
        cleanup_empty_dirs(clip_dir).await;
    }

    Ok(())
}

async fn delete_clip(clip_dir: &Path, rel_path: &str) {
    let wav = clip_dir.join(rel_path);
    let _ = tokio::fs::remove_file(&wav).await;
    // Also remove the spectrogram if it exists.
    let png = wav.with_extension("png");
    let _ = tokio::fs::remove_file(&png).await;
}

async fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return 0;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if meta.is_dir() {
            total += Box::pin(dir_size(&entry.path())).await;
        } else {
            total += meta.len();
        }
    }
    total
}

async fn cleanup_empty_dirs(clip_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(clip_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if meta.is_dir()
            && let Ok(mut sub) = tokio::fs::read_dir(entry.path()).await
            && sub.next_entry().await.ok().flatten().is_none()
        {
            let _ = tokio::fs::remove_dir(entry.path()).await;
        }
    }
}

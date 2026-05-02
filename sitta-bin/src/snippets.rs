//! Asynchronous audio snippet writer and retention worker.
//!
//! The snippet writer receives [`SnippetJob`]s from the inference pipeline via
//! a bounded channel and writes WAV files to disk in a background task. Writes
//! are performed inside [`tokio::task::spawn_blocking`] to avoid blocking the
//! async runtime on slow I/O (SD cards).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sitta_api::server::SnippetMetrics;
use sitta_store::db::Database;
use sitta_store::models::RarityRow;
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

/// Eviction tier — lower tiers are evicted first when disk is tight. Higher
/// tiers are more deliberately preserved. Reviewed-as-correct rows skip the
/// retention worker entirely (handled in [`run_retention`]).
fn tier(rarity: Option<&RarityRow>) -> u8 {
    let Some(r) = rarity else { return 0 };
    if r.first_ever {
        return 3;
    }
    if r.first_week || r.first_season {
        return 2;
    }
    if r.first_day || r.score >= 0.6 {
        return 1;
    }
    0
}

/// Apply the strictest matching rarity multiplier. Returns at least 1 so a
/// row's effective retention is never shorter than the configured base.
fn effective_multiplier(rarity: Option<&RarityRow>, cfg: &SnippetConfig) -> u32 {
    let Some(r) = rarity else { return 1 };
    let mut m: u32 = 1;
    if r.first_ever {
        m = m.max(cfg.first_ever_multiplier);
    }
    if r.first_season {
        m = m.max(cfg.first_season_multiplier);
    }
    if r.first_week {
        m = m.max(cfg.first_week_multiplier);
    }
    if r.first_day {
        m = m.max(cfg.first_day_multiplier);
    }
    if r.score >= 0.6 {
        m = m.max(cfg.high_score_multiplier);
    }
    m.max(1)
}

/// In-memory snapshot of a clip we might evict, with the metadata the retention
/// policy needs (rarity tier, review status, species). Built once per sweep.
struct RetentionCandidate {
    id: Vec<u8>,
    snippet_path: String,
    detected_at: i64,
    scientific_name: String,
    rarity: Option<RarityRow>,
    reviewed_correct: bool,
}

async fn run_retention(
    config: &SnippetConfig,
    clip_dir: &Path,
    db: &Database,
) -> anyhow::Result<()> {
    let mut deleted = 0u64;

    // ── Build the candidate set with everything the policy needs.
    // detections_with_snippets returns rows ordered ASC by detected_at and
    // already filtered to those with a non-NULL snippet_path. Per-row rarity
    // and review lookups are N+1 but the worker runs hourly, not per-request.
    let rows = db.detections_with_snippets(10_000).await?;
    let mut candidates: Vec<RetentionCandidate> = Vec::with_capacity(rows.len());
    for r in rows {
        let Some(snippet_path) = r.snippet_path.clone() else {
            continue;
        };
        let rarity = db.get_rarity(&r.id).await?;
        let reviewed_correct = db.is_review_correct(&r.id).await?;
        candidates.push(RetentionCandidate {
            id: r.id,
            snippet_path,
            detected_at: r.detected_at,
            scientific_name: r.scientific_name.unwrap_or_default(),
            rarity,
            reviewed_correct,
        });
    }

    let now_ms = Utc::now().timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;

    // ── Age sweep: each row gets its own tier-adjusted cutoff. The old
    // "break on first row past cutoff" optimisation is gone because the
    // cutoff varies per row, but 10k iterations is trivial.
    if config.retention_days > 0 {
        let mut to_evict: Vec<usize> = Vec::new();
        for (i, c) in candidates.iter().enumerate() {
            if c.reviewed_correct {
                continue;
            }
            let mul = effective_multiplier(c.rarity.as_ref(), config);
            let span_days = i64::from(config.retention_days).saturating_mul(i64::from(mul));
            let cutoff = now_ms.saturating_sub(span_days.saturating_mul(day_ms));
            if c.detected_at < cutoff {
                to_evict.push(i);
            }
        }
        for &i in &to_evict {
            let c = &candidates[i];
            delete_clip(clip_dir, &c.snippet_path).await;
            db.clear_snippet_path(&c.id).await?;
            deleted += 1;
        }
        // Drop evicted rows from the candidate set so later sweeps don't see
        // them. swap_remove from the back keeps remaining indices valid.
        to_evict.sort_unstable();
        for i in to_evict.into_iter().rev() {
            candidates.swap_remove(i);
        }
    }

    // ── Per-species cap pre-pass: when the cap is set and disk pressure is
    // a concern, trim noisy species back to the cap before global eviction
    // touches anything else. Reviewed-correct and first_ever clips are
    // exempt — they don't count toward the cap and won't be evicted by it.
    if config.per_species_cap > 0 && config.max_disk_mb > 0 {
        let cap = config.per_species_cap as usize;
        let mut by_species: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, c) in candidates.iter().enumerate() {
            if c.reviewed_correct {
                continue;
            }
            if c.rarity.as_ref().is_some_and(|r| r.first_ever) {
                continue;
            }
            by_species
                .entry(c.scientific_name.clone())
                .or_default()
                .push(i);
        }
        let mut to_evict: Vec<usize> = Vec::new();
        for (_, mut indices) in by_species {
            if indices.len() <= cap {
                continue;
            }
            indices.sort_by_key(|&i| candidates[i].detected_at);
            let excess = indices.len() - cap;
            to_evict.extend(indices.into_iter().take(excess));
        }
        for &i in &to_evict {
            let c = &candidates[i];
            delete_clip(clip_dir, &c.snippet_path).await;
            db.clear_snippet_path(&c.id).await?;
            deleted += 1;
        }
        to_evict.sort_unstable();
        for i in to_evict.into_iter().rev() {
            candidates.swap_remove(i);
        }
    }

    // ── Size sweep: evict in (tier ASC, age ASC) order so the least
    // protected, oldest clips go first. Reviewed-correct is always skipped.
    if config.max_disk_mb > 0 {
        let max_bytes = config.max_disk_mb * 1024 * 1024;
        let total = dir_size(clip_dir).await;
        if total > max_bytes {
            let mut to_free = total - max_bytes;
            let mut order: Vec<usize> = (0..candidates.len()).collect();
            order.sort_by_key(|&i| {
                let c = &candidates[i];
                (tier(c.rarity.as_ref()), c.detected_at)
            });
            for i in order {
                if to_free == 0 {
                    break;
                }
                let c = &candidates[i];
                if c.reviewed_correct {
                    continue;
                }
                let full = clip_dir.join(&c.snippet_path);
                let size = tokio::fs::metadata(&full)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                delete_clip(clip_dir, &c.snippet_path).await;
                db.clear_snippet_path(&c.id).await?;
                to_free = to_free.saturating_sub(size);
                deleted += 1;
            }
        }
    }

    if deleted > 0 {
        tracing::info!(deleted, "Retention cleanup complete");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rr(first_ever: bool, first_season: bool, first_week: bool, first_day: bool, score: f64) -> RarityRow {
        RarityRow {
            detection_id: vec![],
            score,
            first_ever,
            first_season,
            first_week,
            first_day,
            days_since_last: None,
            local_count: 0,
            range_score: None,
            temporal_score: 0.0,
        }
    }

    fn cfg() -> SnippetConfig {
        SnippetConfig {
            enabled: true,
            clip_dir: "/tmp".to_string(),
            retention_days: 30,
            max_disk_mb: 1024,
            first_ever_multiplier: 999,
            first_season_multiplier: 8,
            first_week_multiplier: 4,
            first_day_multiplier: 2,
            high_score_multiplier: 2,
            per_species_cap: 0,
        }
    }

    #[test]
    fn tier_orders_by_protection() {
        // Common (no flags, low score) is the easiest to evict.
        assert_eq!(tier(None), 0);
        assert_eq!(tier(Some(&rr(false, false, false, false, 0.1))), 0);

        // Boosted: first_day or score >= 0.6.
        assert_eq!(tier(Some(&rr(false, false, false, true, 0.0))), 1);
        assert_eq!(tier(Some(&rr(false, false, false, false, 0.7))), 1);

        // Long: first_week / first_season.
        assert_eq!(tier(Some(&rr(false, false, true, false, 0.0))), 2);
        assert_eq!(tier(Some(&rr(false, true, false, false, 0.0))), 2);

        // Forever: first_ever (highest).
        assert_eq!(tier(Some(&rr(true, false, false, false, 0.0))), 3);

        // Multi-flag rows take the strongest tier.
        assert_eq!(tier(Some(&rr(true, true, true, true, 0.9))), 3);
    }

    #[test]
    fn effective_multiplier_picks_strictest_match() {
        let c = cfg();
        // No rarity row: baseline (1).
        assert_eq!(effective_multiplier(None, &c), 1);
        // Common with low score: still 1.
        assert_eq!(effective_multiplier(Some(&rr(false, false, false, false, 0.3)), &c), 1);
        // Just first_day: 2.
        assert_eq!(effective_multiplier(Some(&rr(false, false, false, true, 0.0)), &c), 2);
        // first_day + high score: max(2, 2) = 2.
        assert_eq!(effective_multiplier(Some(&rr(false, false, false, true, 0.7)), &c), 2);
        // first_week beats first_day.
        assert_eq!(effective_multiplier(Some(&rr(false, false, true, true, 0.0)), &c), 4);
        // first_season beats first_week.
        assert_eq!(effective_multiplier(Some(&rr(false, true, true, true, 0.0)), &c), 8);
        // first_ever beats everything.
        assert_eq!(effective_multiplier(Some(&rr(true, true, true, true, 0.95)), &c), 999);
    }

    #[test]
    fn effective_multiplier_floors_at_one() {
        // A user who configures multipliers below 1 shouldn't accidentally
        // shorten retention below the base — clamp to 1.
        let mut c = cfg();
        c.first_day_multiplier = 0;
        c.high_score_multiplier = 0;
        assert_eq!(effective_multiplier(Some(&rr(false, false, false, true, 0.7)), &c), 1);
    }
}

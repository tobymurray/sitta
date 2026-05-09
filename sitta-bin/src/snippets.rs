//! Asynchronous audio snippet writer and retention worker.
//!
//! The snippet writer receives [`SnippetJob`]s from the inference pipeline via
//! a bounded channel and writes WAV files to disk in a background task. Writes
//! are performed inside [`tokio::task::spawn_blocking`] to avoid blocking the
//! async runtime on slow I/O (SD cards).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
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
    /// If the channel is closed (writer task dead), the failure escalates
    /// to error level so the operator notices.
    pub fn submit(&self, job: SnippetJob) {
        match self.tx.try_send(job) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.clips_dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    total_dropped = self.metrics.clips_dropped.load(Ordering::Relaxed),
                    "Snippet writer channel full, dropping clip",
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // The writer task is dead. Every subsequent submit will
                // hit this branch — we count it as a failure (not a drop)
                // because the cause is different and the remedy is a
                // process restart, not "make the disk faster".
                self.metrics.clips_failed.fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    total_failed = self.metrics.clips_failed.load(Ordering::Relaxed),
                    "Snippet writer task is dead — channel closed. Restart required to recover.",
                );
            }
        }
    }
}

/// Spawn the background snippet writer task. Returns a [`SnippetWriter`] handle.
///
/// Seeds the metric atomics from the `lifetime_metrics` table so counters
/// survive restarts, and starts a periodic flush task that mirrors them
/// back every 30 seconds plus once on shutdown.
pub async fn spawn_snippet_writer(
    config: SnippetConfig,
    db: Database,
    shutdown: CancellationToken,
) -> SnippetWriter {
    let (tx, rx) = mpsc::channel::<SnippetJob>(64);

    // Load lifetime metrics so the atomics start where we left off rather
    // than at zero. Failures are downgraded to a warning — running with
    // fresh counters is degraded but functional.
    let initial = db.load_lifetime_metrics().await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Failed to load lifetime metrics; starting at 0");
        Default::default()
    });
    let metrics = Arc::new(SnippetMetrics {
        clips_saved: AtomicU64::new(initial.get("clips_saved").copied().unwrap_or(0)),
        clips_dropped: AtomicU64::new(initial.get("clips_dropped").copied().unwrap_or(0)),
        clips_failed: AtomicU64::new(initial.get("clips_failed").copied().unwrap_or(0)),
        bytes_written: AtomicU64::new(initial.get("bytes_written").copied().unwrap_or(0)),
        last_clip_saved_ms: AtomicI64::new(initial.get("last_clip_saved_ms").copied().unwrap_or(0) as i64),
        last_retention_ms: AtomicI64::new(initial.get("last_retention_ms").copied().unwrap_or(0) as i64),
        last_retention_evicted: AtomicU64::new(initial.get("last_retention_evicted").copied().unwrap_or(0)),
        last_disk_size_bytes: AtomicU64::new(initial.get("last_disk_size_bytes").copied().unwrap_or(0)),
    });

    tokio::spawn(writer_loop(config, db.clone(), rx, shutdown.clone(), metrics.clone()));
    tokio::spawn(metrics_flush_loop(db, shutdown, metrics.clone()));

    SnippetWriter { tx, metrics }
}

/// Mirror the in-memory metric atomics back to `lifetime_metrics` every
/// 30 seconds, and once more on shutdown. Reads are best-effort: a flush
/// failure logs a warning and the loop continues with the next tick.
async fn metrics_flush_loop(
    db: Database,
    shutdown: CancellationToken,
    metrics: Arc<SnippetMetrics>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    interval.tick().await; // skip immediate first tick
    loop {
        tokio::select! {
            _ = interval.tick() => { flush_metrics(&db, &metrics).await; }
            () = shutdown.cancelled() => {
                flush_metrics(&db, &metrics).await;
                tracing::info!("Snippet metrics flushed on shutdown");
                break;
            }
        }
    }
}

async fn flush_metrics(db: &Database, metrics: &SnippetMetrics) {
    // Negative timestamps shouldn't happen but clamp to 0 for the u64
    // store column. The deserialize side already does .max(0).
    let last_clip_saved = metrics.last_clip_saved_ms.load(Ordering::Relaxed).max(0) as u64;
    let last_retention = metrics.last_retention_ms.load(Ordering::Relaxed).max(0) as u64;
    let pairs = [
        ("clips_saved", metrics.clips_saved.load(Ordering::Relaxed)),
        ("clips_dropped", metrics.clips_dropped.load(Ordering::Relaxed)),
        ("clips_failed", metrics.clips_failed.load(Ordering::Relaxed)),
        ("bytes_written", metrics.bytes_written.load(Ordering::Relaxed)),
        ("last_clip_saved_ms", last_clip_saved),
        ("last_retention_ms", last_retention),
        ("last_retention_evicted", metrics.last_retention_evicted.load(Ordering::Relaxed)),
        ("last_disk_size_bytes", metrics.last_disk_size_bytes.load(Ordering::Relaxed)),
    ];
    for (key, value) in pairs {
        if let Err(e) = db.set_lifetime_metric(key, value).await {
            tracing::warn!(key, error = %e, "Failed to persist lifetime metric");
        }
    }
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
                    metrics.clips_failed.fetch_add(1, Ordering::Relaxed);
                    tracing::error!(
                        detection_id = %job.detection_id,
                        error = %e,
                        total_failed = metrics.clips_failed.load(Ordering::Relaxed),
                        "Failed to save audio clip"
                    );
                }
            }
            () = shutdown.cancelled() => {
                // Drain remaining jobs before exiting.
                while let Ok(job) = rx.try_recv() {
                    if let Err(e) = process_job(&clip_dir, &db, &job, &metrics).await {
                        metrics.clips_failed.fetch_add(1, Ordering::Relaxed);
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
    // The "last successful clip save" timestamp is the cheapest live
    // signal of "writer is healthy right now" — the diagnostics page uses
    // it to flag a writer that's silent for too long.
    metrics
        .last_clip_saved_ms
        .store(Utc::now().timestamp_millis(), Ordering::Relaxed);

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
    metrics: Arc<SnippetMetrics>,
) {
    tokio::spawn(retention_loop(config, db, shutdown, metrics));
}

async fn retention_loop(
    config: SnippetConfig,
    db: Database,
    shutdown: CancellationToken,
    metrics: Arc<SnippetMetrics>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    interval.tick().await; // first tick is immediate — skip it
    let clip_dir = PathBuf::from(&config.clip_dir);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                match run_retention(&config, &clip_dir, &db).await {
                    Ok(deleted) => {
                        // Record this run in the metrics so the
                        // diagnostics page can show "last sweep" details
                        // without scraping logs.
                        metrics.last_retention_ms.store(Utc::now().timestamp_millis(), Ordering::Relaxed);
                        metrics.last_retention_evicted.store(deleted, Ordering::Relaxed);
                        let size = dir_size(&clip_dir).await;
                        metrics.last_disk_size_bytes.store(size, Ordering::Relaxed);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Retention cleanup failed");
                    }
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
/// policy needs (rarity tier, review status, species, confidence). Built once
/// per sweep. `confidence` powers the per-(species, day) quota's "top M by
/// confidence" preservation rule.
struct RetentionCandidate {
    id: Vec<u8>,
    snippet_path: String,
    detected_at: i64,
    confidence: f64,
    scientific_name: String,
    rarity: Option<RarityRow>,
    reviewed_correct: bool,
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// UTC calendar day for a Unix-ms timestamp. Floor-divide so negative
/// timestamps (used in unit tests, where `now_ms = 0` and historical clips
/// have negative `detected_at`) bucket correctly without straddling 0.
fn utc_day(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS)
}

/// True if the clip is in a "rare" tier that's exempt from the per-day
/// quota. `first_day` and `high_score` are *not* exempt — their natural
/// protection is being among the top-recent or top-confidence clips for
/// their (species, day) bucket. `first_ever` / `first_season` / `first_week`
/// are genuinely uncommon and shouldn't be touched by quota trimming.
fn quota_exempt_rarity(rarity: Option<&RarityRow>) -> bool {
    rarity
        .map(|r| r.first_ever || r.first_season || r.first_week)
        .unwrap_or(false)
}

/// Pick candidates whose age exceeds their tier-adjusted cutoff. Reviewed-correct
/// rows are exempt. Returns indices into `candidates`.
fn select_age_evictions(
    candidates: &[RetentionCandidate],
    config: &SnippetConfig,
    now_ms: i64,
) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        if c.reviewed_correct {
            continue;
        }
        let mul = effective_multiplier(c.rarity.as_ref(), config);
        let span_days = i64::from(config.retention_days).saturating_mul(i64::from(mul));
        let cutoff = now_ms.saturating_sub(span_days.saturating_mul(DAY_MS));
        if c.detected_at < cutoff {
            out.push(i);
        }
    }
    out
}

/// Per-(species, UTC-day) quota sweep. For each bucket, the keep set is the
/// union of "top `recent_n` by detected_at desc" and "top `top_conf_m` by
/// confidence desc"; everything else is returned for eviction. Reviewed
/// correct and rare-tier clips (`first_ever`/`first_season`/`first_week`)
/// are exempt and never appear in the result.
///
/// The bucket key is `(scientific_name, utc_day)`, so today's phoebes are
/// scored independently from yesterday's phoebes — a noisy day can never
/// trim a representative clip from a different day.
///
/// `recent_n + top_conf_m == 0` short-circuits the sweep (caller should
/// gate on this; we still return `Vec::new()` to be safe).
fn select_quota_evictions(
    candidates: &[RetentionCandidate],
    recent_n: usize,
    top_conf_m: usize,
) -> Vec<usize> {
    if recent_n == 0 && top_conf_m == 0 {
        return Vec::new();
    }

    let mut buckets: HashMap<(&str, i64), Vec<usize>> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        if c.reviewed_correct {
            continue;
        }
        if quota_exempt_rarity(c.rarity.as_ref()) {
            continue;
        }
        buckets
            .entry((c.scientific_name.as_str(), utc_day(c.detected_at)))
            .or_default()
            .push(i);
    }

    let mut evictions = Vec::new();
    for (_, indices) in buckets {
        // Skip buckets that are already at or under the conservative
        // upper bound: a bucket of size <= max(recent_n, top_conf_m)
        // can't possibly have anything to evict, since either sub-set
        // alone would already cover the whole bucket.
        if indices.len() <= recent_n.max(top_conf_m) {
            continue;
        }

        // "Most recent N": sort by detected_at desc, take first N.
        let mut by_recency = indices.clone();
        by_recency.sort_by(|&a, &b| candidates[b].detected_at.cmp(&candidates[a].detected_at));
        let recent_keep: std::collections::HashSet<usize> =
            by_recency.iter().take(recent_n).copied().collect();

        // "Top M by confidence": sort by confidence desc, take first M.
        // total_cmp handles NaN deterministically (NaN sinks to the end).
        let mut by_confidence = indices.clone();
        by_confidence.sort_by(|&a, &b| {
            candidates[b]
                .confidence
                .total_cmp(&candidates[a].confidence)
        });
        let conf_keep: std::collections::HashSet<usize> =
            by_confidence.iter().take(top_conf_m).copied().collect();

        for i in indices {
            if !recent_keep.contains(&i) && !conf_keep.contains(&i) {
                evictions.push(i);
            }
        }
    }
    evictions
}

/// Order candidates for size-sweep eviction: least-protected tier first, then
/// oldest within tier. Reviewed-correct rows are kept in the order so the size
/// sweep can skip them inline (the actual skip happens at eviction time so
/// disk-free accounting stays accurate against partial metadata).
fn size_sweep_order(candidates: &[RetentionCandidate]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|&i| {
        let c = &candidates[i];
        (tier(c.rarity.as_ref()), c.detected_at)
    });
    order
}

async fn run_retention(
    config: &SnippetConfig,
    clip_dir: &Path,
    db: &Database,
) -> anyhow::Result<u64> {
    let mut deleted = 0u64;

    // ── Build the candidate set with everything the policy needs.
    // retention_candidates folds rarity + reviewed_correct + confidence into
    // a single SELECT, so the worker no longer does N+1 round-trips on the
    // SD card.
    let rows = db.retention_candidates(10_000).await?;
    let mut candidates: Vec<RetentionCandidate> = rows
        .into_iter()
        .map(|r| RetentionCandidate {
            id: r.id,
            snippet_path: r.snippet_path,
            detected_at: r.detected_at,
            confidence: r.confidence,
            scientific_name: r.scientific_name,
            rarity: r.rarity,
            reviewed_correct: r.reviewed_correct,
        })
        .collect();

    let now_ms = Utc::now().timestamp_millis();

    // ── Age sweep: each row gets its own tier-adjusted cutoff.
    if config.retention_days > 0 {
        let mut to_evict = select_age_evictions(&candidates, config, now_ms);
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

    // ── Per-(species, UTC-day) quota: trim noisy species back to a small
    // set of representative clips per day. Today's phoebes survive on the
    // strength of being among the day's top-N most recent or top-M highest
    // confidence; everything else above the quota is eligible for eviction.
    // Rare-tier and reviewed-correct clips are exempt (handled inside
    // select_quota_evictions).
    let recent_n = config.per_species_per_day_recent as usize;
    let top_conf_m = config.per_species_per_day_top_confidence as usize;
    if recent_n > 0 || top_conf_m > 0 {
        let mut to_evict = select_quota_evictions(&candidates, recent_n, top_conf_m);
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
            let order = size_sweep_order(&candidates);
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

    Ok(deleted)
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
            per_species_per_day_recent: 10,
            per_species_per_day_top_confidence: 10,
        }
    }

    fn candidate(species: &str, age_days: i64, rarity: Option<RarityRow>, reviewed_correct: bool) -> RetentionCandidate {
        candidate_with(species, age_days, rarity, reviewed_correct, 0.5)
    }

    /// Build a candidate with explicit confidence. `age_days` is days before
    /// the test "now" (i64::MAX); using a large positive offset keeps the
    /// candidate's UTC day stable regardless of the wall-clock at test time.
    fn candidate_with(
        species: &str,
        age_days: i64,
        rarity: Option<RarityRow>,
        reviewed_correct: bool,
        confidence: f64,
    ) -> RetentionCandidate {
        RetentionCandidate {
            id: vec![],
            snippet_path: format!("{species}/{age_days}.wav"),
            // Negative = "this many days before now=0". Tests that care
            // about the bucket key (utc_day) use age_days that map to
            // distinct days; tests that don't care leave them at 0.
            detected_at: -age_days * DAY_MS,
            confidence,
            scientific_name: species.to_string(),
            rarity,
            reviewed_correct,
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

    // ── Age sweep ─────────────────────────────────────────────────

    #[test]
    fn age_sweep_evicts_only_past_cutoff() {
        let c = cfg(); // retention_days = 30
        let candidates = vec![
            candidate("RWBL", 10, None, false),  // young — keep
            candidate("RWBL", 31, None, false),  // past 30 days — evict
            candidate("RWBL", 60, None, false),  // way past — evict
        ];
        let evicted = select_age_evictions(&candidates, &c, 0);
        assert_eq!(evicted, vec![1, 2]);
    }

    #[test]
    fn age_sweep_skips_reviewed_correct_even_when_ancient() {
        let c = cfg();
        let candidates = vec![
            candidate("RWBL", 365, None, true),  // year-old but pinned — keep
            candidate("RWBL", 31, None, false),  // ordinary aged-out — evict
        ];
        let evicted = select_age_evictions(&candidates, &c, 0);
        assert_eq!(evicted, vec![1]);
    }

    #[test]
    fn age_sweep_extends_first_ever_via_multiplier() {
        let c = cfg(); // first_ever_multiplier = 999
        let candidates = vec![
            // 1000 days × no rarity = past 30-day cutoff → evict
            candidate("RWBL", 1000, None, false),
            // 1000 days × first_ever (999×30 = 29970) → keep
            candidate("BAOW", 1000, Some(rr(true, false, false, false, 0.9)), false),
            // 100 days × first_day (2×30 = 60) → still past cutoff, evict
            candidate("PHOE", 100, Some(rr(false, false, false, true, 0.0)), false),
        ];
        let evicted = select_age_evictions(&candidates, &c, 0);
        assert_eq!(evicted, vec![0, 2]);
    }

    #[test]
    fn age_sweep_uses_strictest_multiplier_for_multi_flag_rows() {
        let c = cfg();
        // first_season=8 + first_day=2 + score=0.7 → strictest is 8 (first_season)
        // 200 days × 8 × 30 = 240 effective span → keep at 200 days
        let row = rr(false, true, false, true, 0.7);
        let candidates = vec![candidate("RWBL", 200, Some(row), false)];
        let evicted = select_age_evictions(&candidates, &c, 0);
        assert!(evicted.is_empty());
    }

    #[test]
    fn age_sweep_respects_disabled_retention() {
        // retention_days = 0 means "no age sweep". The helper computes
        // span_days = 0 × multiplier = 0, so cutoff = now, and any row
        // with detected_at < now evicts. The caller (run_retention) gates
        // the sweep with `if config.retention_days > 0` — this test
        // documents the gate's role.
        let mut c = cfg();
        c.retention_days = 0;
        let candidates = vec![candidate("RWBL", 10, None, false)];
        let evicted = select_age_evictions(&candidates, &c, 0);
        // Without the caller-side gate, ancient rows would all be picked.
        // The helper returns them; the gate prevents the sweep entirely.
        assert_eq!(evicted, vec![0]);
    }

    // ── Per-(species, day) quota ──────────────────────────────────

    /// Build N candidates for the same species on the same UTC day, each
    /// with a distinct `detected_at` (1 minute apart, going backwards from
    /// `base_age_days`) and the supplied `confidences[]`. Index 0 is the
    /// most recent.
    ///
    /// Anchors at noon of the target UTC day so the minute-offset clips
    /// don't spill across midnight into the previous day's bucket. (Without
    /// the noon offset, anchoring at `-N*DAY_MS` puts the base at UTC
    /// midnight, and `div_euclid` on negative timestamps puts the second
    /// clip onto UTC day `N+1`.)
    fn same_day_bucket(species: &str, base_age_days: i64, confidences: &[f64]) -> Vec<RetentionCandidate> {
        let base = -base_age_days * DAY_MS + 12 * 60 * 60 * 1000;
        confidences
            .iter()
            .enumerate()
            .map(|(i, &conf)| RetentionCandidate {
                id: vec![],
                snippet_path: format!("{species}/{base_age_days}-{i}.wav"),
                // 60_000 ms = 1 minute. With the noon anchor and ≤ 720
                // minutes of offsets, all clips stay inside the same UTC day.
                detected_at: base - (i as i64) * 60_000,
                confidence: conf,
                scientific_name: species.to_string(),
                rarity: None,
                reviewed_correct: false,
            })
            .collect()
    }

    #[test]
    fn quota_keeps_top_recent_and_top_confidence_union() {
        // 30 phoebe clips on the same day with monotonically decreasing
        // confidence (index 0 has the highest confidence AND is the most
        // recent). With recent_n=10, top_conf_m=10, the keep set is the
        // first 10 indices (both criteria pick the same set), so the
        // remaining 20 evict.
        let confidences: Vec<f64> = (0..30).map(|i| 1.0 - (i as f64) * 0.01).collect();
        let candidates = same_day_bucket("Sayornis phoebe", 5, &confidences);
        let mut evicted = select_quota_evictions(&candidates, 10, 10);
        evicted.sort_unstable();
        let expected: Vec<usize> = (10..30).collect();
        assert_eq!(evicted, expected);
    }

    #[test]
    fn quota_union_dedupes_when_recent_and_confidence_disagree() {
        // 30 phoebe clips on the same day. Confidence is HIGHEST for the
        // OLDEST clips (index 29 has confidence 1.0; index 0 has 0.71).
        // recent_n=10 → keep indices 0..10 (most recent). top_conf_m=10 →
        // keep indices 20..30 (highest confidence). Union: 20 clips kept,
        // the middle 10 (indices 10..20) evict.
        let confidences: Vec<f64> = (0..30).map(|i| 0.7 + (i as f64) * 0.01).collect();
        let candidates = same_day_bucket("Sayornis phoebe", 5, &confidences);
        let mut evicted = select_quota_evictions(&candidates, 10, 10);
        evicted.sort_unstable();
        let expected: Vec<usize> = (10..20).collect();
        assert_eq!(evicted, expected);
    }

    #[test]
    fn quota_keeps_all_when_bucket_is_small() {
        // 5 clips on the same day, quota is 10+10. Bucket size <= max(N,M),
        // so nothing evicts — the user is never disappointed when they
        // browse a low-volume species.
        let confidences = vec![0.9, 0.8, 0.7, 0.6, 0.5];
        let candidates = same_day_bucket("Hylocichla mustelina", 5, &confidences);
        let evicted = select_quota_evictions(&candidates, 10, 10);
        assert!(evicted.is_empty());
    }

    #[test]
    fn quota_buckets_by_species_and_day_independently() {
        // PHOE day 5: 12 clips, 2 should evict (keep top 10 recency, all
        // are within top 10 confidence too since confidences are flat).
        // PHOE day 6: 5 clips, all kept.
        // RWBL day 5: 12 clips, 2 should evict.
        let mut candidates = same_day_bucket("Sayornis phoebe", 5, &[0.5; 12]);
        let phoe_day_5_len = candidates.len();
        candidates.extend(same_day_bucket("Sayornis phoebe", 6, &[0.5; 5]));
        candidates.extend(same_day_bucket("Agelaius phoeniceus", 5, &[0.5; 12]));
        let evicted = select_quota_evictions(&candidates, 10, 10);

        // Each over-quota bucket evicts its 2 lowest-rank clips.
        // phoe day 5: indices 0..12, oldest two evict (10, 11).
        // phoe day 6: indices 12..17, all kept.
        // rwbl day 5: indices 17..29, oldest two evict (27, 28).
        let mut got = evicted.clone();
        got.sort_unstable();
        assert_eq!(got, vec![10, 11, 27, 28]);
        // And we used phoe_day_5_len just to anchor the offsets:
        assert_eq!(phoe_day_5_len, 12);
    }

    #[test]
    fn quota_exempts_reviewed_correct() {
        // 30 same-day phoebes, but 5 are reviewed-correct. The exempt rows
        // never appear in the eviction list AND don't count toward the
        // bucket size. So among the 25 non-exempt rows, the keep set is
        // the 10 most recent ∪ 10 most confident (here aligned: indices
        // 0-9 of the non-exempt set), and the rest evict.
        let mut candidates = same_day_bucket("Sayornis phoebe", 5, &[0.5; 30]);
        // Pin every 6th row as reviewed-correct. Non-exempt indices: all
        // except {0, 6, 12, 18, 24}.
        for i in [0, 6, 12, 18, 24] {
            candidates[i].reviewed_correct = true;
        }
        let evicted = select_quota_evictions(&candidates, 10, 10);
        // Exempt indices must never appear in the result.
        for pinned in [0, 6, 12, 18, 24] {
            assert!(!evicted.contains(&pinned), "reviewed_correct {pinned} was evicted");
        }
        // 25 non-exempt rows in the bucket; keep_recent ∪ keep_conf both
        // pick the same 10 (since confidences are flat — sort is stable
        // by detected_at descending → the 10 newest non-exempt indices).
        // 25 - 10 = 15 evictions.
        assert_eq!(evicted.len(), 15);
    }

    #[test]
    fn quota_exempts_first_ever_first_season_first_week() {
        // 12 same-day phoebes; one each is first_ever, first_season,
        // first_week. With recent_n=10, top_conf_m=10, only the 9 plain
        // rows feed the bucket. Bucket size 9 ≤ 10, nothing evicts.
        let mut candidates = same_day_bucket("Sayornis phoebe", 5, &[0.5; 12]);
        candidates[0].rarity = Some(rr(true, false, false, false, 0.9));   // first_ever
        candidates[1].rarity = Some(rr(false, true, false, false, 0.0));   // first_season
        candidates[2].rarity = Some(rr(false, false, true, false, 0.0));   // first_week
        let evicted = select_quota_evictions(&candidates, 10, 10);
        assert!(evicted.is_empty());
    }

    #[test]
    fn quota_does_not_exempt_first_day_or_high_score() {
        // 12 same-day phoebes, indices 0-1 are first_day and high_score
        // respectively. Both are NOT quota-exempt — they ride along by
        // being among the most recent. Confidence is flat, so the keep
        // set is the 10 most recent (indices 0..10). Indices 10, 11
        // evict, including no quota-exempt rows.
        let mut candidates = same_day_bucket("Sayornis phoebe", 5, &[0.5; 12]);
        candidates[0].rarity = Some(rr(false, false, false, true, 0.0));   // first_day, NOT exempt
        candidates[1].rarity = Some(rr(false, false, false, false, 0.85)); // high_score, NOT exempt
        let mut evicted = select_quota_evictions(&candidates, 10, 10);
        evicted.sort_unstable();
        assert_eq!(evicted, vec![10, 11]);
    }

    #[test]
    fn quota_disabled_when_both_knobs_zero() {
        let candidates = same_day_bucket("Sayornis phoebe", 5, &[0.5; 50]);
        assert!(select_quota_evictions(&candidates, 0, 0).is_empty());
    }

    #[test]
    fn quota_with_only_recency_knob() {
        // recent_n=5, top_conf_m=0 → keep only the 5 most recent.
        let candidates = same_day_bucket("Sayornis phoebe", 5, &[0.5; 10]);
        let mut evicted = select_quota_evictions(&candidates, 5, 0);
        evicted.sort_unstable();
        assert_eq!(evicted, vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn quota_with_only_confidence_knob() {
        // recent_n=0, top_conf_m=5 → keep only the 5 highest confidence.
        // Confidences ascending with index, so the keep set is indices 5-9.
        let confidences: Vec<f64> = (0..10).map(|i| 0.5 + (i as f64) * 0.01).collect();
        let candidates = same_day_bucket("Sayornis phoebe", 5, &confidences);
        let mut evicted = select_quota_evictions(&candidates, 0, 5);
        evicted.sort_unstable();
        assert_eq!(evicted, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn quota_today_is_protected_in_the_same_way_as_old_days() {
        // The quota applies uniformly to every UTC day, including today.
        // 50 phoebes today (age_days=0) → quota trims to ~20.
        let confidences: Vec<f64> = (0..50).map(|i| 1.0 - (i as f64) * 0.01).collect();
        let candidates = same_day_bucket("Sayornis phoebe", 0, &confidences);
        let evicted = select_quota_evictions(&candidates, 10, 10);
        // 50 - 10 (recency aligned with confidence here) = 40 evict.
        assert_eq!(evicted.len(), 40);
        // The newest 10 always survive — the user always has audio when
        // browsing today's phoebes.
        for i in 0..10 {
            assert!(!evicted.contains(&i), "newest clip {i} was evicted");
        }
    }

    // ── Size sweep order ──────────────────────────────────────────

    #[test]
    fn size_sweep_orders_by_tier_then_age() {
        let candidates = vec![
            // Tier 0 (common): index 0 oldest, index 1 newest
            candidate("RWBL", 30, None, false),  // 0 — oldest tier-0
            candidate("RWBL", 10, None, false),  // 1 — newest tier-0
            // Tier 1 (first_day)
            candidate("PHOE", 50, Some(rr(false, false, false, true, 0.0)), false), // 2
            // Tier 2 (first_week)
            candidate("BAOW", 100, Some(rr(false, false, true, false, 0.0)), false), // 3
            // Tier 3 (first_ever)
            candidate("WTSP", 200, Some(rr(true, false, false, false, 0.9)), false), // 4
        ];
        // Expected: oldest tier-0, then newest tier-0, then tier-1, tier-2, tier-3.
        let order = size_sweep_order(&candidates);
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn size_sweep_within_tier_oldest_first() {
        let candidates = vec![
            candidate("RWBL", 5, None, false),   // 0 — newest
            candidate("RWBL", 50, None, false),  // 1 — oldest
            candidate("RWBL", 25, None, false),  // 2 — middle
        ];
        let order = size_sweep_order(&candidates);
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn size_sweep_keeps_reviewed_correct_in_order() {
        // The size sweep skips reviewed_correct inline at eviction time so
        // disk accounting can use partial metadata. The order helper itself
        // is purely positional — it doesn't filter.
        let candidates = vec![
            candidate("RWBL", 30, None, false),  // 0 — oldest tier-0, evictable
            candidate("RWBL", 10, None, true),   // 1 — pinned, but still in order
            candidate("BAOW", 100, Some(rr(true, false, false, false, 0.9)), false), // 2 — first_ever
        ];
        let order = size_sweep_order(&candidates);
        assert_eq!(order, vec![0, 1, 2]); // tier 0,0,3 with index 0 older than 1
    }
}

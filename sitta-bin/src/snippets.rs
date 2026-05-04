//! Asynchronous audio snippet writer and retention worker.
//!
//! The snippet writer receives [`SnippetJob`]s from the inference pipeline via
//! a bounded channel and writes WAV files to disk in a background task. Writes
//! are performed inside [`tokio::task::spawn_blocking`] to avoid blocking the
//! async runtime on slow I/O (SD cards).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    let pairs = [
        ("clips_saved", metrics.clips_saved.load(Ordering::Relaxed)),
        ("clips_dropped", metrics.clips_dropped.load(Ordering::Relaxed)),
        ("clips_failed", metrics.clips_failed.load(Ordering::Relaxed)),
        ("bytes_written", metrics.bytes_written.load(Ordering::Relaxed)),
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

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

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

/// Pick the oldest excess clips for any species exceeding `cap`. Reviewed-correct
/// and first_ever rows don't count toward the cap and are never returned. Caller
/// must check that the cap is enabled before invoking.
fn select_species_cap_evictions(
    candidates: &[RetentionCandidate],
    cap: usize,
) -> Vec<usize> {
    let mut by_species: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        if c.reviewed_correct {
            continue;
        }
        if c.rarity.as_ref().is_some_and(|r| r.first_ever) {
            continue;
        }
        by_species
            .entry(c.scientific_name.as_str())
            .or_default()
            .push(i);
    }
    let mut out = Vec::new();
    for (_, mut indices) in by_species {
        if indices.len() <= cap {
            continue;
        }
        indices.sort_by_key(|&i| candidates[i].detected_at);
        let excess = indices.len() - cap;
        out.extend(indices.into_iter().take(excess));
    }
    out
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
) -> anyhow::Result<()> {
    let mut deleted = 0u64;

    // ── Build the candidate set with everything the policy needs.
    // retention_candidates folds rarity + reviewed_correct into a single SELECT,
    // so the worker no longer does N+1 round-trips on the SD card.
    let rows = db.retention_candidates(10_000).await?;
    let mut candidates: Vec<RetentionCandidate> = rows
        .into_iter()
        .map(|r| RetentionCandidate {
            id: r.id,
            snippet_path: r.snippet_path,
            detected_at: r.detected_at,
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

    // ── Per-species cap pre-pass: when the cap is set and disk pressure is
    // a concern, trim noisy species back to the cap before global eviction
    // touches anything else.
    if config.per_species_cap > 0 && config.max_disk_mb > 0 {
        let cap = config.per_species_cap as usize;
        let mut to_evict = select_species_cap_evictions(&candidates, cap);
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

    fn candidate(species: &str, age_days: i64, rarity: Option<RarityRow>, reviewed_correct: bool) -> RetentionCandidate {
        RetentionCandidate {
            id: vec![],
            snippet_path: format!("{species}/{age_days}.wav"),
            detected_at: -age_days * DAY_MS, // negative = "this many days before now=0"
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

    // ── Per-species cap ───────────────────────────────────────────

    #[test]
    fn species_cap_evicts_oldest_excess_per_species() {
        let candidates = vec![
            candidate("RWBL", 10, None, false), // 0
            candidate("RWBL", 20, None, false), // 1
            candidate("RWBL", 30, None, false), // 2 — oldest, evict
            candidate("RWBL", 40, None, false), // 3 — older,  evict
            candidate("BAOW", 50, None, false), // 4 — only one, keep
        ];
        let mut evicted = select_species_cap_evictions(&candidates, 2);
        evicted.sort();
        assert_eq!(evicted, vec![2, 3]);
    }

    #[test]
    fn species_cap_exempts_first_ever() {
        let candidates = vec![
            candidate("RWBL", 10, None, false),                                          // 0
            candidate("RWBL", 20, None, false),                                          // 1
            candidate("RWBL", 30, Some(rr(true, false, false, false, 0.9)), false),      // 2 — first_ever, exempt
            candidate("RWBL", 40, None, false),                                          // 3 — oldest non-exempt, evict
        ];
        // cap = 2: 3 non-exempt rows → 1 excess. The oldest non-exempt evicts.
        let evicted = select_species_cap_evictions(&candidates, 2);
        assert_eq!(evicted, vec![3]);
    }

    #[test]
    fn species_cap_exempts_reviewed_correct() {
        let candidates = vec![
            candidate("RWBL", 10, None, false),  // 0
            candidate("RWBL", 20, None, false),  // 1
            candidate("RWBL", 30, None, true),   // 2 — pinned, exempt
            candidate("RWBL", 40, None, false),  // 3 — oldest non-exempt, evict
        ];
        let evicted = select_species_cap_evictions(&candidates, 2);
        assert_eq!(evicted, vec![3]);
    }

    #[test]
    fn species_cap_independent_per_species() {
        let candidates = vec![
            candidate("RWBL", 10, None, false), // 0
            candidate("RWBL", 20, None, false), // 1
            candidate("RWBL", 30, None, false), // 2 — RWBL excess, evict
            candidate("BAOW", 5, None, false),  // 3
            candidate("BAOW", 8, None, false),  // 4 — BAOW within cap
        ];
        let evicted = select_species_cap_evictions(&candidates, 2);
        assert_eq!(evicted, vec![2]);
    }

    #[test]
    fn species_cap_no_op_when_under_cap() {
        let candidates = vec![
            candidate("RWBL", 10, None, false),
            candidate("BAOW", 20, None, false),
        ];
        let evicted = select_species_cap_evictions(&candidates, 5);
        assert!(evicted.is_empty());
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

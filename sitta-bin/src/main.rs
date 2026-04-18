mod config;
mod consumers;
mod models;
mod persist;
mod seed;
mod snippets;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use sitta_api::server::{self, ApiState, PipelineMetrics};
use sitta_api::settings::{InitialConfig, RuntimeSettings};
use sitta_audio::chunk::AudioChunk;
use sitta_audio::manager::SourceManager;
use sitta_inference::rangefilter::RangeFilter;
use sitta_store::db::Database;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("sitta=info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".into());
    let config_str = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("failed to read config file: {config_path}"))?;
    let config: Config =
        toml::from_str(&config_str).with_context(|| "failed to parse config file")?;

    tracing::info!(
        station_id = %config.station.id,
        station_name = %config.station.name,
        lat = config.station.latitude,
        lon = config.station.longitude,
        sources = config.audio.sources.len(),
        chunk_seconds = config.audio.chunk_seconds,
        "Starting Sitta"
    );

    // ── Model loading ───────────────────────────────────────────
    let taxonomy = models::load_taxonomy(&config)?;
    let (birdnet_classifier, range_filter) = models::load_birdnet(&config, taxonomy.clone())?;

    let mut classifiers: Vec<Arc<dyn sitta_inference::model::Classifier>> = Vec::new();
    if let Some(c) = birdnet_classifier {
        classifiers.push(c);
    }
    if classifiers.is_empty() {
        tracing::warn!("No inference models configured -- running in audio-only mode");
    }
    let classifiers: Arc<[Arc<dyn sitta_inference::model::Classifier>]> = classifiers.into();
    let range_filter: Option<Arc<RangeFilter>> = range_filter.map(Arc::new);

    let perch_model = models::load_perch(&config, taxonomy.clone())?;

    // ── Database setup ──────────────────────────────────────────
    let db = Database::open(Path::new(&config.store.path))
        .await
        .context("failed to open database")?;
    tracing::info!(path = %config.store.path, "Database opened");

    // ── Runtime settings ──────────────────────────────────────────
    use sitta_api::settings::{round4, timezone_from_coords};

    let lat = config.station.latitude.map(|v| round4(f64::from(v)));
    let lon = config.station.longitude.map(|v| round4(f64::from(v)));
    let timezone = config
        .station
        .timezone
        .clone()
        .unwrap_or_else(|| match (lat, lon) {
            (Some(la), Some(lo)) => timezone_from_coords(la, lo),
            _ => "UTC".to_string(),
        });

    let runtime_settings = RuntimeSettings {
        station_name: config.station.name.clone(),
        station_latitude: lat,
        station_longitude: lon,
        timezone,
        display_min_confidence: config.api.display_min_confidence,
        birdnet_min_confidence: config.inference.birdnet.as_ref().map(|b| b.min_confidence),
        birdnet_top_k: config.inference.birdnet.as_ref().map(|b| b.top_k),
        birdnet_meta_threshold: config.inference.birdnet.as_ref().map(|b| b.meta_threshold),
        birdnet_force_allow: config.inference.birdnet.as_ref().map(|b| b.force_allow.clone()),
        perch_min_confidence: config.inference.perch.as_ref().map(|p| p.min_confidence),
        perch_top_k: config.inference.perch.as_ref().map(|p| p.top_k),
    };
    let settings = Arc::new(ArcSwap::from_pointee(runtime_settings));

    let mut persist_ctx = seed::seed_database(
        &db,
        &config,
        &classifiers,
        perch_model.as_ref(),
        taxonomy.as_deref(),
        settings.clone(),
    )
    .await
    .context("failed to seed database")?;

    // ── Snippet writer ──────────────────────────────────────────
    let shutdown = CancellationToken::new();

    if config.snippets.enabled {
        let writer = snippets::spawn_snippet_writer(
            config.snippets.clone(),
            db.clone(),
            shutdown.clone(),
        );
        persist_ctx.snippet_writer = Some(writer);
        snippets::spawn_retention_worker(
            config.snippets.clone(),
            db.clone(),
            shutdown.clone(),
        );
        tracing::info!(
            clip_dir = %config.snippets.clip_dir,
            retention_days = config.snippets.retention_days,
            max_disk_mb = config.snippets.max_disk_mb,
            "Audio snippet saving enabled"
        );
    }

    // ── API server ──────────────────────────────────────────────
    let metrics = Arc::new(PipelineMetrics::default());
    let (settings_notify_tx, _settings_notify_rx) = tokio::sync::watch::channel(());

    let initial_config = Arc::new(InitialConfig {
        station_id: config.station.id.clone(),
        birdnet_model_path: config.inference.birdnet.as_ref().map(|b| b.model_path.clone()),
        birdnet_labels_path: config.inference.birdnet.as_ref().map(|b| b.labels_path.clone()),
        birdnet_meta_model_path: config
            .inference
            .birdnet
            .as_ref()
            .and_then(|b| b.meta_model_path.clone()),
        perch_model_path: config.inference.perch.as_ref().map(|p| p.model_path.clone()),
        perch_labels_path: config.inference.perch.as_ref().map(|p| p.labels_path.clone()),
        store_path: config.store.path.clone(),
        api_bind: config.api.bind.clone(),
    });

    let api_addr: std::net::SocketAddr = config
        .api
        .bind
        .parse()
        .with_context(|| format!("invalid api.bind address: {}", config.api.bind))?;

    // Audio broadcast channel and source manager.
    let (tx, _rx) = broadcast::channel::<Arc<AudioChunk>>(32);
    let source_manager = SourceManager::new(tx.clone(), shutdown.clone(), config.audio.chunk_seconds);
    source_manager.add_initial(&config.audio.sources).await;

    let api_state = ApiState {
        db: db.clone(),
        detection_tx: persist_ctx.detection_tx.clone(),
        settings: settings.clone(),
        settings_notify: Arc::new(settings_notify_tx),
        config_path: std::path::PathBuf::from(&config_path),
        initial_config,
        metrics: metrics.clone(),
        matcher: persist_ctx.matcher.clone(),
        audio_tx: tx.clone(),
        source_manager: source_manager.clone(),
    };
    tokio::spawn(server::serve(api_addr, api_state, shutdown.clone()));

    // ── Inference consumers ─────────────────────────────────────
    if let Some(perch) = perch_model {
        consumers::spawn_perch_consumer(
            perch,
            range_filter.clone(),
            tx.subscribe(),
            shutdown.clone(),
            persist_ctx.clone(),
            metrics.clone(),
        );
    }

    // BirdNET consumer with configurable sliding window.
    let birdnet_stride = config
        .inference
        .birdnet
        .as_ref()
        .map(|b| b.stride_seconds)
        .unwrap_or(config.audio.chunk_seconds as f32);
    let window_samples = (config.audio.chunk_seconds * 48_000) as usize;
    let stride_samples = (birdnet_stride * 48_000.0) as usize;
    tracing::info!(
        window_s = config.audio.chunk_seconds,
        stride_s = birdnet_stride,
        overlap_s = config.audio.chunk_seconds as f32 - birdnet_stride,
        "BirdNET consumer: window={window_samples} stride={stride_samples} samples"
    );
    consumers::spawn_birdnet_consumer(
        classifiers.clone(),
        range_filter.clone(),
        tx.subscribe(),
        shutdown.clone(),
        persist_ctx.clone(),
        metrics.clone(),
        window_samples,
        stride_samples,
    );

    // ── Shutdown ────────────────────────────────────────────────
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for ctrl-c")?;
    tracing::info!("Shutting down...");
    shutdown.cancel();

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    db.close().await;

    Ok(())
}

mod config;
mod consumers;
mod effort;
mod models;
mod mqtt;
mod persist;
mod seed;
mod snippets;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use sitta_api::server::{self, ApiState, PipelineMetrics};
use sitta_api::settings::{InitialConfig, LoadedModelInfo, RuntimeSettings};
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
    // Default filter lists every Sitta crate explicitly. The previous
    // "sitta=info" matched none of our targets — modules in this binary
    // log under `sitta_bin::*` (the lib name), and the workspace crates
    // are `sitta_api`, `sitta_store`, `sitta_audio`, `sitta_inference`,
    // `sitta_taxonomy`, `sitta_spatial`. Without this list, errors from
    // those crates were silently filtered out at the default level.
    const DEFAULT_FILTER: &str = "info,\
        sitta_bin=info,sitta_api=info,sitta_store=info,\
        sitta_audio=info,sitta_inference=info,\
        sitta_taxonomy=warn,sitta_spatial=warn,\
        sqlx=warn,hyper=warn,h2=warn,rustls=warn,tower_http=warn";
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER)),
        )
        .init();

    // Make panics in spawned tokio tasks loud. The default panic hook
    // prints to stderr but doesn't go through tracing, so it can be
    // missed when the operator is filtering by tracing target. We
    // re-emit the panic as a tracing::error and then defer to the old
    // hook so the message + backtrace still reach stderr.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload: &str = info
            .payload()
            .downcast_ref::<&'static str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("(non-string panic payload)");
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        tracing::error!(
            target: "sitta_bin::panic",
            panic_payload = %payload,
            panic_location = %location,
            "Panic in async task — task is dead and will not restart",
        );
        prev_hook(info);
    }));

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
        species_image_url: None,
        display_min_confidence: config.api.display_min_confidence,
        birdnet_min_confidence: config.inference.birdnet.as_ref().map(|b| b.min_confidence),
        birdnet_top_k: config.inference.birdnet.as_ref().map(|b| b.top_k),
        birdnet_meta_threshold: config.inference.birdnet.as_ref().map(|b| b.meta_threshold),
        birdnet_force_allow: config.inference.birdnet.as_ref().map(|b| b.force_allow.clone()),
        perch_min_confidence: config.inference.perch.as_ref().map(|p| p.min_confidence),
        perch_top_k: config.inference.perch.as_ref().map(|p| p.top_k),
        show_range_unverified: config.api.show_range_unverified,
        presence_min_detections: config.presence.min_detections,
        presence_window_minutes: config.presence.window_minutes,
        presence_immediate_threshold: config.presence.immediate_threshold,
        skip_environment_clips: config.api.skip_environment_clips,
        skip_environment_detections: config.api.skip_environment_detections,
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

    // Wire range filter into persist context for rarity scoring.
    persist_ctx.range_filter = range_filter.clone();

    if config.presence.min_detections > 1 {
        tracing::info!(
            min_detections = config.presence.min_detections,
            window_minutes = config.presence.window_minutes,
            immediate_threshold = ?config.presence.immediate_threshold,
            "Presence confirmation enabled"
        );
    }

    // Base URL for detection links in MQTT/SSE events.
    // Prefer explicit config; fall back to the API bind address.
    persist_ctx.api_base_url = Some(
        config.api.base_url.clone().unwrap_or_else(|| {
            let bind = &config.api.bind;
            let host = if bind.starts_with("0.0.0.0") {
                bind.replacen("0.0.0.0", "localhost", 1)
            } else {
                bind.clone()
            };
            format!("http://{host}")
        }),
    );

    // ── Snippet writer ──────────────────────────────────────────
    let shutdown = CancellationToken::new();
    let mut snippet_metrics: Option<Arc<sitta_api::server::SnippetMetrics>> = None;

    if config.snippets.enabled {
        let writer = snippets::spawn_snippet_writer(
            config.snippets.clone(),
            db.clone(),
            shutdown.clone(),
        )
        .await;
        snippet_metrics = Some(writer.metrics.clone());
        persist_ctx.snippet_writer = Some(writer);
        snippets::spawn_retention_worker(
            config.snippets.clone(),
            db.clone(),
            shutdown.clone(),
            snippet_metrics.clone().expect("metrics just set above"),
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

    // Snapshot the loaded inference models for the settings UI.
    let mut loaded_models: Vec<LoadedModelInfo> = Vec::new();
    for c in classifiers.iter() {
        let path = match c.name() {
            n if n.starts_with("BirdNET") => config.inference.birdnet.as_ref().map(|b| b.model_path.clone()),
            n if n.starts_with("Perch") => config.inference.perch.as_ref().map(|p| p.model_path.clone()),
            _ => None,
        };
        let path = path.unwrap_or_default();
        let (size, mtime) = file_stat(&path);
        // Embedding-producing models per birdnet-onnx: Perch v2 always emits
        // 1536-dim, BirdNET v3.0 emits 1024-dim, BirdNET v2.4 / BSG do not.
        let has_embeddings = c.name().starts_with("Perch") || c.name().starts_with("BirdNET v3");
        loaded_models.push(LoadedModelInfo {
            name: c.name().to_string(),
            kind: "classifier",
            model_path: path,
            file_size_bytes: size,
            file_modified_ms: mtime,
            sample_rate: Some(c.sample_rate()),
            window_samples: Some(c.window_samples()),
            has_embeddings: Some(has_embeddings),
        });
    }
    if let Some(p) = perch_model.as_ref() {
        let path = config.inference.perch.as_ref().map(|p| p.model_path.clone()).unwrap_or_default();
        let (size, mtime) = file_stat(&path);
        loaded_models.push(LoadedModelInfo {
            name: p.name().to_string(),
            kind: "classifier",
            model_path: path,
            file_size_bytes: size,
            file_modified_ms: mtime,
            sample_rate: Some(p.sample_rate()),
            window_samples: Some(p.window_samples()),
            has_embeddings: Some(true),
        });
    }
    if let Some(meta_path) = config
        .inference
        .birdnet
        .as_ref()
        .and_then(|b| b.meta_model_path.clone())
    {
        let (size, mtime) = file_stat(&meta_path);
        loaded_models.push(LoadedModelInfo {
            name: "BirdNET range filter (meta-model)".to_string(),
            kind: "meta_model",
            model_path: meta_path,
            file_size_bytes: size,
            file_modified_ms: mtime,
            sample_rate: None,
            window_samples: None,
            has_embeddings: None,
        });
    }

    let initial_config = Arc::new(InitialConfig {
        station_id: config.station.id.clone(),
        mqtt_host: config.mqtt.as_ref().map(|m| m.host.clone()),
        mqtt_port: config.mqtt.as_ref().map(|m| m.port),
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
        loaded_models,
        min_cluster_size: config
            .inference
            .perch
            .as_ref()
            .map(|p| i64::from(p.min_cluster_size))
            .unwrap_or(5),
        min_distinct_days: config
            .inference
            .perch
            .as_ref()
            .map(|p| i64::from(p.min_distinct_days))
            .unwrap_or(2),
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

    // MQTT controller — created before ApiState so it can be shared.
    let mqtt_controller: Arc<mqtt::MqttController> = Arc::new(mqtt::MqttController::new(
        persist_ctx.detection_tx.clone(),
        config.station.id.clone(),
        config.station.name.clone(),
        settings.load().timezone.clone(),
        config.api.display_min_confidence,
        shutdown.clone(),
    ));

    let api_state = ApiState {
        core: sitta_api::server::CoreState {
            db: db.clone(),
            settings: settings.clone(),
            settings_notify: Arc::new(settings_notify_tx),
            config_path: std::path::PathBuf::from(&config_path),
            initial_config,
        },
        audio: sitta_api::server::AudioState {
            audio_tx: tx.clone(),
            source_manager: source_manager.clone(),
        },
        inference: sitta_api::server::InferenceState {
            detection_tx: persist_ctx.detection_tx.clone(),
            matcher: persist_ctx.matcher.clone(),
            metrics: metrics.clone(),
            range_scorer: range_filter.clone().map(|rf| {
                Arc::new(move |name: &str| rf.score_for(name)) as sitta_api::server::RangeScoreFn
            }),
        },
        integrations: sitta_api::server::IntegrationState {
            mqtt_control: Some(mqtt_controller.clone()),
            clip_dir: if config.snippets.enabled {
                Some(std::path::PathBuf::from(&config.snippets.clip_dir))
            } else {
                None
            },
            snippet_metrics: snippet_metrics.clone(),
            snippet_retention: if config.snippets.enabled {
                Some(sitta_api::server::SnippetRetention {
                    retention_days: config.snippets.retention_days,
                    max_disk_mb: config.snippets.max_disk_mb,
                    first_ever_multiplier: config.snippets.first_ever_multiplier,
                    first_season_multiplier: config.snippets.first_season_multiplier,
                    first_week_multiplier: config.snippets.first_week_multiplier,
                    first_day_multiplier: config.snippets.first_day_multiplier,
                    high_score_multiplier: config.snippets.high_score_multiplier,
                    per_species_per_day_recent: config.snippets.per_species_per_day_recent,
                    per_species_per_day_top_confidence: config.snippets.per_species_per_day_top_confidence,
                    low_density_max_days: config.snippets.low_density_max_days,
                    low_density_multiplier: config.snippets.low_density_multiplier,
                })
            } else {
                None
            },
        },
    };
    tokio::spawn(server::serve(api_addr, api_state, shutdown.clone()));

    // ── Effort tracking ────────────────────────────────────────
    // Gap timeout: 2x chunk duration + 5s buffer. If no audio arrives within
    // this window, the session is considered ended (source disconnected).
    let gap_timeout = std::time::Duration::from_secs(
        (config.audio.chunk_seconds as u64) * 2 + 5,
    );
    effort::spawn_effort_tracker(
        db.clone(),
        persist_ctx.source_ids.clone(),
        tx.subscribe(),
        shutdown.clone(),
        gap_timeout,
    );
    tracing::info!(?gap_timeout, "Effort tracker started");

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
        consumers::BirdnetConsumerConfig {
            window_samples,
            stride_samples,
        },
    );

    // ── Background clustering ───────────────────────────────────
    if config.inference.perch.is_some() {
        let cluster_db = db.clone();
        let cluster_shutdown = shutdown.clone();
        let cluster_matcher = persist_ctx.matcher.clone();
        let cluster_config = sitta_store::clustering::ClusterConfig {
            merge_threshold: config
                .inference
                .perch
                .as_ref()
                .map(|p| p.cluster_merge_threshold)
                .unwrap_or(0.70),
            timezone: settings.load().timezone.clone(),
            retention_days: config
                .inference
                .perch
                .as_ref()
                .map(|p| p.candidate_retention_days)
                .unwrap_or(30),
        };
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await; // skip immediate first tick
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        match sitta_store::clustering::run_clustering_pass(&cluster_db, &cluster_config).await {
                            Ok(stats) => {
                                if stats.candidates_processed > 0 {
                                    tracing::info!(
                                        candidates = stats.candidates_processed,
                                        assigned = stats.assigned_to_existing,
                                        new_clusters = stats.new_clusters_created,
                                        pruned = stats.pruned,
                                        "Clustering pass complete"
                                    );
                                }
                            }
                            Err(e) => tracing::error!(error = %e, "Clustering pass failed"),
                        }
                        // Reload matcher in case any clusters were enrolled via API since last pass.
                        if let Some(m) = &cluster_matcher {
                            let _ = m.reload().await;
                        }
                    }
                    () = cluster_shutdown.cancelled() => break,
                }
            }
        });
        tracing::info!("Background clustering task started (5-minute interval)");
    }

    // ── MQTT publisher ──────────────────────────────────────────
    if let Some(ref mqtt_config) = config.mqtt {
        mqtt_controller.start(mqtt_config).await;
    }

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

/// Read (file_size_bytes, file_modified_ms) for a model file. Returns
/// `(None, None)` if the path is empty, missing, or unreadable — the row
/// still renders so the user can see the path that *should* have been there.
fn file_stat(path: &str) -> (Option<u64>, Option<i64>) {
    if path.is_empty() {
        return (None, None);
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return (None, None);
    };
    let size = Some(meta.len());
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());
    (size, mtime)
}

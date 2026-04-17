mod config;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use sitta_audio::chunk::AudioChunk;
use sitta_audio::rtsp::RtspSource;
use sitta_audio::source::SourceConfig;
use sitta_inference::model::Classifier;
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
        sources = config.audio.sources.len(),
        chunk_seconds = config.audio.chunk_seconds,
        "Starting Sitta"
    );

    // Load classifiers.
    let classifiers = load_classifiers(&config)?;
    if classifiers.is_empty() {
        tracing::warn!("No inference models configured -- running in audio-only mode");
    }
    let classifiers: Arc<[Arc<dyn Classifier>]> = classifiers.into();

    let (tx, _rx) = broadcast::channel::<Arc<AudioChunk>>(32);
    let shutdown = CancellationToken::new();

    // Spawn a capture task for each audio source.
    for source_config in &config.audio.sources {
        match source_config {
            SourceConfig::Rtsp(rtsp_config) => {
                let source =
                    RtspSource::new(rtsp_config.clone(), tx.clone(), config.audio.chunk_seconds);
                let token = shutdown.clone();
                tokio::spawn(async move {
                    source.run(token).await;
                });
            }
            SourceConfig::Local(local_config) => {
                tracing::warn!(
                    source = %local_config.name,
                    "Local audio capture not yet implemented, skipping"
                );
            }
        }
    }

    // Spawn inference consumer (or audio-level logger if no models loaded).
    let mut rx = tx.subscribe();
    let consumer_shutdown = shutdown.clone();
    let consumer_classifiers = classifiers.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(chunk) => {
                            handle_chunk(&chunk, &consumer_classifiers).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(dropped = n, "Inference consumer lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                () = consumer_shutdown.cancelled() => break,
            }
        }
    });

    // Wait for shutdown signal.
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for ctrl-c")?;
    tracing::info!("Shutting down...");
    shutdown.cancel();

    // Brief grace period for tasks to clean up.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    Ok(())
}

fn load_classifiers(config: &Config) -> Result<Vec<Arc<dyn Classifier>>> {
    let mut classifiers: Vec<Arc<dyn Classifier>> = Vec::new();

    if let Some(birdnet_config) = &config.inference.birdnet {
        let model = sitta_inference::birdnet::BirdNet::load(
            Path::new(&birdnet_config.model_path),
            Path::new(&birdnet_config.labels_path),
            birdnet_config.min_confidence,
            birdnet_config.top_k,
        )
        .context("failed to load BirdNET model")?;
        classifiers.push(Arc::new(model));
    }

    Ok(classifiers)
}

async fn handle_chunk(chunk: &AudioChunk, classifiers: &[Arc<dyn Classifier>]) {
    if classifiers.is_empty() {
        // No models -- log audio levels as before.
        tracing::info!(
            source = %chunk.source_name,
            chunk_id = %chunk.id,
            duration_s = format_args!("{:.1}", chunk.duration_secs()),
            rms_dbfs = format_args!("{:.1}", chunk.rms_dbfs()),
            "Audio chunk (no inference)"
        );
        return;
    }

    for classifier in classifiers {
        // Validate chunk matches model requirements.
        if chunk.samples.len() != classifier.window_samples() {
            tracing::debug!(
                source = %chunk.source_name,
                model = classifier.name(),
                expected = classifier.window_samples(),
                got = chunk.samples.len(),
                "Chunk size mismatch, skipping"
            );
            continue;
        }

        // Run inference on a blocking thread (CPU-bound work).
        let samples = chunk.samples.clone();
        let model = classifier.clone();
        let source_name = chunk.source_name.clone();
        let chunk_id = chunk.id;

        let result = tokio::task::spawn_blocking(move || model.classify(&samples)).await;

        match result {
            Ok(Ok(detections)) => {
                if detections.is_empty() {
                    tracing::debug!(
                        source = %source_name,
                        model = classifier.name(),
                        "No detections above threshold"
                    );
                } else {
                    for d in &detections {
                        tracing::info!(
                            source = %source_name,
                            chunk_id = %chunk_id,
                            model = classifier.name(),
                            species = %d.species.common_name,
                            scientific_name = %d.species.scientific_name,
                            confidence = format_args!("{:.3}", d.confidence),
                            "Detection"
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!(
                    source = %source_name,
                    model = classifier.name(),
                    error = %e,
                    "Inference failed"
                );
            }
            Err(e) => {
                tracing::error!(
                    model = classifier.name(),
                    error = %e,
                    "Inference task panicked"
                );
            }
        }
    }
}

mod config;

use std::sync::Arc;

use anyhow::{Context, Result};
use sitta_audio::chunk::AudioChunk;
use sitta_audio::rtsp::RtspSource;
use sitta_audio::source::SourceConfig;
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

    // Placeholder consumer: log chunk statistics.
    // This will be replaced by the inference engine in Phase 2.
    let mut rx = tx.subscribe();
    let consumer_shutdown = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(chunk) => {
                            tracing::info!(
                                source = %chunk.source_name,
                                chunk_id = %chunk.id,
                                duration_s = format_args!("{:.1}", chunk.duration_secs()),
                                peak = format_args!("{:.4}", chunk.peak()),
                                rms_dbfs = format_args!("{:.1}", chunk.rms_dbfs()),
                                samples = chunk.samples.len(),
                                "Audio chunk"
                            );
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(dropped = n, "Consumer lagged");
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

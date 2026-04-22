use std::sync::Arc;

use chrono::Utc;
use futures_util::StreamExt;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::chunk::{AudioChunk, PcmStreamHeader};
use crate::source::RemoteSourceConfig;

/// Audio source that streams PCM from another Sitta instance over HTTP.
pub struct RemoteSource {
    config: RemoteSourceConfig,
    tx: broadcast::Sender<Arc<AudioChunk>>,
}

impl RemoteSource {
    pub fn new(
        config: RemoteSourceConfig,
        tx: broadcast::Sender<Arc<AudioChunk>>,
    ) -> Self {
        Self { config, tx }
    }

    /// Run the capture loop with automatic reconnection.
    pub async fn run(self, shutdown: CancellationToken) {
        loop {
            tracing::info!(source = %self.config.name, url = %crate::sanitize_url(&self.config.url), "Connecting to remote audio stream");

            tokio::select! {
                result = self.capture_loop() => {
                    match result {
                        Ok(()) => tracing::info!(source = %self.config.name, "Remote stream ended"),
                        Err(e) => tracing::error!(source = %self.config.name, error = %e, "Remote stream error"),
                    }
                }
                () = shutdown.cancelled() => {
                    tracing::info!(source = %self.config.name, "Remote source shutting down");
                    return;
                }
            }

            // Reconnect after delay.
            tracing::info!(
                source = %self.config.name,
                delay_s = self.config.reconnect_seconds,
                "Reconnecting..."
            );
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(self.config.reconnect_seconds)) => {}
                () = shutdown.cancelled() => return,
            }
        }
    }

    async fn capture_loop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let response = reqwest::get(&self.config.url).await?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()).into());
        }

        let mut stream = response.bytes_stream();
        let mut buf = bytes::BytesMut::new();

        // Read the 20-byte header first.
        while buf.len() < std::mem::size_of::<PcmStreamHeader>() {
            match stream.next().await {
                Some(Ok(chunk)) => buf.extend_from_slice(&chunk),
                Some(Err(e)) => return Err(e.into()),
                None => return Err("stream ended before header".into()),
            }
        }

        let header_bytes = buf.split_to(std::mem::size_of::<PcmStreamHeader>());
        let header: PcmStreamHeader = *bytemuck::from_bytes(&header_bytes);

        let sample_rate = header.sample_rate;
        let channels = header.channels;
        let chunk_samples = header.chunk_samples as usize;
        let chunk_bytes = chunk_samples * std::mem::size_of::<f32>();

        tracing::info!(
            source = %self.config.name,
            sample_rate,
            channels,
            chunk_samples,
            "Remote stream header received"
        );

        // Stream audio chunks.
        loop {
            // Accumulate until we have one full chunk.
            while buf.len() < chunk_bytes {
                match stream.next().await {
                    Some(Ok(data)) => buf.extend_from_slice(&data),
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()), // clean EOF
                }
            }

            let chunk_data = buf.split_to(chunk_bytes);
            let samples: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&chunk_data)
                .to_vec();

            let audio_chunk = Arc::new(AudioChunk {
                id: Uuid::now_v7(),
                source_name: self.config.name.clone(),
                timestamp_ns: 0,
                captured_at: Utc::now(),
                sample_rate,
                channels,
                samples,
            });

            // Ignore send error — means no receivers yet.
            let _ = self.tx.send(audio_chunk);
        }
    }
}

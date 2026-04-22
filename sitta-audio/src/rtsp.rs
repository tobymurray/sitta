use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::chunk::AudioChunk;
use crate::source::RtspSourceConfig;

/// Captures audio from an RTSP stream via an ffmpeg subprocess.
///
/// ffmpeg handles all codec negotiation and decoding, outputting raw f32le PCM
/// to stdout. This means the RTSP stream can use any codec ffmpeg supports.
pub struct RtspSource {
    config: RtspSourceConfig,
    tx: broadcast::Sender<Arc<AudioChunk>>,
    chunk_duration_secs: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("failed to spawn ffmpeg: {0}")]
    FfmpegSpawn(std::io::Error),
    #[error("ffmpeg stdout not available")]
    NoStdout,
    #[error("read error: {0}")]
    Io(#[from] std::io::Error),
    #[error("RTSP stream ended (ffmpeg exited)")]
    StreamEnded,
}

impl RtspSource {
    pub fn new(
        config: RtspSourceConfig,
        tx: broadcast::Sender<Arc<AudioChunk>>,
        chunk_duration_secs: u32,
    ) -> Self {
        Self {
            config,
            tx,
            chunk_duration_secs,
        }
    }

    /// Run the capture loop, reconnecting on failure until shutdown is signalled.
    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            source = %self.config.name,
            url = %crate::sanitize_url(&self.config.url),
            sample_rate = self.config.sample_rate,
            channels = self.config.channels,
            "Starting RTSP capture"
        );

        loop {
            match self.capture_loop(&shutdown).await {
                Ok(()) => {
                    tracing::info!(source = %self.config.name, "RTSP capture stopped");
                    break;
                }
                Err(e) => {
                    tracing::error!(
                        source = %self.config.name,
                        error = %e,
                        reconnect_in = self.config.reconnect_seconds,
                        "RTSP capture failed"
                    );
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_secs(self.config.reconnect_seconds)) => {}
                        () = shutdown.cancelled() => break,
                    }
                }
            }
        }
    }

    async fn capture_loop(&self, shutdown: &CancellationToken) -> Result<(), CaptureError> {
        let mut child = self
            .build_ffmpeg_command()
            .spawn()
            .map_err(CaptureError::FfmpegSpawn)?;

        let stdout = child.stdout.take().ok_or(CaptureError::NoStdout)?;

        // Log ffmpeg stderr in a background task.
        if let Some(stderr) = child.stderr.take() {
            let name = self.config.name.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.is_empty() {
                        tracing::warn!(source = %name, ffmpeg_stderr = %line);
                    }
                }
            });
        }

        let sample_rate = self.config.sample_rate;
        let channels = self.config.channels as u32;
        let chunk_samples = (sample_rate * channels * self.chunk_duration_secs) as usize;
        let chunk_bytes = chunk_samples * 4; // f32 = 4 bytes

        let mut reader = BufReader::with_capacity(64 * 1024, stdout);
        let mut read_buf = vec![0u8; 16 * 1024];
        let mut acc = Vec::with_capacity(chunk_bytes);
        let mut chunk_index: u64 = 0;

        loop {
            tokio::select! {
                result = reader.read(&mut read_buf) => {
                    let n = result?;
                    if n == 0 {
                        return Err(CaptureError::StreamEnded);
                    }
                    acc.extend_from_slice(&read_buf[..n]);

                    while acc.len() >= chunk_bytes {
                        let samples: Vec<f32> = acc[..chunk_bytes]
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                            .collect();

                        let timestamp_ns =
                            chunk_index * self.chunk_duration_secs as u64 * 1_000_000_000;

                        let chunk = Arc::new(AudioChunk {
                            id: Uuid::now_v7(),
                            source_name: self.config.name.clone(),
                            timestamp_ns,
                            captured_at: Utc::now(),
                            sample_rate,
                            channels: self.config.channels,
                            samples,
                        });

                        // Ignore send errors (no receivers connected).
                        let _ = self.tx.send(chunk);
                        acc.drain(..chunk_bytes);
                        chunk_index += 1;
                    }
                }
                () = shutdown.cancelled() => {
                    return Ok(());
                }
            }
        }
    }

    fn build_ffmpeg_command(&self) -> Command {
        let timeout_us = self.config.timeout_seconds * 1_000_000;

        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-rtsp_transport",
            self.config.transport.as_str(),
            "-timeout",
            &timeout_us.to_string(),
            "-i",
            &self.config.url,
            "-vn",
            "-f",
            "f32le",
            "-ar",
            &self.config.sample_rate.to_string(),
            "-ac",
            &self.config.channels.to_string(),
            "-hide_banner",
            "-loglevel",
            "error",
            "pipe:1",
        ]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        cmd.kill_on_drop(true);
        cmd
    }
}

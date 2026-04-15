use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A chunk of audio samples from a single source.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Unique chunk identifier (UUIDv7, time-sortable).
    pub id: Uuid,
    /// Name of the source that produced this chunk.
    pub source_name: String,
    /// Monotonic timestamp in nanoseconds, relative to the start of capture.
    pub timestamp_ns: u64,
    /// Wall-clock time when this chunk was received.
    pub captured_at: DateTime<Utc>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
    /// Interleaved f32 samples, normalised to [-1.0, 1.0].
    pub samples: Vec<f32>,
}

impl AudioChunk {
    /// Duration of this chunk in seconds.
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / (self.sample_rate as f64 * self.channels as f64)
    }

    /// Peak absolute sample value.
    pub fn peak(&self) -> f32 {
        self.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
    }

    /// RMS (root mean square) level.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.samples.iter().map(|s| s * s).sum::<f32>() / self.samples.len() as f32).sqrt()
    }

    /// RMS level in decibels (relative to full scale).
    pub fn rms_dbfs(&self) -> f32 {
        let rms = self.rms();
        if rms <= 0.0 {
            return f32::NEG_INFINITY;
        }
        20.0 * rms.log10()
    }
}

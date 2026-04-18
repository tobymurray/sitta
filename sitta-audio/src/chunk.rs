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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk(samples: Vec<f32>) -> AudioChunk {
        AudioChunk {
            id: Uuid::nil(),
            source_name: "test".into(),
            timestamp_ns: 0,
            captured_at: Utc::now(),
            sample_rate: 48000,
            channels: 1,
            samples,
        }
    }

    #[test]
    fn duration_secs_mono() {
        let chunk = test_chunk(vec![0.0; 144_000]);
        assert!((chunk.duration_secs() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn duration_secs_stereo() {
        let mut chunk = test_chunk(vec![0.0; 96_000]);
        chunk.channels = 2;
        // 96000 samples / (48000 Hz * 2 channels) = 1.0s
        assert!((chunk.duration_secs() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn duration_secs_empty() {
        let chunk = test_chunk(vec![]);
        assert_eq!(chunk.duration_secs(), 0.0);
    }

    #[test]
    fn peak_positive_and_negative() {
        let chunk = test_chunk(vec![0.1, -0.5, 0.3, -0.2]);
        assert!((chunk.peak() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn peak_empty() {
        let chunk = test_chunk(vec![]);
        assert_eq!(chunk.peak(), 0.0);
    }

    #[test]
    fn rms_silence() {
        let chunk = test_chunk(vec![0.0; 100]);
        assert_eq!(chunk.rms(), 0.0);
    }

    #[test]
    fn rms_known_value() {
        // RMS of [1.0, -1.0] = sqrt((1+1)/2) = 1.0
        let chunk = test_chunk(vec![1.0, -1.0]);
        assert!((chunk.rms() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rms_empty() {
        let chunk = test_chunk(vec![]);
        assert_eq!(chunk.rms(), 0.0);
    }

    #[test]
    fn rms_dbfs_full_scale() {
        // Full-scale sine wave peak at 1.0 → RMS = 1/√2 ≈ 0.707 → -3.01 dBFS
        let rms = 1.0_f32 / 2.0_f32.sqrt();
        let samples = vec![rms; 100]; // constant signal at this RMS
        let chunk = test_chunk(samples);
        let dbfs = chunk.rms_dbfs();
        assert!((dbfs - (-3.01)).abs() < 0.1);
    }

    #[test]
    fn rms_dbfs_silence() {
        let chunk = test_chunk(vec![0.0; 100]);
        assert!(chunk.rms_dbfs().is_infinite() && chunk.rms_dbfs().is_sign_negative());
    }
}

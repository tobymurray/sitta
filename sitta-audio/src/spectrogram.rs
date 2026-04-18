//! Mel spectrogram generation from audio samples.
//!
//! Produces a PNG image from raw f32 PCM audio using a short-time Fourier
//! transform (STFT), mel filterbank, and a viridis-style colormap. Pure Rust
//! with no external binary dependencies (no sox, ffmpeg, or imagemagick).
//!
//! Tuned for Raspberry Pi 5: a 512-point FFT over 5 seconds of 48 kHz audio
//! produces ~937 frames and takes <5 ms including PNG encoding.

use std::f32::consts::PI;
use std::io;
use std::path::Path;

use image::{ImageBuffer, Rgb};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Parameters controlling spectrogram appearance and computation.
pub struct SpectrogramParams {
    /// FFT window size in samples. Default: 512.
    pub fft_size: usize,
    /// Hop between successive windows. Default: 256 (50% overlap).
    pub hop_size: usize,
    /// Number of mel frequency bins. Default: 80.
    pub mel_bins: usize,
    /// Lower frequency bound in Hz. Default: 150 (below most bird calls).
    pub f_min: f32,
    /// Upper frequency bound in Hz. Default: 15000 (above most bird calls).
    pub f_max: f32,
    /// Output image width in pixels. 0 = one pixel per time frame.
    pub width: u32,
    /// Output image height in pixels. 0 = one pixel per mel bin.
    pub height: u32,
}

impl Default for SpectrogramParams {
    fn default() -> Self {
        Self {
            fft_size: 512,
            hop_size: 256,
            mel_bins: 80,
            f_min: 150.0,
            f_max: 15000.0,
            width: 0,
            height: 0,
        }
    }
}

/// Generate a mel spectrogram PNG from audio samples.
pub fn generate_spectrogram(
    samples: &[f32],
    sample_rate: u32,
    params: &SpectrogramParams,
    output_path: &Path,
) -> io::Result<()> {
    if samples.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no samples"));
    }

    let fft_size = params.fft_size;
    let hop = params.hop_size;
    let sr = sample_rate as f32;

    // Build Hann window.
    let hann: Vec<f32> = (0..fft_size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / fft_size as f32).cos()))
        .collect();

    // Compute STFT magnitude.
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);
    let num_frames = if samples.len() >= fft_size {
        (samples.len() - fft_size) / hop + 1
    } else {
        0
    };
    let freq_bins = fft_size / 2 + 1;

    let mut magnitudes: Vec<Vec<f32>> = Vec::with_capacity(num_frames);
    let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];

    for frame_idx in 0..num_frames {
        let start = frame_idx * hop;
        let mut buf: Vec<Complex<f32>> = (0..fft_size)
            .map(|i| {
                let s = if start + i < samples.len() {
                    samples[start + i]
                } else {
                    0.0
                };
                Complex::new(s * hann[i], 0.0)
            })
            .collect();
        fft.process_with_scratch(&mut buf, &mut scratch);
        let mag: Vec<f32> = buf[..freq_bins]
            .iter()
            .map(|c| (c.re * c.re + c.im * c.im).sqrt())
            .collect();
        magnitudes.push(mag);
    }

    // Build mel filterbank.
    let mel_fb = mel_filterbank(params.mel_bins, fft_size, sr, params.f_min, params.f_max);

    // Apply filterbank: mel_spec[frame][mel_bin].
    let mut mel_spec: Vec<Vec<f32>> = Vec::with_capacity(num_frames);
    for mag in &magnitudes {
        let mut mel_frame = vec![0.0f32; params.mel_bins];
        for (bin, weights) in mel_fb.iter().enumerate() {
            let mut sum = 0.0f32;
            for &(freq_idx, weight) in weights {
                if freq_idx < mag.len() {
                    sum += mag[freq_idx] * weight;
                }
            }
            mel_frame[bin] = sum;
        }
        mel_spec.push(mel_frame);
    }

    // Convert to dB scale.
    let mut db_min = f32::MAX;
    let mut db_max = f32::MIN;
    for frame in &mut mel_spec {
        for val in frame.iter_mut() {
            *val = 10.0 * (*val + 1e-10).log10();
            if *val < db_min {
                db_min = *val;
            }
            if *val > db_max {
                db_max = *val;
            }
        }
    }

    // Normalize to [0, 1].
    let db_range = (db_max - db_min).max(1e-6);
    for frame in &mut mel_spec {
        for val in frame.iter_mut() {
            *val = ((*val - db_min) / db_range).clamp(0.0, 1.0);
        }
    }

    // Render to image.
    let img_w = if params.width > 0 {
        params.width
    } else {
        num_frames as u32
    };
    let img_h = if params.height > 0 {
        params.height
    } else {
        params.mel_bins as u32
    };

    let mut img = ImageBuffer::new(img_w, img_h);
    for px in 0..img_w {
        let frame_idx = (px as f32 / img_w as f32 * num_frames as f32) as usize;
        let frame_idx = frame_idx.min(num_frames.saturating_sub(1));
        for py in 0..img_h {
            // Flip vertically: low frequencies at bottom.
            let mel_idx =
                ((img_h - 1 - py) as f32 / img_h as f32 * params.mel_bins as f32) as usize;
            let mel_idx = mel_idx.min(params.mel_bins.saturating_sub(1));
            let val = mel_spec[frame_idx][mel_idx];
            let (r, g, b) = viridis(val);
            img.put_pixel(px, py, Rgb([r, g, b]));
        }
    }

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    img.save(output_path)
        .map_err(io::Error::other)?;
    Ok(())
}

/// Build a mel filterbank as a sparse representation.
/// Returns `mel_bins` entries, each a Vec of (fft_bin_index, weight).
fn mel_filterbank(
    num_mels: usize,
    fft_size: usize,
    sample_rate: f32,
    f_min: f32,
    f_max: f32,
) -> Vec<Vec<(usize, f32)>> {
    let freq_bins = fft_size / 2 + 1;
    let mel_min = hz_to_mel(f_min);
    let mel_max = hz_to_mel(f_max);

    // num_mels + 2 points: edges of the triangular filters.
    let mel_points: Vec<f32> = (0..=num_mels + 1)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (num_mels + 1) as f32)
        .collect();
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
    let bin_points: Vec<f32> = hz_points
        .iter()
        .map(|&f| f * fft_size as f32 / sample_rate)
        .collect();

    let mut filters = Vec::with_capacity(num_mels);
    for m in 0..num_mels {
        let left = bin_points[m];
        let center = bin_points[m + 1];
        let right = bin_points[m + 2];
        let mut weights = Vec::new();
        for k in 0..freq_bins {
            let kf = k as f32;
            let w = if kf > left && kf < center {
                (kf - left) / (center - left)
            } else if kf >= center && kf < right {
                (right - kf) / (right - center)
            } else {
                0.0
            };
            if w > 0.0 {
                weights.push((k, w));
            }
        }
        filters.push(weights);
    }
    filters
}

fn hz_to_mel(f: f32) -> f32 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_to_hz(m: f32) -> f32 {
    700.0 * (10.0f32.powf(m / 2595.0) - 1.0)
}

/// Viridis-inspired colormap. Input: [0, 1]. Output: (r, g, b).
fn viridis(t: f32) -> (u8, u8, u8) {
    // Simplified piecewise-linear approximation of the viridis colormap.
    let (r, g, b) = if t < 0.25 {
        let s = t / 0.25;
        (
            68.0 + s * (49.0 - 68.0),
            1.0 + s * (54.0 - 1.0),
            84.0 + s * (149.0 - 84.0),
        )
    } else if t < 0.5 {
        let s = (t - 0.25) / 0.25;
        (
            49.0 + s * (33.0 - 49.0),
            54.0 + s * (144.0 - 54.0),
            149.0 + s * (141.0 - 149.0),
        )
    } else if t < 0.75 {
        let s = (t - 0.5) / 0.25;
        (
            33.0 + s * (144.0 - 33.0),
            144.0 + s * (201.0 - 144.0),
            141.0 + s * (74.0 - 141.0),
        )
    } else {
        let s = (t - 0.75) / 0.25;
        (
            144.0 + s * (253.0 - 144.0),
            201.0 + s * (231.0 - 201.0),
            74.0 + s * (37.0 - 74.0),
        )
    };
    (r as u8, g as u8, b as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine_wave(freq: f32, sample_rate: u32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate as f32 * duration_secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn generates_png_from_sine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spec.png");
        let samples = sine_wave(1000.0, 48000, 3.0);

        generate_spectrogram(&samples, 48000, &SpectrogramParams::default(), &path).unwrap();
        assert!(path.exists());

        // Verify it's a valid PNG by checking magic bytes.
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[..4], b"\x89PNG");
    }

    #[test]
    fn output_dimensions_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dims.png");
        let samples = sine_wave(440.0, 48000, 1.0);
        let params = SpectrogramParams::default();

        generate_spectrogram(&samples, 48000, &params, &path).unwrap();

        let img = image::open(&path).unwrap();
        let expected_frames = (48000 - params.fft_size) / params.hop_size + 1;
        assert_eq!(img.width(), expected_frames as u32);
        assert_eq!(img.height(), params.mel_bins as u32);
    }

    #[test]
    fn output_dimensions_custom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom.png");
        let samples = sine_wave(2000.0, 48000, 3.0);
        let params = SpectrogramParams {
            width: 800,
            height: 200,
            ..Default::default()
        };

        generate_spectrogram(&samples, 48000, &params, &path).unwrap();

        let img = image::open(&path).unwrap();
        assert_eq!(img.width(), 800);
        assert_eq!(img.height(), 200);
    }

    #[test]
    fn five_second_perch_clip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("perch.png");
        let samples = sine_wave(3000.0, 48000, 5.0);
        let params = SpectrogramParams {
            width: 600,
            height: 200,
            ..Default::default()
        };

        generate_spectrogram(&samples, 48000, &params, &path).unwrap();
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
    }

    #[test]
    fn rejects_empty_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.png");

        let err = generate_spectrogram(&[], 48000, &SpectrogramParams::default(), &path);
        assert!(err.is_err());
    }

    #[test]
    fn mel_conversion_roundtrip() {
        for freq in [100.0, 440.0, 1000.0, 4000.0, 10000.0] {
            let mel = hz_to_mel(freq);
            let hz = mel_to_hz(mel);
            assert!((hz - freq).abs() < 0.1, "roundtrip failed for {freq}");
        }
    }

    #[test]
    fn viridis_range() {
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let (r, g, b) = viridis(t);
            // Just verify no panics and values are in u8 range (guaranteed by type).
            assert!(r <= 255 && g <= 255 && b <= 255);
        }
    }

    #[test]
    fn creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep").join("nested").join("spec.png");
        let samples = sine_wave(1000.0, 48000, 1.0);

        generate_spectrogram(&samples, 48000, &SpectrogramParams::default(), &path).unwrap();
        assert!(path.exists());
    }
}

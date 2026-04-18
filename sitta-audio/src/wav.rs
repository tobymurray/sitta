//! Minimal WAV reader/writer for audio clip persistence.
//!
//! Writes 16-bit PCM WAV (f32 → i16 conversion halves file size compared to
//! f32le). Uses atomic writes (temp file + rename) to avoid serving partial
//! files. No external crate dependencies.

use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

/// Write f32 samples as a 16-bit PCM WAV file.
///
/// The write is atomic: data goes to `{path}.tmp` first, then renamed on
/// success. If a `.tmp` file already exists it is overwritten.
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32, channels: u16) -> io::Result<()> {
    let tmp_path = path.with_extension("wav.tmp");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes_per_sample: u16 = 2; // 16-bit PCM
    let block_align = channels * bytes_per_sample;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_size = samples.len() as u32 * u32::from(bytes_per_sample);
    let file_size = 36 + data_size; // RIFF header (44) minus 8-byte preamble + data

    let file = fs::File::create(&tmp_path)?;
    let mut w = BufWriter::new(file);

    // RIFF header
    w.write_all(b"RIFF")?;
    w.write_all(&file_size.to_le_bytes())?;
    w.write_all(b"WAVE")?;

    // fmt sub-chunk
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // sub-chunk size
    w.write_all(&1u16.to_le_bytes())?; // audio format: PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&(bytes_per_sample * 8).to_le_bytes())?; // bits per sample

    // data sub-chunk
    w.write_all(b"data")?;
    w.write_all(&data_size.to_le_bytes())?;
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let int_sample = (clamped * f32::from(i16::MAX)) as i16;
        w.write_all(&int_sample.to_le_bytes())?;
    }
    w.flush()?;
    drop(w);

    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Read a 16-bit PCM WAV file back into f32 samples.
///
/// Returns `(samples, sample_rate, channels)`.
pub fn read_wav(path: &Path) -> io::Result<(Vec<f32>, u32, u16)> {
    let data = fs::read(path)?;
    if data.len() < 44 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "WAV file too short"));
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a WAV file"));
    }

    let channels = u16::from_le_bytes([data[22], data[23]]);
    let sample_rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let bits_per_sample = u16::from_le_bytes([data[34], data[35]]);

    // Find the data chunk (it's usually at offset 36, but be robust).
    let data_start = find_chunk(&data, b"data")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no data chunk"))?;
    let data_size =
        u32::from_le_bytes([data[data_start], data[data_start + 1], data[data_start + 2], data[data_start + 3]])
            as usize;
    let pcm_start = data_start + 4;
    let pcm_end = (pcm_start + data_size).min(data.len());
    let pcm = &data[pcm_start..pcm_end];

    let samples = match bits_per_sample {
        16 => pcm
            .chunks_exact(2)
            .map(|b| {
                let int_sample = i16::from_le_bytes([b[0], b[1]]);
                f32::from(int_sample) / f32::from(i16::MAX)
            })
            .collect(),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported bits per sample: {bits_per_sample}"),
            ));
        }
    };

    Ok((samples, sample_rate, channels))
}

/// Compute the duration of a WAV file in milliseconds from its metadata,
/// without reading the sample data.
pub fn duration_ms_from_path(path: &Path) -> io::Result<i64> {
    let mut file = fs::File::open(path)?;
    let mut header = [0u8; 44];
    file.read_exact(&mut header)?;

    let byte_rate = u32::from_le_bytes([header[28], header[29], header[30], header[31]]);
    if byte_rate == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "byte rate is zero"));
    }

    let data = fs::read(path)?;
    let data_offset = find_chunk(&data, b"data")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no data chunk"))?;
    let data_size = u32::from_le_bytes([
        data[data_offset],
        data[data_offset + 1],
        data[data_offset + 2],
        data[data_offset + 3],
    ]);

    let duration_ms = (u64::from(data_size) * 1000) / u64::from(byte_rate);
    Ok(duration_ms as i64)
}

/// Find a RIFF chunk by its 4-byte ID. Returns the offset of the chunk's
/// size field (i.e., 4 bytes past the chunk ID).
fn find_chunk(data: &[u8], id: &[u8; 4]) -> Option<usize> {
    // Skip RIFF header (12 bytes), then scan sub-chunks.
    let mut pos = 12;
    while pos + 8 <= data.len() {
        if &data[pos..pos + 4] == id {
            return Some(pos + 4);
        }
        let chunk_size =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        pos += 8 + chunk_size;
        // Chunks are word-aligned.
        if pos % 2 != 0 {
            pos += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine_wave(freq: f32, sample_rate: u32, duration_secs: f32) -> Vec<f32> {
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        (0..num_samples)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn round_trip_preserves_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let samples = sine_wave(440.0, 48000, 1.0);

        write_wav(&path, &samples, 48000, 1).unwrap();
        let (read_samples, rate, channels) = read_wav(&path).unwrap();

        assert_eq!(rate, 48000);
        assert_eq!(channels, 1);
        assert_eq!(read_samples.len(), samples.len());

        // 16-bit quantization introduces error up to 1/32767 ~ 3e-5.
        let max_error: f32 = samples
            .iter()
            .zip(read_samples.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_error < 0.001, "max quantization error {max_error} too large");
    }

    #[test]
    fn round_trip_stereo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo.wav");
        // Interleaved stereo: L R L R ...
        let samples: Vec<f32> = (0..9600).map(|i| (i as f32 / 9600.0) * 2.0 - 1.0).collect();

        write_wav(&path, &samples, 48000, 2).unwrap();
        let (read_samples, rate, channels) = read_wav(&path).unwrap();

        assert_eq!(rate, 48000);
        assert_eq!(channels, 2);
        assert_eq!(read_samples.len(), samples.len());
    }

    #[test]
    fn clamps_out_of_range_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clamp.wav");
        let samples = vec![-2.0, -1.0, 0.0, 1.0, 2.0];

        write_wav(&path, &samples, 16000, 1).unwrap();
        let (read_samples, _, _) = read_wav(&path).unwrap();

        // -2.0 and 2.0 should be clamped to -1.0 and 1.0.
        assert!((read_samples[0] - (-1.0)).abs() < 0.001);
        assert!((read_samples[4] - 1.0).abs() < 0.001);
    }

    #[test]
    fn atomic_write_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atomic.wav");
        let samples = sine_wave(1000.0, 48000, 0.5);

        write_wav(&path, &samples, 48000, 1).unwrap();

        // The tmp file should not exist after a successful write.
        assert!(!path.with_extension("wav.tmp").exists());
        assert!(path.exists());
    }

    #[test]
    fn creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026").join("04").join("17").join("test.wav");
        let samples = vec![0.0; 100];

        write_wav(&path, &samples, 48000, 1).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn duration_ms_correct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("duration.wav");
        // 3 seconds at 48kHz mono
        let samples = vec![0.0; 144_000];

        write_wav(&path, &samples, 48000, 1).unwrap();
        let ms = duration_ms_from_path(&path).unwrap();
        assert_eq!(ms, 3000);
    }

    #[test]
    fn five_second_perch_clip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("perch.wav");
        // 5 seconds at 48kHz mono (Perch window size before resampling)
        let samples = sine_wave(2000.0, 48000, 5.0);

        write_wav(&path, &samples, 48000, 1).unwrap();
        let ms = duration_ms_from_path(&path).unwrap();
        assert_eq!(ms, 5000);

        let (read_back, rate, ch) = read_wav(&path).unwrap();
        assert_eq!(rate, 48000);
        assert_eq!(ch, 1);
        assert_eq!(read_back.len(), 240_000);
    }

    #[test]
    fn empty_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wav");

        write_wav(&path, &[], 48000, 1).unwrap();
        let (samples, rate, ch) = read_wav(&path).unwrap();
        assert_eq!(samples.len(), 0);
        assert_eq!(rate, 48000);
        assert_eq!(ch, 1);
    }

    #[test]
    fn rejects_non_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_a.wav");
        std::fs::write(&path, b"this is not a wav file at all").unwrap();

        let err = read_wav(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}

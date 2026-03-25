/// Pre-computed spectrogram via STFT for visualization.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

const FFT_SIZE: usize = 2048;
const HOP_SIZE: usize = 512;
const DB_FLOOR: f32 = -80.0;
const DB_CEIL: f32 = 0.0;

/// Pre-computed spectrogram magnitudes, quantized to u8.
pub struct SpectrogramResult {
    /// Magnitude values quantized to u8: 0 = -80dB (floor), 255 = 0dB (ceiling).
    /// Layout: row-major [column][bin], access via `magnitudes[col * num_bins + bin]`.
    pub magnitudes: Vec<u8>,
    /// Number of frequency bins (FFT_SIZE/2 + 1).
    pub num_bins: usize,
    /// Number of time columns.
    pub num_columns: usize,
    /// Hop size in audio frames.
    pub hop_size: usize,
    /// FFT window size.
    pub fft_size: usize,
}

/// Compute a spectrogram from mono audio samples.
/// Input should be mono f32 samples. If stereo, downmix first.
pub fn compute_spectrogram(samples: &[f32], _sample_rate: u32) -> SpectrogramResult {
    let num_bins = FFT_SIZE / 2 + 1;

    if samples.len() < FFT_SIZE {
        return SpectrogramResult {
            magnitudes: Vec::new(),
            num_bins,
            num_columns: 0,
            hop_size: HOP_SIZE,
            fft_size: FFT_SIZE,
        };
    }

    // Hann window
    let hann: Vec<f32> = (0..FFT_SIZE)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos()))
        .collect();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    let num_columns = (samples.len().saturating_sub(FFT_SIZE)) / HOP_SIZE + 1;
    let mut magnitudes = vec![0u8; num_columns * num_bins];
    let mut fft_buf = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];

    for col in 0..num_columns {
        let start = col * HOP_SIZE;
        let end = start + FFT_SIZE;
        if end > samples.len() {
            break;
        }

        // Window + copy to FFT buffer
        for i in 0..FFT_SIZE {
            fft_buf[i] = Complex::new(samples[start + i] * hann[i], 0.0);
        }

        fft.process(&mut fft_buf);

        // Compute magnitude in dB, quantize to u8
        let offset = col * num_bins;
        for bin in 0..num_bins {
            let mag = fft_buf[bin].norm();
            let db = if mag > 1e-10 { 20.0 * mag.log10() } else { DB_FLOOR };
            let normalized = ((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
            magnitudes[offset + bin] = (normalized * 255.0) as u8;
        }
    }

    SpectrogramResult {
        magnitudes,
        num_bins,
        num_columns,
        hop_size: HOP_SIZE,
        fft_size: FFT_SIZE,
    }
}

/// Downmix interleaved audio to mono.
pub fn downmix_to_mono(data: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels as usize;
    if ch <= 1 {
        return data.to_vec();
    }
    let frames = data.len() / ch;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut sum = 0.0f32;
        for c in 0..ch {
            sum += data[frame * ch + c];
        }
        mono.push(sum / ch as f32);
    }
    mono
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spectrogram_sine() {
        let sr = 48000u32;
        let freq = 440.0f32;
        let duration = 0.5;
        let samples: Vec<f32> = (0..(sr as f64 * duration) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect();

        let result = compute_spectrogram(&samples, sr);
        assert!(result.num_columns > 0);
        assert_eq!(result.num_bins, FFT_SIZE / 2 + 1);

        // The 440Hz bin should have high magnitude
        let bin_440 = (440.0 * FFT_SIZE as f32 / sr as f32).round() as usize;
        // Check middle column
        let mid_col = result.num_columns / 2;
        let val = result.magnitudes[mid_col * result.num_bins + bin_440];
        assert!(val > 100, "Expected high magnitude at 440Hz bin, got {val}");
    }

    #[test]
    fn test_downmix() {
        let stereo = vec![1.0f32, 0.0, 0.0, 1.0, 0.5, 0.5];
        let mono = downmix_to_mono(&stereo, 2);
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.5).abs() < 0.01);
        assert!((mono[1] - 0.5).abs() < 0.01);
        assert!((mono[2] - 0.5).abs() < 0.01);
    }
}

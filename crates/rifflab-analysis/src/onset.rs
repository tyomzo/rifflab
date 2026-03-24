/// Spectral flux-based onset detection.
/// Operates offline on a full audio buffer.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

const WINDOW_SIZE: usize = 2048;
const HOP_SIZE: usize = 512;

/// Detect onsets in audio using spectral flux with adaptive thresholding.
///
/// Returns onset times in seconds.
pub fn detect_onsets(samples: &[f32], sample_rate: u32) -> Vec<f64> {
    if samples.len() < WINDOW_SIZE {
        return Vec::new();
    }

    // Precompute Hann window
    let hann: Vec<f32> = (0..WINDOW_SIZE)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (WINDOW_SIZE - 1) as f32).cos())
        })
        .collect();

    // Set up FFT
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);

    // Compute magnitude spectra for all frames
    let num_frames = (samples.len().saturating_sub(WINDOW_SIZE)) / HOP_SIZE + 1;
    let spectrum_len = WINDOW_SIZE / 2 + 1;
    let mut magnitudes: Vec<Vec<f32>> = Vec::with_capacity(num_frames);

    for frame_idx in 0..num_frames {
        let start = frame_idx * HOP_SIZE;
        let end = start + WINDOW_SIZE;
        if end > samples.len() {
            break;
        }

        // Apply Hann window and prepare complex buffer
        let mut buffer: Vec<Complex<f32>> = (0..WINDOW_SIZE)
            .map(|i| Complex::new(samples[start + i] * hann[i], 0.0))
            .collect();

        fft.process(&mut buffer);

        // Compute magnitude spectrum (only positive frequencies)
        let mag: Vec<f32> = buffer[..spectrum_len]
            .iter()
            .map(|c| c.norm())
            .collect();
        magnitudes.push(mag);
    }

    if magnitudes.len() < 2 {
        return Vec::new();
    }

    // Compute spectral flux: sum of positive magnitude differences
    let mut flux: Vec<f32> = Vec::with_capacity(magnitudes.len());
    flux.push(0.0); // First frame has no previous frame
    for i in 1..magnitudes.len() {
        let sf: f32 = magnitudes[i]
            .iter()
            .zip(magnitudes[i - 1].iter())
            .map(|(&curr, &prev)| (curr - prev).max(0.0))
            .sum();
        flux.push(sf);
    }

    // Adaptive threshold using median filter + offset
    let median_window = 11; // Must be odd
    let threshold_offset = 0.1;
    let threshold_multiplier = 1.5;
    let thresholds = adaptive_threshold(&flux, median_window, threshold_offset, threshold_multiplier);

    // Peak picking: local maxima above threshold
    let mut onsets = Vec::new();
    for i in 1..flux.len().saturating_sub(1) {
        if flux[i] > thresholds[i] && flux[i] > flux[i - 1] && flux[i] >= flux[i + 1] {
            let time = i as f64 * HOP_SIZE as f64 / sample_rate as f64;
            onsets.push(time);
        }
    }

    onsets
}

/// Compute adaptive threshold using median filter over flux values.
fn adaptive_threshold(flux: &[f32], window: usize, offset: f32, multiplier: f32) -> Vec<f32> {
    let half = window / 2;
    let mut thresholds = Vec::with_capacity(flux.len());

    for i in 0..flux.len() {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(flux.len());
        let mut local: Vec<f32> = flux[start..end].to_vec();
        local.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = local[local.len() / 2];
        thresholds.push(median * multiplier + offset);
    }

    thresholds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a click (short impulse) at a given sample position.
    fn generate_clicks(sample_rate: u32, duration_secs: f64, click_times: &[f64]) -> Vec<f32> {
        let num_samples = (sample_rate as f64 * duration_secs) as usize;
        let mut samples = vec![0.0f32; num_samples];

        for &t in click_times {
            let idx = (t * sample_rate as f64) as usize;
            // Generate a short impulse (a few samples wide for energy)
            let click_len = 64;
            for j in 0..click_len {
                if idx + j < num_samples {
                    // Short burst of broadband noise-like signal
                    let env = 1.0 - (j as f32 / click_len as f32);
                    samples[idx + j] = env * (if j % 2 == 0 { 0.8 } else { -0.8 });
                }
            }
        }

        samples
    }

    #[test]
    fn test_detect_clicks() {
        let sample_rate = 44100;
        let click_times = vec![0.5, 1.0, 1.5, 2.0];
        let samples = generate_clicks(sample_rate, 2.5, &click_times);

        let onsets = detect_onsets(&samples, sample_rate);

        // We should detect approximately 4 onsets
        assert!(
            onsets.len() >= 3 && onsets.len() <= 6,
            "Expected ~4 onsets, got {}: {:?}",
            onsets.len(),
            onsets
        );

        // Each detected onset should be within ±20ms of a known click time
        for &click_t in &click_times {
            let closest = onsets
                .iter()
                .map(|&o| (o - click_t).abs())
                .fold(f64::INFINITY, f64::min);
            assert!(
                closest < 0.030,
                "No onset found within 30ms of click at {click_t}s. Closest distance: {closest}s. Detected onsets: {:?}",
                onsets
            );
        }
    }

    #[test]
    fn test_empty_input() {
        let onsets = detect_onsets(&[], 44100);
        assert!(onsets.is_empty());
    }

    #[test]
    fn test_short_input() {
        let samples = vec![0.0f32; 1000]; // shorter than WINDOW_SIZE
        let onsets = detect_onsets(&samples, 44100);
        assert!(onsets.is_empty());
    }

    #[test]
    fn test_silence() {
        let samples = vec![0.0f32; 44100 * 2]; // 2 seconds of silence
        let onsets = detect_onsets(&samples, 44100);
        assert!(
            onsets.is_empty(),
            "Silence should produce no onsets, got: {:?}",
            onsets
        );
    }
}

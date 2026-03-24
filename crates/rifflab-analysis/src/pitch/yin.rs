use rifflab_core::analysis::PitchFrame;

/// YIN pitch detection algorithm.
///
/// Maintains an internal circular buffer for pitch detection across
/// audio callback boundaries.
pub struct YinDetector {
    buffer: Vec<f32>,
    write_pos: usize,
    sample_rate: u32,
    threshold: f32,
    min_freq: f32,
    max_freq: f32,
    /// Pre-allocated difference function buffer (avoids alloc in detect).
    diff: Vec<f32>,
    /// Pre-allocated cumulative mean normalized difference buffer.
    cmnd: Vec<f32>,
}

impl YinDetector {
    pub fn new(sample_rate: u32) -> Self {
        // Buffer size = 2 * max_period. For min_freq 30 Hz at 48kHz: period = 1600 samples.
        let max_period = (sample_rate as f32 / 30.0).ceil() as usize;
        let buf_size = max_period * 2;
        Self {
            buffer: vec![0.0; buf_size],
            write_pos: 0,
            sample_rate,
            threshold: 0.15,
            min_freq: 30.0,
            max_freq: 2000.0,
            diff: vec![0.0; max_period],
            cmnd: vec![0.0; max_period],
        }
    }

    /// Feed samples from the audio callback and detect pitch.
    /// Call this once per audio buffer.
    pub fn detect(&mut self, samples: &[f32]) -> PitchFrame {
        // Write incoming samples into circular buffer
        for &s in samples {
            self.buffer[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % self.buffer.len();
        }

        // Run YIN on the buffer
        let (freq, confidence) = self.yin_detect();

        let (midi_note, cents) = if freq > 0.0 {
            freq_to_midi_cents(freq)
        } else {
            (0, 0.0)
        };

        PitchFrame {
            frequency_hz: freq,
            confidence,
            midi_note,
            cents_deviation: cents,
        }
    }

    fn yin_detect(&mut self) -> (f32, f32) {
        let buf = &self.buffer;
        let half_len = buf.len() / 2;
        let min_tau = (self.sample_rate as f32 / self.max_freq).floor() as usize;
        let max_tau = (self.sample_rate as f32 / self.min_freq).ceil() as usize;
        let max_tau = max_tau.min(half_len);

        if min_tau >= max_tau {
            return (0.0, 0.0);
        }

        // Step 1-2: Difference function (re-use pre-allocated buffer)
        let diff = &mut self.diff[..max_tau];
        diff.fill(0.0);
        for tau in 1..max_tau {
            let mut sum = 0.0;
            for j in 0..half_len {
                let idx1 = (self.write_pos + buf.len() - half_len + j) % buf.len();
                let idx2 = (idx1 + tau) % buf.len();
                let d = buf[idx1] - buf[idx2];
                sum += d * d;
            }
            diff[tau] = sum;
        }

        // Step 3: Cumulative mean normalized difference function (CMND)
        let cmnd = &mut self.cmnd[..max_tau];
        cmnd.fill(0.0);
        cmnd[0] = 1.0;
        let mut running_sum = 0.0;
        for tau in 1..max_tau {
            running_sum += diff[tau];
            if running_sum > 0.0 {
                cmnd[tau] = diff[tau] * tau as f32 / running_sum;
            } else {
                cmnd[tau] = 1.0;
            }
        }

        // Step 4: Absolute threshold
        let mut best_tau = 0;
        for tau in min_tau..max_tau {
            if cmnd[tau] < self.threshold {
                // Find the local minimum
                while best_tau + 1 < max_tau && cmnd[best_tau + 1] < cmnd[best_tau] {
                    best_tau += 1;
                }
                if best_tau < min_tau {
                    best_tau = tau;
                }
                break;
            }
            if best_tau == 0 || cmnd[tau] < cmnd[best_tau] {
                best_tau = tau;
            }
        }

        if best_tau == 0 || best_tau < min_tau {
            return (0.0, 0.0);
        }

        // Step 5: Parabolic interpolation
        let tau_f = if best_tau > 0 && best_tau + 1 < max_tau {
            let s0 = cmnd[best_tau - 1];
            let s1 = cmnd[best_tau];
            let s2 = cmnd[best_tau + 1];
            let denom = 2.0 * s1 - s2 - s0;
            if denom.abs() > 1e-10 {
                best_tau as f32 + (s0 - s2) / (2.0 * denom)
            } else {
                best_tau as f32
            }
        } else {
            best_tau as f32
        };

        let freq = self.sample_rate as f32 / tau_f;
        let confidence = 1.0 - cmnd[best_tau];

        if freq >= self.min_freq && freq <= self.max_freq {
            (freq, confidence.max(0.0).min(1.0))
        } else {
            (0.0, 0.0)
        }
    }
}

impl YinDetector {
    /// Process a full audio buffer offline and return pitch data for each frame.
    ///
    /// Returns a `Vec` of `(time_seconds, frequency_hz, confidence)` tuples.
    /// Uses a hop size of 256 samples (same as real-time).
    /// Frames with very low energy (silence) return 0.0 Hz.
    pub fn detect_batch(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Vec<(f64, f32, f32)> {
        let hop_size = 256;
        let silence_threshold = 1e-6; // RMS threshold below which we consider silence
        let mut results = Vec::new();

        // Reset internal state for a clean run
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.sample_rate = sample_rate;

        let num_chunks = samples.len() / hop_size;
        for i in 0..num_chunks {
            let start = i * hop_size;
            let end = start + hop_size;
            let chunk = &samples[start..end];

            // Check energy: if the chunk is essentially silent, skip pitch detection
            let rms = (chunk.iter().map(|&s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            if rms < silence_threshold {
                // Still feed the buffer so state stays consistent
                for &s in chunk {
                    self.buffer[self.write_pos] = s;
                    self.write_pos = (self.write_pos + 1) % self.buffer.len();
                }
                let time = start as f64 / sample_rate as f64;
                results.push((time, 0.0, 0.0));
                continue;
            }

            let frame = self.detect(chunk);
            let time = start as f64 / sample_rate as f64;
            results.push((time, frame.frequency_hz, frame.confidence));
        }

        results
    }
}

/// Convert frequency to MIDI note number and cents deviation.
fn freq_to_midi_cents(freq: f32) -> (u8, f32) {
    let midi_float = 69.0 + 12.0 * (freq / 440.0).log2();
    let midi_note = midi_float.round() as u8;
    let cents = (midi_float - midi_note as f32) * 100.0;
    (midi_note, cents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freq_to_midi_cents() {
        let (note, cents) = freq_to_midi_cents(440.0);
        assert_eq!(note, 69); // A4
        assert!(cents.abs() < 0.1);
    }

    #[test]
    fn test_detect_sine_wave() {
        let sample_rate = 48000;
        let freq = 440.0;
        let mut detector = YinDetector::new(sample_rate);

        // Generate a few buffers of sine wave to fill the detector's buffer
        let buffer_size = 256;
        let num_buffers = 20;
        let mut result = PitchFrame {
            frequency_hz: 0.0,
            confidence: 0.0,
            midi_note: 0,
            cents_deviation: 0.0,
        };

        for i in 0..num_buffers {
            let samples: Vec<f32> = (0..buffer_size)
                .map(|j| {
                    let t = (i * buffer_size + j) as f32 / sample_rate as f32;
                    (2.0 * std::f32::consts::PI * freq * t).sin()
                })
                .collect();
            result = detector.detect(&samples);
        }

        assert!(
            (result.frequency_hz - freq).abs() < 5.0,
            "Expected ~{freq} Hz, got {} Hz",
            result.frequency_hz
        );
        assert!(result.confidence > 0.5);
        // MIDI note 69 = A4, should still round correctly within 5 Hz
        assert_eq!(result.midi_note, 69);
    }

    #[test]
    fn test_detect_batch_sine_440() {
        let sample_rate = 48000u32;
        let freq = 440.0f32;
        let duration_secs = 0.5;
        let num_samples = (sample_rate as f64 * duration_secs) as usize;

        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect();

        let mut detector = YinDetector::new(sample_rate);
        let results = detector.detect_batch(&samples, sample_rate);

        assert!(!results.is_empty(), "Should have at least one frame");

        // Skip the first few frames while the buffer fills up
        let stable_results: Vec<_> = results.iter().skip(15).collect();
        assert!(!stable_results.is_empty());

        for &&(_, freq_hz, confidence) in &stable_results {
            if confidence > 0.5 {
                assert!(
                    (freq_hz - freq).abs() < 10.0,
                    "Expected ~{freq} Hz, got {freq_hz} Hz"
                );
            }
        }
    }

    #[test]
    fn test_detect_batch_silence() {
        let sample_rate = 48000u32;
        let duration_secs = 0.2;
        let num_samples = (sample_rate as f64 * duration_secs) as usize;

        let samples = vec![0.0f32; num_samples];

        let mut detector = YinDetector::new(sample_rate);
        let results = detector.detect_batch(&samples, sample_rate);

        assert!(!results.is_empty());

        for &(_, freq_hz, _confidence) in &results {
            assert!(
                freq_hz.abs() < 1.0,
                "Silence should give ~0 Hz, got {freq_hz} Hz"
            );
        }
    }
}

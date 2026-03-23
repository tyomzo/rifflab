use rifflab_core::audio::{AudioProcessor, ParamId};

/// Simple noise gate with threshold, attack, and release.
pub struct NoiseGate {
    threshold: f32,     // dBFS
    attack_ms: f32,
    release_ms: f32,
    envelope: f32,      // current envelope level (0.0–1.0)
    bypassed: bool,
}

impl NoiseGate {
    pub fn new() -> Self {
        Self {
            threshold: -40.0,
            attack_ms: 1.0,
            release_ms: 50.0,
            envelope: 0.0,
            bypassed: false,
        }
    }
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioProcessor for NoiseGate {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        let threshold_linear = 10.0f32.powf(self.threshold / 20.0);
        let attack_coeff = (-1.0 / (self.attack_ms * 0.001 * sample_rate as f32)).exp();
        let release_coeff = (-1.0 / (self.release_ms * 0.001 * sample_rate as f32)).exp();

        for sample in buffer.iter_mut() {
            let abs = sample.abs();
            if abs > self.envelope {
                self.envelope = attack_coeff * self.envelope + (1.0 - attack_coeff) * abs;
            } else {
                self.envelope = release_coeff * self.envelope + (1.0 - release_coeff) * abs;
            }

            if self.envelope < threshold_linear {
                *sample = 0.0;
            }
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.threshold = value,
            1 => self.attack_ms = value.max(0.1),
            2 => self.release_ms = value.max(1.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn name(&self) -> &str {
        "Noise Gate"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

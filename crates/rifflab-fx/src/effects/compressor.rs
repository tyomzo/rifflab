use rifflab_core::audio::{AudioProcessor, ParamId};

/// Dynamic range compressor.
pub struct Compressor {
    threshold: f32,     // dBFS
    ratio: f32,         // e.g., 4.0 = 4:1
    attack_ms: f32,
    release_ms: f32,
    makeup_gain: f32,   // dB
    envelope: f32,
    bypassed: bool,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            threshold: -20.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            makeup_gain: 0.0,
            envelope: 0.0,
            bypassed: false,
        }
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioProcessor for Compressor {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        let attack_coeff = (-1.0 / (self.attack_ms * 0.001 * sample_rate as f32)).exp();
        let release_coeff = (-1.0 / (self.release_ms * 0.001 * sample_rate as f32)).exp();
        let makeup_linear = 10.0f32.powf(self.makeup_gain / 20.0);
        let threshold_linear = 10.0f32.powf(self.threshold / 20.0);

        for sample in buffer.iter_mut() {
            let abs = sample.abs();

            // Envelope follower
            if abs > self.envelope {
                self.envelope = attack_coeff * self.envelope + (1.0 - attack_coeff) * abs;
            } else {
                self.envelope = release_coeff * self.envelope + (1.0 - release_coeff) * abs;
            }

            // Gain computation
            let gain = if self.envelope > threshold_linear {
                let over_db = 20.0 * (self.envelope / threshold_linear).log10();
                let compressed_over = over_db / self.ratio;
                let gain_reduction = over_db - compressed_over;
                10.0f32.powf(-gain_reduction / 20.0)
            } else {
                1.0
            };

            *sample *= gain * makeup_linear;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.threshold = value,
            1 => self.ratio = value.max(1.0),
            2 => self.attack_ms = value.max(0.1),
            3 => self.release_ms = value.max(1.0),
            4 => self.makeup_gain = value,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn name(&self) -> &str {
        "Compressor"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

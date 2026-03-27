use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Noise gate with smooth gain ramping and hysteresis.
///
/// Instead of hard on/off, the gate smoothly ramps gain between 0.0 and 1.0.
/// Hysteresis prevents rapid toggling when the signal hovers near the threshold:
/// the gate opens at `threshold` but doesn't close until `threshold - 6dB`.
pub struct NoiseGate {
    threshold: f32,     // dBFS
    attack_ms: f32,
    release_ms: f32,
    envelope: f32,      // current envelope level
    gate_gain: f32,     // current gate gain (0.0–1.0), smoothly ramped
    gate_open: bool,    // hysteresis state
    bypassed: bool,
}

impl NoiseGate {
    pub fn new() -> Self {
        Self {
            threshold: -40.0,
            attack_ms: 1.0,
            release_ms: 50.0,
            envelope: 0.0,
            gate_gain: 0.0,
            gate_open: false,
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
        // Close threshold is 6dB below open threshold (hysteresis)
        let close_linear = threshold_linear * 0.5;
        let attack_coeff = (-1.0 / (self.attack_ms * 0.001 * sample_rate as f32)).exp();
        let release_coeff = (-1.0 / (self.release_ms * 0.001 * sample_rate as f32)).exp();

        // Smooth gain ramp coefficients: open in ~1ms, close in ~5ms
        let open_smooth = (-1.0 / (0.001 * sample_rate as f32)).exp();
        let close_smooth = (-1.0 / (0.005 * sample_rate as f32)).exp();

        let frames = buffer.len() / 2;
        for frame in 0..frames {
            let li = frame * 2;
            let ri = frame * 2 + 1;
            let abs = buffer[li].abs().max(buffer[ri].abs());

            // Envelope follower
            if abs > self.envelope {
                self.envelope = attack_coeff * self.envelope + (1.0 - attack_coeff) * abs;
            } else {
                self.envelope = release_coeff * self.envelope + (1.0 - release_coeff) * abs;
            }

            // Hysteresis: open at threshold, close at threshold - 6dB
            if self.envelope >= threshold_linear {
                self.gate_open = true;
            } else if self.envelope < close_linear {
                self.gate_open = false;
            }

            // Smooth gain ramp toward target (1.0 = open, 0.0 = closed)
            let target = if self.gate_open { 1.0 } else { 0.0 };
            let coeff = if self.gate_open { open_smooth } else { close_smooth };
            self.gate_gain = coeff * self.gate_gain + (1.0 - coeff) * target;

            buffer[li] *= self.gate_gain;
            buffer[ri] *= self.gate_gain;
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
        self.gate_gain = 0.0;
        self.gate_open = false;
    }

    fn name(&self) -> &str {
        "Noise Gate"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

impl EffectDescriptor for NoiseGate {
    fn effect_type_id(&self) -> &str {
        "builtin:noise_gate"
    }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor {
                id: ParamId(0),
                name: "Threshold".into(),
                unit: "dB".into(),
                min: -80.0,
                max: 0.0,
                default: -40.0,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(1),
                name: "Attack".into(),
                unit: "ms".into(),
                min: 0.1,
                max: 100.0,
                default: 1.0,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(2),
                name: "Release".into(),
                unit: "ms".into(),
                min: 1.0,
                max: 1000.0,
                default: 50.0,
                step: None,
                kind: ParamKind::Float,
            },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.threshold,
            1 => self.attack_ms,
            2 => self.release_ms,
            _ => 0.0,
        }
    }
}

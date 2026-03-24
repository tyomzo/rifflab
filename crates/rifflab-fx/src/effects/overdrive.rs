use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Overdrive / distortion effect with selectable waveshaper.
pub struct Overdrive {
    drive: f32,         // 0.0–1.0
    tone: f32,          // 0.0–1.0 (low-pass filter cutoff)
    mix: f32,           // 0.0–1.0 (dry/wet)
    shaper: WaveShaperType,
    /// Simple one-pole low-pass state for tone control.
    lp_state: f32,
    bypassed: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum WaveShaperType {
    Tanh,
    HardClip,
    SoftClip,
}

impl Overdrive {
    pub fn new() -> Self {
        Self {
            drive: 0.5,
            tone: 0.5,
            mix: 1.0,
            shaper: WaveShaperType::Tanh,
            lp_state: 0.0,
            bypassed: false,
        }
    }

    fn shape(&self, x: f32) -> f32 {
        match self.shaper {
            WaveShaperType::Tanh => x.tanh(),
            WaveShaperType::HardClip => x.clamp(-1.0, 1.0),
            WaveShaperType::SoftClip => {
                if x.abs() < 1.0 {
                    x - (x * x * x) / 3.0
                } else {
                    x.signum() * 2.0 / 3.0
                }
            }
        }
    }
}

impl Default for Overdrive {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioProcessor for Overdrive {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let gain = 1.0 + self.drive * 30.0; // Drive range: 1x to 31x
        let tone_coeff = self.tone.clamp(0.01, 0.99);

        for sample in buffer.iter_mut() {
            let dry = *sample;
            let driven = self.shape(*sample * gain);

            // One-pole low-pass for tone control
            self.lp_state += tone_coeff * (driven - self.lp_state);
            let wet = self.lp_state;

            *sample = dry * (1.0 - self.mix) + wet * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.drive = value.clamp(0.0, 1.0),
            1 => self.tone = value.clamp(0.0, 1.0),
            2 => self.mix = value.clamp(0.0, 1.0),
            3 => {
                self.shaper = match value as u32 {
                    0 => WaveShaperType::Tanh,
                    1 => WaveShaperType::HardClip,
                    2 => WaveShaperType::SoftClip,
                    _ => WaveShaperType::Tanh,
                }
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.lp_state = 0.0;
    }

    fn name(&self) -> &str {
        "Overdrive"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

impl EffectDescriptor for Overdrive {
    fn effect_type_id(&self) -> &str {
        "builtin:overdrive"
    }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor {
                id: ParamId(0),
                name: "Drive".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(1),
                name: "Tone".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(2),
                name: "Mix".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(3),
                name: "Shaper Type".into(),
                unit: "".into(),
                min: 0.0,
                max: 2.0,
                default: 0.0,
                step: Some(1.0),
                kind: ParamKind::Enum(vec![
                    "Tanh".into(),
                    "HardClip".into(),
                    "SoftClip".into(),
                ]),
            },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.drive,
            1 => self.tone,
            2 => self.mix,
            3 => match self.shaper {
                WaveShaperType::Tanh => 0.0,
                WaveShaperType::HardClip => 1.0,
                WaveShaperType::SoftClip => 2.0,
            },
            _ => 0.0,
        }
    }
}

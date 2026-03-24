use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Cabinet simulation using biquad-based tone shaping.
///
/// Models the frequency response of a guitar/bass speaker cabinet
/// with a resonant lowpass filter and optional high-pass for tightness.
pub struct Cabinet {
    /// Cabinet type: 0=Guitar 1x12, 1=Guitar 4x12, 2=Bass 1x15.
    cab_type: u32,
    /// Tone control (0.0–1.0): low = dark, high = bright.
    tone: f32,
    /// Resonance at the cutoff frequency (0.0–1.0).
    resonance: f32,
    bypassed: bool,
    sample_rate: f32,
    // Biquad state (stereo)
    lp_b0: f32, lp_b1: f32, lp_b2: f32, lp_a1: f32, lp_a2: f32,
    lp_x1_l: f32, lp_x2_l: f32, lp_y1_l: f32, lp_y2_l: f32,
    lp_x1_r: f32, lp_x2_r: f32, lp_y1_r: f32, lp_y2_r: f32,
    dirty: bool,
}

impl Cabinet {
    pub fn new(sample_rate: u32) -> Self {
        let mut cab = Self {
            cab_type: 0,
            tone: 0.5,
            resonance: 0.3,
            bypassed: false,
            sample_rate: sample_rate as f32,
            lp_b0: 1.0, lp_b1: 0.0, lp_b2: 0.0, lp_a1: 0.0, lp_a2: 0.0,
            lp_x1_l: 0.0, lp_x2_l: 0.0, lp_y1_l: 0.0, lp_y2_l: 0.0,
            lp_x1_r: 0.0, lp_x2_r: 0.0, lp_y1_r: 0.0, lp_y2_r: 0.0,
            dirty: true,
        };
        cab.update_coefficients();
        cab
    }

    fn cutoff_for_type(&self) -> f32 {
        let base = match self.cab_type {
            0 => 4500.0, // 1x12 guitar — bright
            1 => 3500.0, // 4x12 guitar — darker, more body
            2 => 2500.0, // 1x15 bass — warm
            _ => 4000.0,
        };
        // Tone sweeps ±2000Hz around the base
        base + (self.tone - 0.5) * 4000.0
    }

    fn update_coefficients(&mut self) {
        let freq = self.cutoff_for_type().clamp(200.0, 18000.0);
        let q = 0.5 + self.resonance * 3.0; // Q range: 0.5 – 3.5
        let w0 = 2.0 * std::f32::consts::PI * freq / self.sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        // Lowpass biquad
        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        self.lp_b0 = b0 / a0;
        self.lp_b1 = b1 / a0;
        self.lp_b2 = b2 / a0;
        self.lp_a1 = a1 / a0;
        self.lp_a2 = a2 / a0;
        self.dirty = false;
    }

    #[inline]
    fn process_sample_l(&mut self, input: f32) -> f32 {
        let output = self.lp_b0 * input + self.lp_b1 * self.lp_x1_l + self.lp_b2 * self.lp_x2_l
            - self.lp_a1 * self.lp_y1_l - self.lp_a2 * self.lp_y2_l;
        self.lp_x2_l = self.lp_x1_l;
        self.lp_x1_l = input;
        self.lp_y2_l = self.lp_y1_l;
        self.lp_y1_l = output;
        output
    }

    #[inline]
    fn process_sample_r(&mut self, input: f32) -> f32 {
        let output = self.lp_b0 * input + self.lp_b1 * self.lp_x1_r + self.lp_b2 * self.lp_x2_r
            - self.lp_a1 * self.lp_y1_r - self.lp_a2 * self.lp_y2_r;
        self.lp_x2_r = self.lp_x1_r;
        self.lp_x1_r = input;
        self.lp_y2_r = self.lp_y1_r;
        self.lp_y1_r = output;
        output
    }
}

impl AudioProcessor for Cabinet {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        if self.dirty {
            self.update_coefficients();
        }
        let frames = buffer.len() / 2;
        for frame in 0..frames {
            buffer[frame * 2] = self.process_sample_l(buffer[frame * 2]);
            buffer[frame * 2 + 1] = self.process_sample_r(buffer[frame * 2 + 1]);
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => { self.cab_type = (value as u32).clamp(0, 2); self.dirty = true; }
            1 => { self.tone = value.clamp(0.0, 1.0); self.dirty = true; }
            2 => { self.resonance = value.clamp(0.0, 1.0); self.dirty = true; }
            3 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.lp_x1_l = 0.0; self.lp_x2_l = 0.0; self.lp_y1_l = 0.0; self.lp_y2_l = 0.0;
        self.lp_x1_r = 0.0; self.lp_x2_r = 0.0; self.lp_y1_r = 0.0; self.lp_y2_r = 0.0;
    }

    fn name(&self) -> &str { "Cabinet" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for Cabinet {
    fn effect_type_id(&self) -> &str { "builtin:cabinet" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor {
                id: ParamId(0), name: "Type".into(), unit: "".into(),
                min: 0.0, max: 2.0, default: 0.0, step: Some(1.0),
                kind: ParamKind::Enum(vec!["Guitar 1x12".into(), "Guitar 4x12".into(), "Bass 1x15".into()]),
            },
            ParamDescriptor { id: ParamId(1), name: "Tone".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Resonance".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.3, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(3), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.cab_type as f32,
            1 => self.tone,
            2 => self.resonance,
            3 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

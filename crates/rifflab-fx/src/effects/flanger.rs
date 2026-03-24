use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Flanger effect: LFO-modulated very short delay (0.1–5ms) with feedback.
pub struct Flanger {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    lfo_phase: f32,
    /// LFO rate in Hz (0.05–2.0).
    rate: f32,
    /// Modulation depth in ms (0.0–5.0).
    depth: f32,
    /// Feedback (-0.95–0.95).
    feedback: f32,
    /// Dry/wet mix (0.0–1.0).
    mix: f32,
    bypassed: bool,
    sample_rate: u32,
}

const MAX_DELAY_SAMPLES: usize = 512; // ~10ms at 48kHz

impl Flanger {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            buffer_l: vec![0.0; MAX_DELAY_SAMPLES],
            buffer_r: vec![0.0; MAX_DELAY_SAMPLES],
            write_pos: 0,
            lfo_phase: 0.0,
            rate: 0.3,
            depth: 2.0,
            feedback: 0.5,
            mix: 0.5,
            bypassed: false,
            sample_rate,
        }
    }

    fn read_interpolated(buffer: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
        let len = buffer.len();
        let pos = write_pos as f32 - delay_samples;
        let pos = if pos < 0.0 { pos + len as f32 } else { pos };
        let idx0 = pos.floor() as usize % len;
        let idx1 = (idx0 + 1) % len;
        let frac = pos - pos.floor();
        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }
}

impl AudioProcessor for Flanger {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let frames = buffer.len() / 2;
        let buf_len = self.buffer_l.len();
        let base_delay = 1.0 * self.sample_rate as f32 / 1000.0; // 1ms center
        let depth_samples = self.depth * self.sample_rate as f32 / 1000.0;
        let lfo_inc = self.rate / self.sample_rate as f32;

        for frame in 0..frames {
            let in_l = buffer[frame * 2];
            let in_r = buffer[frame * 2 + 1];

            let lfo_l = (self.lfo_phase * std::f32::consts::TAU).sin();
            let lfo_r = ((self.lfo_phase + 0.5) * std::f32::consts::TAU).sin(); // 180° for stereo width

            let delay_l = (base_delay + lfo_l * depth_samples).max(1.0);
            let delay_r = (base_delay + lfo_r * depth_samples).max(1.0);

            let wet_l = Self::read_interpolated(&self.buffer_l, self.write_pos, delay_l);
            let wet_r = Self::read_interpolated(&self.buffer_r, self.write_pos, delay_r);

            self.buffer_l[self.write_pos] = in_l + wet_l * self.feedback;
            self.buffer_r[self.write_pos] = in_r + wet_r * self.feedback;
            self.write_pos = (self.write_pos + 1) % buf_len;

            self.lfo_phase += lfo_inc;
            if self.lfo_phase >= 1.0 { self.lfo_phase -= 1.0; }

            buffer[frame * 2] = in_l * (1.0 - self.mix) + wet_l * self.mix;
            buffer[frame * 2 + 1] = in_r * (1.0 - self.mix) + wet_r * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.rate = value.clamp(0.05, 2.0),
            1 => self.depth = value.clamp(0.0, 5.0),
            2 => self.feedback = value.clamp(-0.95, 0.95),
            3 => self.mix = value.clamp(0.0, 1.0),
            4 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.buffer_l.fill(0.0);
        self.buffer_r.fill(0.0);
        self.write_pos = 0;
        self.lfo_phase = 0.0;
    }

    fn name(&self) -> &str { "Flanger" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for Flanger {
    fn effect_type_id(&self) -> &str { "builtin:flanger" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor { id: ParamId(0), name: "Rate".into(), unit: "Hz".into(), min: 0.05, max: 2.0, default: 0.3, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(1), name: "Depth".into(), unit: "ms".into(), min: 0.0, max: 5.0, default: 2.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Feedback".into(), unit: "".into(), min: -0.95, max: 0.95, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(3), name: "Mix".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(4), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.rate,
            1 => self.depth,
            2 => self.feedback,
            3 => self.mix,
            4 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Chorus effect: LFO-modulated delay line (1–20ms range).
pub struct Chorus {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    /// LFO phase (0.0–1.0).
    lfo_phase: f32,
    /// LFO rate in Hz (0.1–5.0).
    rate: f32,
    /// Modulation depth in ms (0.0–10.0).
    depth: f32,
    /// Dry/wet mix (0.0–1.0).
    mix: f32,
    bypassed: bool,
    sample_rate: u32,
}

const MAX_DELAY_SAMPLES: usize = 2048; // ~42ms at 48kHz

impl Chorus {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            buffer_l: vec![0.0; MAX_DELAY_SAMPLES],
            buffer_r: vec![0.0; MAX_DELAY_SAMPLES],
            write_pos: 0,
            lfo_phase: 0.0,
            rate: 1.0,
            depth: 3.0,
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

impl AudioProcessor for Chorus {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let frames = buffer.len() / 2;
        let buf_len = self.buffer_l.len();
        let base_delay = 7.0 * self.sample_rate as f32 / 1000.0; // 7ms center
        let depth_samples = self.depth * self.sample_rate as f32 / 1000.0;
        let lfo_inc = self.rate / self.sample_rate as f32;

        for frame in 0..frames {
            let in_l = buffer[frame * 2];
            let in_r = buffer[frame * 2 + 1];

            // LFO (sine, stereo offset for width)
            let lfo_l = (self.lfo_phase * std::f32::consts::TAU).sin();
            let lfo_r = ((self.lfo_phase + 0.25) * std::f32::consts::TAU).sin();

            let delay_l = base_delay + lfo_l * depth_samples;
            let delay_r = base_delay + lfo_r * depth_samples;

            let wet_l = Self::read_interpolated(&self.buffer_l, self.write_pos, delay_l.max(1.0));
            let wet_r = Self::read_interpolated(&self.buffer_r, self.write_pos, delay_r.max(1.0));

            self.buffer_l[self.write_pos] = in_l;
            self.buffer_r[self.write_pos] = in_r;
            self.write_pos = (self.write_pos + 1) % buf_len;

            self.lfo_phase += lfo_inc;
            if self.lfo_phase >= 1.0 { self.lfo_phase -= 1.0; }

            buffer[frame * 2] = in_l * (1.0 - self.mix) + wet_l * self.mix;
            buffer[frame * 2 + 1] = in_r * (1.0 - self.mix) + wet_r * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.rate = value.clamp(0.1, 5.0),
            1 => self.depth = value.clamp(0.0, 10.0),
            2 => self.mix = value.clamp(0.0, 1.0),
            3 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.buffer_l.fill(0.0);
        self.buffer_r.fill(0.0);
        self.write_pos = 0;
        self.lfo_phase = 0.0;
    }

    fn name(&self) -> &str { "Chorus" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for Chorus {
    fn effect_type_id(&self) -> &str { "builtin:chorus" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor { id: ParamId(0), name: "Rate".into(), unit: "Hz".into(), min: 0.1, max: 5.0, default: 1.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(1), name: "Depth".into(), unit: "ms".into(), min: 0.0, max: 10.0, default: 3.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Mix".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(3), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.rate,
            1 => self.depth,
            2 => self.mix,
            3 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

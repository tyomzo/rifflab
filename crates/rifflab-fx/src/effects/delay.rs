use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Stereo delay effect with feedback.
pub struct Delay {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    delay_samples: usize,
    /// Delay time in ms (1–2000).
    time_ms: f32,
    /// Feedback (0.0–0.95).
    feedback: f32,
    /// Dry/wet mix (0.0–1.0).
    mix: f32,
    bypassed: bool,
    sample_rate: u32,
}

const MAX_DELAY_MS: f32 = 2000.0;

impl Delay {
    pub fn new(sample_rate: u32) -> Self {
        let max_samples = (sample_rate as f32 * MAX_DELAY_MS / 1000.0) as usize + 1;
        let time_ms = 300.0;
        let delay_samples = (time_ms * sample_rate as f32 / 1000.0) as usize;
        Self {
            buffer_l: vec![0.0; max_samples],
            buffer_r: vec![0.0; max_samples],
            write_pos: 0,
            delay_samples,
            time_ms,
            feedback: 0.4,
            mix: 0.3,
            bypassed: false,
            sample_rate,
        }
    }

    fn update_delay_samples(&mut self) {
        self.delay_samples = (self.time_ms * self.sample_rate as f32 / 1000.0) as usize;
        self.delay_samples = self.delay_samples.min(self.buffer_l.len() - 1);
    }
}

impl AudioProcessor for Delay {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let frames = buffer.len() / 2;
        let buf_len = self.buffer_l.len();
        for frame in 0..frames {
            let in_l = buffer[frame * 2];
            let in_r = buffer[frame * 2 + 1];

            let read_pos = (self.write_pos + buf_len - self.delay_samples) % buf_len;
            let delayed_l = self.buffer_l[read_pos];
            let delayed_r = self.buffer_r[read_pos];

            self.buffer_l[self.write_pos] = in_l + delayed_l * self.feedback;
            self.buffer_r[self.write_pos] = in_r + delayed_r * self.feedback;

            self.write_pos = (self.write_pos + 1) % buf_len;

            buffer[frame * 2] = in_l * (1.0 - self.mix) + delayed_l * self.mix;
            buffer[frame * 2 + 1] = in_r * (1.0 - self.mix) + delayed_r * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => { self.time_ms = value; self.update_delay_samples(); }
            1 => self.feedback = value.clamp(0.0, 0.95),
            2 => self.mix = value.clamp(0.0, 1.0),
            3 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.buffer_l.fill(0.0);
        self.buffer_r.fill(0.0);
        self.write_pos = 0;
    }

    fn name(&self) -> &str { "Delay" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for Delay {
    fn effect_type_id(&self) -> &str { "builtin:delay" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor { id: ParamId(0), name: "Time".into(), unit: "ms".into(), min: 1.0, max: MAX_DELAY_MS, default: 300.0, step: Some(1.0), kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(1), name: "Feedback".into(), unit: "".into(), min: 0.0, max: 0.95, default: 0.4, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Mix".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.3, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(3), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.time_ms,
            1 => self.feedback,
            2 => self.mix,
            3 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

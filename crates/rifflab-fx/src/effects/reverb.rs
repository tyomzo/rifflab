use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Simple Schroeder reverb with 4 comb filters and 2 allpass filters.
/// Separate filter state per channel for correct stereo processing.
pub struct Reverb {
    comb_l: [CombFilter; 4],
    comb_r: [CombFilter; 4],
    allpass_l: [AllpassFilter; 2],
    allpass_r: [AllpassFilter; 2],
    room_size: f32,
    damping: f32,
    wet: f32,
    dry: f32,
    bypassed: bool,
}

struct CombFilter {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
    damp: f32,
    damp_state: f32,
}

impl CombFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
            feedback: 0.7,
            damp: 0.3,
            damp_state: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.damp_state = output * (1.0 - self.damp) + self.damp_state * self.damp;
        self.buffer[self.index] = input + self.damp_state * self.feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.damp_state = 0.0;
    }
}

struct AllpassFilter {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
}

impl AllpassFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
            feedback: 0.5,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = -input + buffered;
        self.buffer[self.index] = input + buffered * self.feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
    }
}

impl Reverb {
    pub fn new(sample_rate: u32) -> Self {
        let scale = sample_rate as f32 / 44100.0;
        // Slightly offset R channel delay lengths for stereo width
        let stereo_spread = 23;
        Self {
            comb_l: [
                CombFilter::new((1116.0 * scale) as usize),
                CombFilter::new((1188.0 * scale) as usize),
                CombFilter::new((1277.0 * scale) as usize),
                CombFilter::new((1356.0 * scale) as usize),
            ],
            comb_r: [
                CombFilter::new(((1116 + stereo_spread) as f32 * scale) as usize),
                CombFilter::new(((1188 + stereo_spread) as f32 * scale) as usize),
                CombFilter::new(((1277 + stereo_spread) as f32 * scale) as usize),
                CombFilter::new(((1356 + stereo_spread) as f32 * scale) as usize),
            ],
            allpass_l: [
                AllpassFilter::new((556.0 * scale) as usize),
                AllpassFilter::new((441.0 * scale) as usize),
            ],
            allpass_r: [
                AllpassFilter::new(((556 + stereo_spread) as f32 * scale) as usize),
                AllpassFilter::new(((441 + stereo_spread) as f32 * scale) as usize),
            ],
            room_size: 0.5,
            damping: 0.3,
            wet: 0.3,
            dry: 0.7,
            bypassed: false,
        }
    }

    fn update_params(&mut self) {
        let feedback = 0.28 + self.room_size * 0.7;
        for comb in self.comb_l.iter_mut().chain(self.comb_r.iter_mut()) {
            comb.feedback = feedback;
            comb.damp = self.damping;
        }
    }
}

impl AudioProcessor for Reverb {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        self.update_params();

        // Process interleaved stereo in frame pairs
        let frames = buffer.len() / 2;
        for frame in 0..frames {
            let li = frame * 2;
            let ri = frame * 2 + 1;

            let in_l = buffer[li];
            let in_r = buffer[ri];

            let mut comb_sum_l = 0.0;
            let mut comb_sum_r = 0.0;
            for comb in &mut self.comb_l {
                comb_sum_l += comb.process(in_l);
            }
            for comb in &mut self.comb_r {
                comb_sum_r += comb.process(in_r);
            }

            let mut out_l = comb_sum_l;
            let mut out_r = comb_sum_r;
            for ap in &mut self.allpass_l {
                out_l = ap.process(out_l);
            }
            for ap in &mut self.allpass_r {
                out_r = ap.process(out_r);
            }

            buffer[li] = in_l * self.dry + out_l * self.wet;
            buffer[ri] = in_r * self.dry + out_r * self.wet;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.room_size = value.clamp(0.0, 1.0),
            1 => self.damping = value.clamp(0.0, 1.0),
            2 => self.wet = value.clamp(0.0, 1.0),
            3 => self.dry = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn reset(&mut self) {
        for comb in self.comb_l.iter_mut().chain(self.comb_r.iter_mut()) {
            comb.reset();
        }
        for ap in self.allpass_l.iter_mut().chain(self.allpass_r.iter_mut()) {
            ap.reset();
        }
    }

    fn name(&self) -> &str {
        "Reverb"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

impl EffectDescriptor for Reverb {
    fn effect_type_id(&self) -> &str {
        "builtin:reverb"
    }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor {
                id: ParamId(0),
                name: "Room Size".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(1),
                name: "Damping".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 0.3,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(2),
                name: "Wet".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 0.3,
                step: None,
                kind: ParamKind::Float,
            },
            ParamDescriptor {
                id: ParamId(3),
                name: "Dry".into(),
                unit: "".into(),
                min: 0.0,
                max: 1.0,
                default: 0.7,
                step: None,
                kind: ParamKind::Float,
            },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.room_size,
            1 => self.damping,
            2 => self.wet,
            3 => self.dry,
            _ => 0.0,
        }
    }
}

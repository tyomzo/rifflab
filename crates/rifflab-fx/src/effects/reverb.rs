use rifflab_core::audio::{AudioProcessor, ParamId};

/// Simple Schroeder reverb with 4 comb filters and 2 allpass filters.
pub struct Reverb {
    comb_filters: [CombFilter; 4],
    allpass_filters: [AllpassFilter; 2],
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
        Self {
            comb_filters: [
                CombFilter::new((1116.0 * scale) as usize),
                CombFilter::new((1188.0 * scale) as usize),
                CombFilter::new((1277.0 * scale) as usize),
                CombFilter::new((1356.0 * scale) as usize),
            ],
            allpass_filters: [
                AllpassFilter::new((556.0 * scale) as usize),
                AllpassFilter::new((441.0 * scale) as usize),
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
        for comb in &mut self.comb_filters {
            comb.feedback = feedback;
            comb.damp = self.damping;
        }
    }
}

impl AudioProcessor for Reverb {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        self.update_params();

        for sample in buffer.iter_mut() {
            let input = *sample;
            let mut comb_sum = 0.0;

            for comb in &mut self.comb_filters {
                comb_sum += comb.process(input);
            }

            let mut output = comb_sum;
            for ap in &mut self.allpass_filters {
                output = ap.process(output);
            }

            *sample = input * self.dry + output * self.wet;
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
        for comb in &mut self.comb_filters {
            comb.reset();
        }
        for ap in &mut self.allpass_filters {
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

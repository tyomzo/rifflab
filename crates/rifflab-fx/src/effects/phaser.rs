use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Phaser effect: LFO-modulated allpass cascade.
pub struct Phaser {
    allpass_l: [AllpassStage; 8],
    allpass_r: [AllpassStage; 8],
    lfo_phase: f32,
    /// LFO rate in Hz (0.05–4.0).
    rate: f32,
    /// Modulation depth (0.0–1.0).
    depth: f32,
    /// Number of active allpass stages (2–8).
    stages: u32,
    /// Feedback (-0.95–0.95).
    feedback: f32,
    /// Dry/wet mix (0.0–1.0).
    mix: f32,
    /// Previous output for feedback path.
    fb_l: f32,
    fb_r: f32,
    bypassed: bool,
    sample_rate: u32,
}

#[derive(Clone, Copy, Default)]
struct AllpassStage {
    y1: f32,
}

impl AllpassStage {
    fn process(&mut self, input: f32, coeff: f32) -> f32 {
        let output = -coeff * input + self.y1;
        self.y1 = coeff * output + input;
        output
    }

    fn reset(&mut self) {
        self.y1 = 0.0;
    }
}

impl Phaser {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            allpass_l: [AllpassStage::default(); 8],
            allpass_r: [AllpassStage::default(); 8],
            lfo_phase: 0.0,
            rate: 0.5,
            depth: 0.7,
            stages: 4,
            feedback: 0.3,
            mix: 0.5,
            fb_l: 0.0,
            fb_r: 0.0,
            bypassed: false,
            sample_rate,
        }
    }
}

impl AudioProcessor for Phaser {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let frames = buffer.len() / 2;
        let lfo_inc = self.rate / self.sample_rate as f32;
        let num_stages = self.stages.clamp(2, 8) as usize;
        // Frequency range for allpass sweep: 200Hz – 5000Hz
        let min_freq = 200.0f32;
        let max_freq = 5000.0f32;

        for frame in 0..frames {
            let in_l = buffer[frame * 2];
            let in_r = buffer[frame * 2 + 1];

            // LFO
            let lfo = (self.lfo_phase * std::f32::consts::TAU).sin() * 0.5 + 0.5; // 0..1
            let sweep = min_freq + (max_freq - min_freq) * lfo * self.depth;

            // First-order allpass coefficient from frequency
            let coeff = (std::f32::consts::PI * sweep / self.sample_rate as f32).tan();
            let coeff = (coeff - 1.0) / (coeff + 1.0);

            // Process through allpass cascade with feedback
            let mut sig_l = in_l + self.fb_l * self.feedback;
            let mut sig_r = in_r + self.fb_r * self.feedback;

            for i in 0..num_stages {
                sig_l = self.allpass_l[i].process(sig_l, coeff);
                sig_r = self.allpass_r[i].process(sig_r, coeff);
            }

            self.fb_l = sig_l;
            self.fb_r = sig_r;

            self.lfo_phase += lfo_inc;
            if self.lfo_phase >= 1.0 { self.lfo_phase -= 1.0; }

            buffer[frame * 2] = in_l * (1.0 - self.mix) + sig_l * self.mix;
            buffer[frame * 2 + 1] = in_r * (1.0 - self.mix) + sig_r * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.rate = value.clamp(0.05, 4.0),
            1 => self.depth = value.clamp(0.0, 1.0),
            2 => self.stages = (value as u32).clamp(2, 8),
            3 => self.feedback = value.clamp(-0.95, 0.95),
            4 => self.mix = value.clamp(0.0, 1.0),
            5 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        for s in &mut self.allpass_l { s.reset(); }
        for s in &mut self.allpass_r { s.reset(); }
        self.fb_l = 0.0;
        self.fb_r = 0.0;
        self.lfo_phase = 0.0;
    }

    fn name(&self) -> &str { "Phaser" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for Phaser {
    fn effect_type_id(&self) -> &str { "builtin:phaser" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor { id: ParamId(0), name: "Rate".into(), unit: "Hz".into(), min: 0.05, max: 4.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(1), name: "Depth".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.7, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Stages".into(), unit: "".into(), min: 2.0, max: 8.0, default: 4.0, step: Some(1.0), kind: ParamKind::Int },
            ParamDescriptor { id: ParamId(3), name: "Feedback".into(), unit: "".into(), min: -0.95, max: 0.95, default: 0.3, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(4), name: "Mix".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(5), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.rate,
            1 => self.depth,
            2 => self.stages as f32,
            3 => self.feedback,
            4 => self.mix,
            5 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

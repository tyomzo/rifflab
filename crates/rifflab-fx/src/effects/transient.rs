use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Transient shaper: controls the attack and sustain of a signal independently.
///
/// Uses an envelope follower to detect the transient (attack) portion of the signal
/// and applies separate gain to attack and sustain phases. Useful for:
/// - Softening pick attacks (negative attack = rounder, more bowed/plucked feel)
/// - Shortening sustain (negative sustain = faster decay, pizzicato-like)
/// - Enhancing punch (positive attack = more transient snap)
pub struct TransientShaper {
    /// Attack gain in dB (-24 to +24). Negative = softer attack.
    attack_db: f32,
    /// Sustain gain in dB (-24 to +24). Negative = shorter sustain.
    sustain_db: f32,
    /// Detection sensitivity (attack time of the envelope in ms).
    speed_ms: f32,
    /// Dry/wet mix (0.0–1.0).
    mix: f32,
    bypassed: bool,
    // Internal state (stereo)
    env_fast_l: f32,
    env_fast_r: f32,
    env_slow_l: f32,
    env_slow_r: f32,
    sample_rate: f32,
}

impl TransientShaper {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            attack_db: 0.0,
            sustain_db: 0.0,
            speed_ms: 10.0,
            mix: 1.0,
            bypassed: false,
            env_fast_l: 0.0,
            env_fast_r: 0.0,
            env_slow_l: 0.0,
            env_slow_r: 0.0,
            sample_rate: sample_rate as f32,
        }
    }
}

impl AudioProcessor for TransientShaper {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        self.sample_rate = sample_rate as f32;

        let fast_attack = (-1.0 / (0.001 * self.sample_rate)).exp(); // 1ms
        let fast_release = (-1.0 / (self.speed_ms as f64 * 0.001 * self.sample_rate as f64)).exp() as f32;
        let slow_attack = (-1.0 / (self.speed_ms as f64 * 0.005 * self.sample_rate as f64)).exp() as f32;
        let slow_release = (-1.0 / (self.speed_ms as f64 * 0.050 * self.sample_rate as f64)).exp() as f32;

        let attack_gain = 10.0f32.powf(self.attack_db / 20.0);
        let sustain_gain = 10.0f32.powf(self.sustain_db / 20.0);

        let frames = buffer.len() / 2;
        for i in 0..frames {
            let l = buffer[i * 2];
            let r = buffer[i * 2 + 1];
            let abs_l = l.abs();
            let abs_r = r.abs();

            // Fast envelope (tracks transients)
            let coeff_fl = if abs_l > self.env_fast_l { fast_attack } else { fast_release };
            self.env_fast_l = coeff_fl * self.env_fast_l + (1.0 - coeff_fl) * abs_l;

            let coeff_fr = if abs_r > self.env_fast_r { fast_attack } else { fast_release };
            self.env_fast_r = coeff_fr * self.env_fast_r + (1.0 - coeff_fr) * abs_r;

            // Slow envelope (tracks sustain)
            let coeff_sl = if abs_l > self.env_slow_l { slow_attack } else { slow_release };
            self.env_slow_l = coeff_sl * self.env_slow_l + (1.0 - coeff_sl) * abs_l;

            let coeff_sr = if abs_r > self.env_slow_r { slow_attack } else { slow_release };
            self.env_slow_r = coeff_sr * self.env_slow_r + (1.0 - coeff_sr) * abs_r;

            // Transient detector: fast - slow (positive during attack, ~0 during sustain)
            let transient_l = (self.env_fast_l - self.env_slow_l).max(0.0);
            let transient_r = (self.env_fast_r - self.env_slow_r).max(0.0);

            // Normalize transient to 0..1 range
            let trans_norm_l = if self.env_slow_l > 1e-8 { (transient_l / self.env_slow_l).min(1.0) } else { 0.0 };
            let trans_norm_r = if self.env_slow_r > 1e-8 { (transient_r / self.env_slow_r).min(1.0) } else { 0.0 };

            // Compute per-sample gain: blend between attack_gain and sustain_gain
            let gain_l = trans_norm_l * attack_gain + (1.0 - trans_norm_l) * sustain_gain;
            let gain_r = trans_norm_r * attack_gain + (1.0 - trans_norm_r) * sustain_gain;

            // Apply with mix
            buffer[i * 2] = l * (1.0 - self.mix) + l * gain_l * self.mix;
            buffer[i * 2 + 1] = r * (1.0 - self.mix) + r * gain_r * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.attack_db = value.clamp(-24.0, 24.0),
            1 => self.sustain_db = value.clamp(-24.0, 24.0),
            2 => self.speed_ms = value.clamp(1.0, 100.0),
            3 => self.mix = value.clamp(0.0, 1.0),
            4 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.env_fast_l = 0.0; self.env_fast_r = 0.0;
        self.env_slow_l = 0.0; self.env_slow_r = 0.0;
    }

    fn name(&self) -> &str { "Transient Shaper" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for TransientShaper {
    fn effect_type_id(&self) -> &str { "builtin:transient" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![
            ParamDescriptor { id: ParamId(0), name: "Attack".into(), unit: "dB".into(), min: -24.0, max: 24.0, default: 0.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(1), name: "Sustain".into(), unit: "dB".into(), min: -24.0, max: 24.0, default: 0.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Speed".into(), unit: "ms".into(), min: 1.0, max: 100.0, default: 10.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(3), name: "Mix".into(), unit: "".into(), min: 0.0, max: 1.0, default: 1.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(4), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.attack_db,
            1 => self.sustain_db,
            2 => self.speed_ms,
            3 => self.mix,
            4 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

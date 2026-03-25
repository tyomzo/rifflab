use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

const MIN_XOVER_GAP_HZ: f32 = 100.0;
const MAX_BUF: usize = 4096; // max interleaved stereo samples (2048 frames)

// ─── Stereo Biquad ───────────────────────────────────────────────────────────

#[derive(Clone)]
struct LrBiquad {
    b0: f64, b1: f64, b2: f64,
    a1: f64, a2: f64,
    x1_l: f64, x2_l: f64, y1_l: f64, y2_l: f64,
    x1_r: f64, x2_r: f64, y1_r: f64, y2_r: f64,
}

impl LrBiquad {
    fn new() -> Self {
        Self {
            b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0,
            x1_l: 0.0, x2_l: 0.0, y1_l: 0.0, y2_l: 0.0,
            x1_r: 0.0, x2_r: 0.0, y1_r: 0.0, y2_r: 0.0,
        }
    }

    #[inline]
    fn process_l(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1_l + self.b2 * self.x2_l
              - self.a1 * self.y1_l - self.a2 * self.y2_l;
        self.x2_l = self.x1_l; self.x1_l = x;
        self.y2_l = self.y1_l; self.y1_l = y;
        y
    }

    #[inline]
    fn process_r(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1_r + self.b2 * self.x2_r
              - self.a1 * self.y1_r - self.a2 * self.y2_r;
        self.x2_r = self.x1_r; self.x1_r = x;
        self.y2_r = self.y1_r; self.y1_r = y;
        y
    }

    fn reset(&mut self) {
        self.x1_l = 0.0; self.x2_l = 0.0; self.y1_l = 0.0; self.y2_l = 0.0;
        self.x1_r = 0.0; self.x2_r = 0.0; self.y1_r = 0.0; self.y2_r = 0.0;
    }
}

// ─── LR4 Crossover ──────────────────────────────────────────────────────────

/// Linkwitz-Riley 4th-order crossover: splits signal into LP + HP bands.
/// Two cascaded 2nd-order Butterworth filters per path.
struct LR4Crossover {
    lp: [LrBiquad; 2],
    hp: [LrBiquad; 2],
}

impl LR4Crossover {
    fn new() -> Self {
        Self {
            lp: [LrBiquad::new(), LrBiquad::new()],
            hp: [LrBiquad::new(), LrBiquad::new()],
        }
    }

    fn set_frequency(&mut self, freq: f64, sample_rate: f64) {
        let w0 = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = std::f64::consts::FRAC_1_SQRT_2; // 0.7071
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;

        // Lowpass Butterworth 2nd-order
        let lp_b0 = ((1.0 - cos_w0) / 2.0) / a0;
        let lp_b1 = (1.0 - cos_w0) / a0;
        let lp_b2 = lp_b0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha) / a0;

        for bq in &mut self.lp {
            bq.b0 = lp_b0; bq.b1 = lp_b1; bq.b2 = lp_b2;
            bq.a1 = a1; bq.a2 = a2;
        }

        // Highpass Butterworth 2nd-order
        let hp_b0 = ((1.0 + cos_w0) / 2.0) / a0;
        let hp_b1 = (-(1.0 + cos_w0)) / a0;
        let hp_b2 = hp_b0;

        for bq in &mut self.hp {
            bq.b0 = hp_b0; bq.b1 = hp_b1; bq.b2 = hp_b2;
            bq.a1 = a1; bq.a2 = a2;
        }
    }

    #[inline]
    fn process_lp(&mut self, l: f32, r: f32) -> (f32, f32) {
        let l1 = self.lp[0].process_l(l as f64);
        let r1 = self.lp[0].process_r(r as f64);
        (self.lp[1].process_l(l1) as f32, self.lp[1].process_r(r1) as f32)
    }

    #[inline]
    fn process_hp(&mut self, l: f32, r: f32) -> (f32, f32) {
        let l1 = self.hp[0].process_l(l as f64);
        let r1 = self.hp[0].process_r(r as f64);
        (self.hp[1].process_l(l1) as f32, self.hp[1].process_r(r1) as f32)
    }

    fn reset(&mut self) {
        for bq in &mut self.lp { bq.reset(); }
        for bq in &mut self.hp { bq.reset(); }
    }
}

// ─── Per-Band Processors ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum BandMode { Clean = 0, Compressed = 1, Drive = 2 }

impl BandMode {
    fn from_f32(v: f32) -> Self {
        match v.round() as u32 { 1 => Self::Compressed, 2 => Self::Drive, _ => Self::Clean }
    }
}

/// Lightweight per-band compressor. Fixed attack/release/ratio.
#[derive(Clone)]
struct BandCompressor {
    envelope_l: f32,
    envelope_r: f32,
}

impl BandCompressor {
    fn new() -> Self { Self { envelope_l: 0.0, envelope_r: 0.0 } }

    /// Process stereo buffer in-place. `amount` controls threshold: 0→-10dB, 1→-50dB.
    fn process(&mut self, buf: &mut [f32], amount: f32, sample_rate: f32) {
        let threshold_db = -10.0 - amount * 40.0;
        let threshold = 10.0f32.powf(threshold_db / 20.0);
        let ratio = 4.0f32;
        let attack = (-1.0 / (0.005 * sample_rate)).exp();
        let release = (-1.0 / (0.050 * sample_rate)).exp();

        let frames = buf.len() / 2;
        for i in 0..frames {
            let l = buf[i * 2];
            let r = buf[i * 2 + 1];

            // Envelope follower
            let abs_l = l.abs();
            let abs_r = r.abs();
            let coeff_l = if abs_l > self.envelope_l { attack } else { release };
            let coeff_r = if abs_r > self.envelope_r { attack } else { release };
            self.envelope_l = coeff_l * self.envelope_l + (1.0 - coeff_l) * abs_l;
            self.envelope_r = coeff_r * self.envelope_r + (1.0 - coeff_r) * abs_r;

            // Gain reduction
            let gain_l = if self.envelope_l > threshold {
                let over_db = 20.0 * (self.envelope_l / threshold).log10();
                let reduced = over_db * (1.0 - 1.0 / ratio);
                10.0f32.powf(-reduced / 20.0)
            } else { 1.0 };

            let gain_r = if self.envelope_r > threshold {
                let over_db = 20.0 * (self.envelope_r / threshold).log10();
                let reduced = over_db * (1.0 - 1.0 / ratio);
                10.0f32.powf(-reduced / 20.0)
            } else { 1.0 };

            buf[i * 2] = l * gain_l;
            buf[i * 2 + 1] = r * gain_r;
        }
    }
}

/// Lightweight per-band drive (tanh waveshaper + tone LP).
#[derive(Clone)]
struct BandDrive {
    lp_l: f32,
    lp_r: f32,
}

impl BandDrive {
    fn new() -> Self { Self { lp_l: 0.0, lp_r: 0.0 } }

    /// Process stereo buffer in-place. `amount` controls drive: 0→1x, 1→31x gain.
    fn process(&mut self, buf: &mut [f32], amount: f32, sample_rate: f32) {
        let gain = 1.0 + amount * 30.0;
        // Tone LP at ~4kHz to tame harsh highs from distortion
        let tone_freq = 4000.0f32;
        let rc = 1.0 / (2.0 * std::f32::consts::PI * tone_freq);
        let dt = 1.0 / sample_rate;
        let alpha = dt / (rc + dt);

        let frames = buf.len() / 2;
        for i in 0..frames {
            let l = (buf[i * 2] * gain).tanh();
            let r = (buf[i * 2 + 1] * gain).tanh();
            self.lp_l += alpha * (l - self.lp_l);
            self.lp_r += alpha * (r - self.lp_r);
            buf[i * 2] = self.lp_l;
            buf[i * 2 + 1] = self.lp_r;
        }
    }
}

// ─── Multiband Effect ────────────────────────────────────────────────────────

/// 3-band multiband processor with Linkwitz-Riley crossovers.
pub struct Multiband {
    xover_low_mid: LR4Crossover,
    xover_mid_high: LR4Crossover,
    freq_low_mid: f32,
    freq_mid_high: f32,
    band_mode: [BandMode; 3],
    band_gain_db: [f32; 3],
    band_amount: [f32; 3],
    band_comp: [BandCompressor; 3],
    band_drive: [BandDrive; 3],
    mix: f32,
    bypassed: bool,
    sample_rate: f32,
    dirty: bool,
    // Pre-allocated band buffers
    buf_low: Vec<f32>,
    buf_mid: Vec<f32>,
    buf_high: Vec<f32>,
}

impl Multiband {
    pub fn new(sample_rate: u32) -> Self {
        let mut m = Self {
            xover_low_mid: LR4Crossover::new(),
            xover_mid_high: LR4Crossover::new(),
            freq_low_mid: 250.0,
            freq_mid_high: 2500.0,
            band_mode: [BandMode::Clean; 3],
            band_gain_db: [0.0; 3],
            band_amount: [0.5; 3],
            band_comp: [BandCompressor::new(), BandCompressor::new(), BandCompressor::new()],
            band_drive: [BandDrive::new(), BandDrive::new(), BandDrive::new()],
            mix: 1.0,
            bypassed: false,
            sample_rate: sample_rate as f32,
            dirty: true,
            buf_low: vec![0.0; MAX_BUF],
            buf_mid: vec![0.0; MAX_BUF],
            buf_high: vec![0.0; MAX_BUF],
        };
        m.update_coefficients();
        m
    }

    fn update_coefficients(&mut self) {
        let sr = self.sample_rate as f64;
        self.xover_low_mid.set_frequency(self.freq_low_mid as f64, sr);
        self.xover_mid_high.set_frequency(self.freq_mid_high as f64, sr);
        self.dirty = false;
    }
}

impl AudioProcessor for Multiband {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if self.sample_rate != sample_rate as f32 {
            self.sample_rate = sample_rate as f32;
            self.dirty = true;
        }
        if self.dirty {
            self.update_coefficients();
        }

        let len = buffer.len();
        let frames = len / 2;

        // Ensure buffers are large enough (rare, only on buffer size change)
        if self.buf_low.len() < len {
            self.buf_low.resize(len, 0.0);
            self.buf_mid.resize(len, 0.0);
            self.buf_high.resize(len, 0.0);
        }

        // Split into 3 bands via two LR4 crossovers
        for i in 0..frames {
            let l = buffer[i * 2];
            let r = buffer[i * 2 + 1];

            // First split: input → low + rest
            let (low_l, low_r) = self.xover_low_mid.process_lp(l, r);
            let (rest_l, rest_r) = self.xover_low_mid.process_hp(l, r);

            // Second split: rest → mid + high
            let (mid_l, mid_r) = self.xover_mid_high.process_lp(rest_l, rest_r);
            let (high_l, high_r) = self.xover_mid_high.process_hp(rest_l, rest_r);

            self.buf_low[i * 2] = low_l;
            self.buf_low[i * 2 + 1] = low_r;
            self.buf_mid[i * 2] = mid_l;
            self.buf_mid[i * 2 + 1] = mid_r;
            self.buf_high[i * 2] = high_l;
            self.buf_high[i * 2 + 1] = high_r;
        }

        // Process each band
        let sr = self.sample_rate;
        let bands: [(&mut [f32], BandMode, f32, f32); 3] = [
            (&mut self.buf_low[..len], self.band_mode[0], self.band_amount[0], self.band_gain_db[0]),
            (&mut self.buf_mid[..len], self.band_mode[1], self.band_amount[1], self.band_gain_db[1]),
            (&mut self.buf_high[..len], self.band_mode[2], self.band_amount[2], self.band_gain_db[2]),
        ];

        // Can't iterate with &mut self due to borrow checker, process inline
        // Low band
        match self.band_mode[0] {
            BandMode::Compressed => self.band_comp[0].process(&mut self.buf_low[..len], self.band_amount[0], sr),
            BandMode::Drive => self.band_drive[0].process(&mut self.buf_low[..len], self.band_amount[0], sr),
            BandMode::Clean => {}
        }
        // Mid band
        match self.band_mode[1] {
            BandMode::Compressed => self.band_comp[1].process(&mut self.buf_mid[..len], self.band_amount[1], sr),
            BandMode::Drive => self.band_drive[1].process(&mut self.buf_mid[..len], self.band_amount[1], sr),
            BandMode::Clean => {}
        }
        // High band
        match self.band_mode[2] {
            BandMode::Compressed => self.band_comp[2].process(&mut self.buf_high[..len], self.band_amount[2], sr),
            BandMode::Drive => self.band_drive[2].process(&mut self.buf_high[..len], self.band_amount[2], sr),
            BandMode::Clean => {}
        }

        // Apply per-band gain and recombine
        let gain_low = 10.0f32.powf(self.band_gain_db[0] / 20.0);
        let gain_mid = 10.0f32.powf(self.band_gain_db[1] / 20.0);
        let gain_high = 10.0f32.powf(self.band_gain_db[2] / 20.0);

        for i in 0..len {
            let wet = self.buf_low[i] * gain_low
                    + self.buf_mid[i] * gain_mid
                    + self.buf_high[i] * gain_high;
            buffer[i] = buffer[i] * (1.0 - self.mix) + wet * self.mix;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => {
                self.freq_low_mid = value.clamp(20.0, self.freq_mid_high - MIN_XOVER_GAP_HZ);
                self.dirty = true;
            }
            1 => {
                self.freq_mid_high = value.clamp(self.freq_low_mid + MIN_XOVER_GAP_HZ, 20000.0);
                self.dirty = true;
            }
            2 => self.band_mode[0] = BandMode::from_f32(value),
            3 => self.band_gain_db[0] = value.clamp(-24.0, 12.0),
            4 => self.band_amount[0] = value.clamp(0.0, 1.0),
            5 => self.band_mode[1] = BandMode::from_f32(value),
            6 => self.band_gain_db[1] = value.clamp(-24.0, 12.0),
            7 => self.band_amount[1] = value.clamp(0.0, 1.0),
            8 => self.band_mode[2] = BandMode::from_f32(value),
            9 => self.band_gain_db[2] = value.clamp(-24.0, 12.0),
            10 => self.band_amount[2] = value.clamp(0.0, 1.0),
            11 => self.mix = value.clamp(0.0, 1.0),
            12 => self.bypassed = value > 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.xover_low_mid.reset();
        self.xover_mid_high.reset();
        for c in &mut self.band_comp { *c = BandCompressor::new(); }
        for d in &mut self.band_drive { *d = BandDrive::new(); }
    }

    fn name(&self) -> &str { "Multiband" }
    fn is_bypassed(&self) -> bool { self.bypassed }
}

impl EffectDescriptor for Multiband {
    fn effect_type_id(&self) -> &str { "builtin:multiband" }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        let mode_labels = vec!["Clean".into(), "Compressed".into(), "Drive".into()];
        vec![
            ParamDescriptor { id: ParamId(0), name: "Low/Mid Xover".into(), unit: "Hz".into(), min: 20.0, max: 2000.0, default: 250.0, step: Some(1.0), kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(1), name: "Mid/High Xover".into(), unit: "Hz".into(), min: 100.0, max: 20000.0, default: 2500.0, step: Some(1.0), kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(2), name: "Low Mode".into(), unit: "".into(), min: 0.0, max: 2.0, default: 0.0, step: Some(1.0), kind: ParamKind::Enum(mode_labels.clone()) },
            ParamDescriptor { id: ParamId(3), name: "Low Gain".into(), unit: "dB".into(), min: -24.0, max: 12.0, default: 0.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(4), name: "Low Amount".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(5), name: "Mid Mode".into(), unit: "".into(), min: 0.0, max: 2.0, default: 0.0, step: Some(1.0), kind: ParamKind::Enum(mode_labels.clone()) },
            ParamDescriptor { id: ParamId(6), name: "Mid Gain".into(), unit: "dB".into(), min: -24.0, max: 12.0, default: 0.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(7), name: "Mid Amount".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(8), name: "High Mode".into(), unit: "".into(), min: 0.0, max: 2.0, default: 0.0, step: Some(1.0), kind: ParamKind::Enum(mode_labels) },
            ParamDescriptor { id: ParamId(9), name: "High Gain".into(), unit: "dB".into(), min: -24.0, max: 12.0, default: 0.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(10), name: "High Amount".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.5, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(11), name: "Mix".into(), unit: "".into(), min: 0.0, max: 1.0, default: 1.0, step: None, kind: ParamKind::Float },
            ParamDescriptor { id: ParamId(12), name: "Bypass".into(), unit: "".into(), min: 0.0, max: 1.0, default: 0.0, step: Some(1.0), kind: ParamKind::Bool },
        ]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => self.freq_low_mid,
            1 => self.freq_mid_high,
            2 => self.band_mode[0] as u32 as f32,
            3 => self.band_gain_db[0],
            4 => self.band_amount[0],
            5 => self.band_mode[1] as u32 as f32,
            6 => self.band_gain_db[1],
            7 => self.band_amount[1],
            8 => self.band_mode[2] as u32 as f32,
            9 => self.band_gain_db[2],
            10 => self.band_amount[2],
            11 => self.mix,
            12 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flat_magnitude_clean_bands() {
        // LR4 crossovers sum to an allpass (flat magnitude, phase-shifted).
        // Verify that the output RMS matches the input RMS (energy preserved).
        let sr = 48000u32;
        let frames = 2048;
        let mut mb = Multiband::new(sr);

        let make_buf = |offset: usize| -> Vec<f32> {
            (0..frames * 2).map(|i| {
                let t = (offset + i / 2) as f32 / sr as f32;
                (200.0 * t * std::f32::consts::TAU).sin() * 0.3
                + (1000.0 * t * std::f32::consts::TAU).sin() * 0.2
                + (5000.0 * t * std::f32::consts::TAU).sin() * 0.1
            }).collect()
        };

        // Settle filters
        for pass in 0..8 {
            let mut warmup = make_buf(pass * frames);
            mb.process(&mut warmup, sr);
        }

        // Measure
        let mut buf = make_buf(8 * frames);
        let original = buf.clone();
        mb.process(&mut buf, sr);

        let rms_in = (original.iter().map(|s| s * s).sum::<f32>() / original.len() as f32).sqrt();
        let rms_out = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt();

        let ratio = rms_out / rms_in;
        assert!(
            (ratio - 1.0).abs() < 0.15,
            "RMS should be preserved (ratio={ratio:.3}, in={rms_in:.4}, out={rms_out:.4})"
        );
    }

    #[test]
    fn test_band_isolation() {
        // 100Hz sine should be mostly in the low band
        let sr = 48000u32;
        let frames = 2048;
        let mut mb = Multiband::new(sr);
        mb.freq_low_mid = 250.0;
        mb.freq_mid_high = 2500.0;
        mb.dirty = true;

        // Mute mid and high to isolate low
        mb.band_gain_db[1] = -96.0; // effectively mute mid
        mb.band_gain_db[2] = -96.0; // effectively mute high

        let freq = 100.0f32;
        let mut buf: Vec<f32> = (0..frames * 2)
            .map(|i| {
                let t = (i / 2) as f32 / sr as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5
            })
            .collect();

        mb.process(&mut buf, sr);

        // Output should still have significant energy (low band passed through)
        let rms: f32 = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt();
        assert!(rms > 0.1, "100Hz should pass through low band, rms = {rms}");
    }

    #[test]
    fn test_crossover_freq_clamping() {
        let mut mb = Multiband::new(48000);
        mb.set_param(ParamId(0), 3000.0); // Try to set low/mid above mid/high (2500)
        assert!(mb.freq_low_mid < mb.freq_mid_high);
    }
}

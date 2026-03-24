use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// A single biquad filter section.
#[derive(Clone)]
struct Biquad {
    b0: f64, b1: f64, b2: f64,
    a1: f64, a2: f64,
    x1: f64, x2: f64,
    y1: f64, y2: f64,
}

impl Biquad {
    fn new() -> Self {
        Self {
            b0: 1.0, b1: 0.0, b2: 0.0,
            a1: 0.0, a2: 0.0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }

    fn process_sample(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    fn set_low_shelf(&mut self, freq: f64, gain_db: f64, sample_rate: f64) {
        let a = 10.0f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * (2.0f64).sqrt();

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha;
        self.b0 = (a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha)) / a0;
        self.b1 = (2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0)) / a0;
        self.b2 = (a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha)) / a0;
        self.a1 = (-2.0 * ((a - 1.0) + (a + 1.0) * cos_w0)) / a0;
        self.a2 = ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha) / a0;
    }

    fn set_high_shelf(&mut self, freq: f64, gain_db: f64, sample_rate: f64) {
        let a = 10.0f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * (2.0f64).sqrt();

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha;
        self.b0 = (a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha)) / a0;
        self.b1 = (-2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0)) / a0;
        self.b2 = (a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha)) / a0;
        self.a1 = (2.0 * ((a - 1.0) - (a + 1.0) * cos_w0)) / a0;
        self.a2 = ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha) / a0;
    }

    fn set_peaking(&mut self, freq: f64, gain_db: f64, q: f64, sample_rate: f64) {
        let a = 10.0f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        self.b0 = (1.0 + alpha * a) / a0;
        self.b1 = (-2.0 * cos_w0) / a0;
        self.b2 = (1.0 - alpha * a) / a0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha / a) / a0;
    }
}

/// 4-band parametric EQ: low shelf, 2x peaking, high shelf.
pub struct ParametricEq {
    bands: [Biquad; 4],
    /// Band parameters: [freq, gain_db, q] per band.
    params: [(f64, f64, f64); 4],
    sample_rate: f64,
    dirty: bool,
    bypassed: bool,
}

impl ParametricEq {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f64;
        Self {
            bands: [Biquad::new(), Biquad::new(), Biquad::new(), Biquad::new()],
            params: [
                (100.0, 0.0, 0.707),   // Low shelf
                (500.0, 0.0, 1.0),     // Peaking mid-low
                (2000.0, 0.0, 1.0),    // Peaking mid-high
                (8000.0, 0.0, 0.707),  // High shelf
            ],
            sample_rate: sr,
            dirty: true,
            bypassed: false,
        }
    }

    fn update_coefficients(&mut self) {
        if !self.dirty {
            return;
        }
        let (f, g, _q) = self.params[0];
        self.bands[0].set_low_shelf(f, g, self.sample_rate);

        let (f, g, q) = self.params[1];
        self.bands[1].set_peaking(f, g, q, self.sample_rate);

        let (f, g, q) = self.params[2];
        self.bands[2].set_peaking(f, g, q, self.sample_rate);

        let (f, g, _q) = self.params[3];
        self.bands[3].set_high_shelf(f, g, self.sample_rate);

        self.dirty = false;
    }
}

impl AudioProcessor for ParametricEq {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if self.sample_rate != sample_rate as f64 {
            self.sample_rate = sample_rate as f64;
            self.dirty = true;
        }
        self.update_coefficients();

        for sample in buffer.iter_mut() {
            let mut s = *sample as f64;
            for band in &mut self.bands {
                s = band.process_sample(s);
            }
            *sample = s as f32;
        }
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        // Params 0-2: band 0 (freq, gain, q), 3-5: band 1, etc.
        let band_idx = (param.0 / 3) as usize;
        let param_idx = (param.0 % 3) as usize;
        if band_idx < 4 {
            match param_idx {
                0 => self.params[band_idx].0 = value as f64,
                1 => self.params[band_idx].1 = value as f64,
                2 => self.params[band_idx].2 = (value as f64).max(0.1),
                _ => {}
            }
            self.dirty = true;
        }
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.reset();
        }
    }

    fn name(&self) -> &str {
        "Parametric EQ"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

impl EffectDescriptor for ParametricEq {
    fn effect_type_id(&self) -> &str {
        "builtin:eq"
    }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        let band_names = ["Low Shelf", "Mid-Low", "Mid-High", "High Shelf"];
        let mut descs = Vec::with_capacity(12);
        for (i, band_name) in band_names.iter().enumerate() {
            let base = (i * 3) as u32;
            let (default_freq, default_gain, default_q) = match i {
                0 => (100.0, 0.0, 0.707),
                1 => (500.0, 0.0, 1.0),
                2 => (2000.0, 0.0, 1.0),
                3 => (8000.0, 0.0, 0.707),
                _ => unreachable!(),
            };
            descs.push(ParamDescriptor {
                id: ParamId(base),
                name: format!("{} Freq", band_name),
                unit: "Hz".into(),
                min: 20.0,
                max: 20000.0,
                default: default_freq,
                step: None,
                kind: ParamKind::Float,
            });
            descs.push(ParamDescriptor {
                id: ParamId(base + 1),
                name: format!("{} Gain", band_name),
                unit: "dB".into(),
                min: -24.0,
                max: 24.0,
                default: default_gain,
                step: None,
                kind: ParamKind::Float,
            });
            descs.push(ParamDescriptor {
                id: ParamId(base + 2),
                name: format!("{} Q", band_name),
                unit: "".into(),
                min: 0.1,
                max: 18.0,
                default: default_q,
                step: None,
                kind: ParamKind::Float,
            });
        }
        descs
    }

    fn get_param(&self, param: ParamId) -> f32 {
        let band_idx = (param.0 / 3) as usize;
        let param_idx = param.0 % 3;
        if band_idx < 4 {
            match param_idx {
                0 => self.params[band_idx].0 as f32,
                1 => self.params[band_idx].1 as f32,
                2 => self.params[band_idx].2 as f32,
                _ => 0.0,
            }
        } else {
            0.0
        }
    }
}

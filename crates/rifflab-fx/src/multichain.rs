use crate::chain::EffectChain;
use crate::effects::multiband::LR4Crossover;
use crate::registry::EffectRegistry;
use rifflab_core::audio::AudioProcessor;
use rifflab_core::preset::{EffectPreset, EffectState};
use serde::{Deserialize, Serialize};

const MAX_BUF: usize = 4096;
const MIN_XOVER_GAP_HZ: f32 = 100.0;

/// Multiband preset data for serialization.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MultibandPreset {
    pub crossover_low_mid: f32,
    pub crossover_mid_high: f32,
    pub band_gains: [f32; 3],
    pub chains: [Vec<EffectState>; 3],
}

/// Routes audio through a 3-band crossover with independent effect chains per band.
pub struct MultibandRouter {
    pub crossover_low_mid: f32,
    pub crossover_mid_high: f32,
    pub band_gains: [f32; 3],
    xover1: LR4Crossover,
    xover2: LR4Crossover,
    pub chains: [EffectChain; 3],
    buf_low: Vec<f32>,
    buf_mid: Vec<f32>,
    buf_high: Vec<f32>,
    sample_rate: f32,
    dirty: bool,
}

impl MultibandRouter {
    pub fn new() -> Self {
        Self {
            crossover_low_mid: 250.0,
            crossover_mid_high: 2500.0,
            band_gains: [0.0; 3],
            xover1: LR4Crossover::new(),
            xover2: LR4Crossover::new(),
            chains: [EffectChain::new(), EffectChain::new(), EffectChain::new()],
            buf_low: vec![0.0; MAX_BUF],
            buf_mid: vec![0.0; MAX_BUF],
            buf_high: vec![0.0; MAX_BUF],
            sample_rate: 48000.0,
            dirty: true,
        }
    }

    fn update_coefficients(&mut self) {
        let sr = self.sample_rate as f64;
        self.xover1.set_frequency(self.crossover_low_mid as f64, sr);
        self.xover2.set_frequency(self.crossover_mid_high as f64, sr);
        self.dirty = false;
    }

    pub fn set_crossover_low_mid(&mut self, freq: f32) {
        self.crossover_low_mid = freq.clamp(20.0, self.crossover_mid_high - MIN_XOVER_GAP_HZ);
        self.dirty = true;
    }

    pub fn set_crossover_mid_high(&mut self, freq: f32) {
        self.crossover_mid_high = freq.clamp(self.crossover_low_mid + MIN_XOVER_GAP_HZ, 20000.0);
        self.dirty = true;
    }

    /// Check if any band has effects loaded.
    pub fn has_effects(&self) -> bool {
        self.chains.iter().any(|c| !c.is_empty())
    }

    /// Process the buffer through the multiband crossover and per-band chains.
    pub fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if self.sample_rate != sample_rate as f32 {
            self.sample_rate = sample_rate as f32;
            self.dirty = true;
        }
        if self.dirty {
            self.update_coefficients();
        }

        let len = buffer.len();
        let frames = len / 2;

        // Ensure buffers are large enough
        if self.buf_low.len() < len {
            self.buf_low.resize(len, 0.0);
            self.buf_mid.resize(len, 0.0);
            self.buf_high.resize(len, 0.0);
        }

        // Split into 3 bands
        for i in 0..frames {
            let l = buffer[i * 2];
            let r = buffer[i * 2 + 1];

            let (low_l, low_r) = self.xover1.process_lp(l, r);
            let (rest_l, rest_r) = self.xover1.process_hp(l, r);
            let (mid_l, mid_r) = self.xover2.process_lp(rest_l, rest_r);
            let (high_l, high_r) = self.xover2.process_hp(rest_l, rest_r);

            self.buf_low[i * 2] = low_l;
            self.buf_low[i * 2 + 1] = low_r;
            self.buf_mid[i * 2] = mid_l;
            self.buf_mid[i * 2 + 1] = mid_r;
            self.buf_high[i * 2] = high_l;
            self.buf_high[i * 2 + 1] = high_r;
        }

        // Process each band through its effect chain
        self.chains[0].process(&mut self.buf_low[..len], sample_rate);
        self.chains[1].process(&mut self.buf_mid[..len], sample_rate);
        self.chains[2].process(&mut self.buf_high[..len], sample_rate);

        // Apply per-band gain and recombine
        let gain_low = 10.0f32.powf(self.band_gains[0] / 20.0);
        let gain_mid = 10.0f32.powf(self.band_gains[1] / 20.0);
        let gain_high = 10.0f32.powf(self.band_gains[2] / 20.0);

        for i in 0..len {
            buffer[i] = self.buf_low[i] * gain_low
                      + self.buf_mid[i] * gain_mid
                      + self.buf_high[i] * gain_high;
        }
    }

    pub fn reset(&mut self) {
        self.xover1.reset();
        self.xover2.reset();
        for chain in &mut self.chains {
            chain.reset();
        }
    }

    /// Snapshot as a MultibandPreset.
    pub fn to_preset(&self) -> MultibandPreset {
        MultibandPreset {
            crossover_low_mid: self.crossover_low_mid,
            crossover_mid_high: self.crossover_mid_high,
            band_gains: self.band_gains,
            chains: [
                self.chains[0].to_preset("low").effects,
                self.chains[1].to_preset("mid").effects,
                self.chains[2].to_preset("high").effects,
            ],
        }
    }

    /// Load from a MultibandPreset.
    pub fn load_from_preset(&mut self, preset: &MultibandPreset, registry: &EffectRegistry) {
        self.crossover_low_mid = preset.crossover_low_mid;
        self.crossover_mid_high = preset.crossover_mid_high;
        self.band_gains = preset.band_gains;
        self.dirty = true;

        for (i, chain_effects) in preset.chains.iter().enumerate() {
            let chain_preset = EffectPreset {
                name: String::new(),
                effects: chain_effects.clone(),
            };
            self.chains[i].load_from_preset(&chain_preset, registry);
        }
    }
}

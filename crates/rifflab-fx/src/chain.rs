use rifflab_core::audio::{AudioProcessor, ParamId};

/// An ordered chain of audio effects.
/// Implements AudioProcessor itself so it can be plugged into the audio graph.
pub struct EffectChain {
    effects: Vec<Box<dyn AudioProcessor>>,
}

impl EffectChain {
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    pub fn add(&mut self, effect: Box<dyn AudioProcessor>) {
        self.effects.push(effect);
    }

    pub fn insert(&mut self, index: usize, effect: Box<dyn AudioProcessor>) {
        self.effects.insert(index, effect);
    }

    pub fn remove(&mut self, index: usize) -> Box<dyn AudioProcessor> {
        self.effects.remove(index)
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn effects(&self) -> &[Box<dyn AudioProcessor>] {
        &self.effects
    }

    pub fn effects_mut(&mut self) -> &mut [Box<dyn AudioProcessor>] {
        &mut self.effects
    }
}

impl Default for EffectChain {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioProcessor for EffectChain {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        for effect in &mut self.effects {
            if !effect.is_bypassed() {
                effect.process(buffer, sample_rate);
            }
        }
    }

    fn set_param(&mut self, _param: ParamId, _value: f32) {
        // Params are set on individual effects, not on the chain
    }

    fn reset(&mut self) {
        for effect in &mut self.effects {
            effect.reset();
        }
    }

    fn name(&self) -> &str {
        "Effect Chain"
    }
}

use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId};

/// An ordered chain of audio effects.
/// Implements both AudioProcessor and EffectDescriptor so it can be plugged
/// into the audio graph while still exposing rich metadata.
pub struct EffectChain {
    effects: Vec<Box<dyn EffectDescriptor>>,
}

impl EffectChain {
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    pub fn add(&mut self, effect: Box<dyn EffectDescriptor>) {
        self.effects.push(effect);
    }

    pub fn insert(&mut self, index: usize, effect: Box<dyn EffectDescriptor>) {
        self.effects.insert(index, effect);
    }

    pub fn remove(&mut self, index: usize) -> Box<dyn EffectDescriptor> {
        self.effects.remove(index)
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn effects(&self) -> &[Box<dyn EffectDescriptor>] {
        &self.effects
    }

    pub fn effects_mut(&mut self) -> &mut [Box<dyn EffectDescriptor>] {
        &mut self.effects
    }

    /// Move an effect from one position to another.
    pub fn move_effect(&mut self, from: usize, to: usize) {
        if from == to || from >= self.effects.len() || to >= self.effects.len() {
            return;
        }
        let effect = self.effects.remove(from);
        self.effects.insert(to, effect);
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

impl EffectDescriptor for EffectChain {
    fn effect_type_id(&self) -> &str {
        "builtin:chain"
    }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        // Aggregate descriptors from all child effects.
        // Each child's params are returned as-is; callers should use the
        // per-effect accessors for disambiguation.
        let mut all = Vec::new();
        for effect in &self.effects {
            all.extend(effect.param_descriptors());
        }
        all
    }

    fn get_param(&self, param: ParamId) -> f32 {
        // Delegate to the first child that recognises this param id.
        for effect in &self.effects {
            for desc in effect.param_descriptors() {
                if desc.id == param {
                    return effect.get_param(param);
                }
            }
        }
        0.0
    }
}

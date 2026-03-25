use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId};
use rifflab_core::preset::{EffectPreset, EffectState, ParamValue};

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

    /// Snapshot the current chain state as a preset.
    pub fn to_preset(&self, name: &str) -> EffectPreset {
        EffectPreset {
            name: name.to_string(),
            effects: self.effects.iter().map(|e| {
                let descs = e.param_descriptors();
                EffectState {
                    effect_type: e.effect_type_id().to_string(),
                    active: !e.is_bypassed(),
                    params: descs.iter().map(|d| ParamValue {
                        name: d.name.clone(),
                        id: d.id,
                        value: e.get_param(d.id),
                        min: d.min,
                        max: d.max,
                    }).collect(),
                }
            }).collect(),
        }
    }

    /// Replace the chain contents from a preset using the registry to create effects.
    pub fn load_from_preset(
        &mut self,
        preset: &EffectPreset,
        registry: &crate::registry::EffectRegistry,
    ) {
        self.effects.clear();
        for state in &preset.effects {
            if let Some(mut effect) = registry.create_effect(&state.effect_type) {
                // Apply saved params
                for pv in &state.params {
                    effect.set_param(pv.id, pv.value);
                }
                // Apply bypass
                if !state.active {
                    for desc in effect.param_descriptors() {
                        if desc.name == "Bypass" {
                            effect.set_param(desc.id, 1.0);
                            break;
                        }
                    }
                }
                self.effects.push(effect);
            } else {
                log::warn!("Unknown effect type '{}' in preset, skipping", state.effect_type);
            }
        }
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

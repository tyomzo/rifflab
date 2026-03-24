use rifflab_core::audio::{AudioProcessor, EffectDescriptor, ParamDescriptor, ParamId, ParamKind};

/// Passthrough tuner effect.
///
/// Audio passes through unchanged. Pitch detection is handled at a higher
/// layer (analysis module); this effect exists as a placeholder in the
/// signal chain that can be bypassed like any other effect.
pub struct Tuner {
    bypassed: bool,
}

impl Tuner {
    pub fn new() -> Self {
        Self { bypassed: false }
    }
}

impl Default for Tuner {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioProcessor for Tuner {
    fn process(&mut self, _buffer: &mut [f32], _sample_rate: u32) {
        // Passthrough — no processing.
    }

    fn set_param(&mut self, param: ParamId, value: f32) {
        match param.0 {
            0 => self.bypassed = value >= 0.5,
            _ => {}
        }
    }

    fn reset(&mut self) {
        // Nothing to reset.
    }

    fn name(&self) -> &str {
        "Tuner"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }
}

impl EffectDescriptor for Tuner {
    fn effect_type_id(&self) -> &str {
        "builtin:tuner"
    }

    fn param_descriptors(&self) -> Vec<ParamDescriptor> {
        vec![ParamDescriptor {
            id: ParamId(0),
            name: "Bypass".into(),
            unit: "".into(),
            min: 0.0,
            max: 1.0,
            default: 0.0,
            step: Some(1.0),
            kind: ParamKind::Bool,
        }]
    }

    fn get_param(&self, param: ParamId) -> f32 {
        match param.0 {
            0 => if self.bypassed { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }
}

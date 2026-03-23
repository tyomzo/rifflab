use serde::{Deserialize, Serialize};
use crate::audio::ParamId;

/// A saved effect chain preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectPreset {
    /// Human-readable preset name.
    pub name: String,
    /// Ordered list of effects in the chain.
    pub effects: Vec<EffectState>,
}

/// State of a single effect in a preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectState {
    /// Effect type identifier (e.g., "compressor", "overdrive").
    pub effect_type: String,
    /// Whether the effect is active (not bypassed).
    pub active: bool,
    /// Parameter values.
    pub params: Vec<ParamValue>,
}

/// A named parameter value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamValue {
    /// Parameter name for display.
    pub name: String,
    /// Parameter ID.
    pub id: ParamId,
    /// Current value.
    pub value: f32,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
}

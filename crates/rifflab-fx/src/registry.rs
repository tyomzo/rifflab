use rifflab_core::audio::EffectDescriptor;

use crate::effects::cabinet::Cabinet;
use crate::effects::multiband::Multiband;
use crate::effects::chorus::Chorus;
use crate::effects::compressor::Compressor;
use crate::effects::delay::Delay;
use crate::effects::eq::ParametricEq;
use crate::effects::flanger::Flanger;
use crate::effects::noise_gate::NoiseGate;
use crate::effects::overdrive::Overdrive;
use crate::effects::phaser::Phaser;
use crate::effects::reverb::Reverb;
use crate::effects::tuner::Tuner;

/// Factory for a single registered effect type.
struct EffectFactory {
    type_id: String,
    name: String,
    category: String,
    create: Box<dyn Fn() -> Box<dyn EffectDescriptor> + Send + Sync>,
}

/// Registry of all available effect types.
///
/// Built-in effects are registered automatically on construction.
/// Additional factories (e.g. user scripts) can be added later.
pub struct EffectRegistry {
    factories: Vec<EffectFactory>,
}

impl EffectRegistry {
    /// Create a new registry pre-populated with all built-in effects.
    pub fn new() -> Self {
        let mut reg = Self {
            factories: Vec::new(),
        };

        reg.register(
            "builtin:noise_gate",
            "Noise Gate",
            "Built-in",
            || Box::new(NoiseGate::new()),
        );

        reg.register(
            "builtin:compressor",
            "Compressor",
            "Built-in",
            || Box::new(Compressor::new()),
        );

        reg.register(
            "builtin:overdrive",
            "Overdrive",
            "Built-in",
            || Box::new(Overdrive::new()),
        );

        reg.register(
            "builtin:eq",
            "Parametric EQ",
            "Built-in",
            || Box::new(ParametricEq::new(48000)),
        );

        reg.register(
            "builtin:reverb",
            "Reverb",
            "Built-in",
            || Box::new(Reverb::new(48000)),
        );

        reg.register(
            "builtin:tuner",
            "Tuner",
            "Built-in",
            || Box::new(Tuner::new()),
        );

        reg.register(
            "builtin:delay",
            "Delay",
            "Built-in",
            || Box::new(Delay::new(48000)),
        );

        reg.register(
            "builtin:chorus",
            "Chorus",
            "Built-in",
            || Box::new(Chorus::new(48000)),
        );

        reg.register(
            "builtin:flanger",
            "Flanger",
            "Built-in",
            || Box::new(Flanger::new(48000)),
        );

        reg.register(
            "builtin:phaser",
            "Phaser",
            "Built-in",
            || Box::new(Phaser::new(48000)),
        );

        reg.register(
            "builtin:cabinet",
            "Cabinet Sim",
            "Built-in",
            || Box::new(Cabinet::new(48000)),
        );

        reg.register(
            "builtin:multiband",
            "Multiband",
            "Built-in",
            || Box::new(Multiband::new(48000)),
        );

        reg
    }

    /// Register a new effect factory.
    pub fn register<F>(
        &mut self,
        type_id: &str,
        name: &str,
        category: &str,
        create: F,
    ) where
        F: Fn() -> Box<dyn EffectDescriptor> + Send + Sync + 'static,
    {
        self.factories.push(EffectFactory {
            type_id: type_id.to_string(),
            name: name.to_string(),
            category: category.to_string(),
            create: Box::new(create),
        });
    }

    /// List all registered effects as `(type_id, name, category)` tuples.
    pub fn list_effects(&self) -> Vec<(String, String, String)> {
        self.factories
            .iter()
            .map(|f| (f.type_id.clone(), f.name.clone(), f.category.clone()))
            .collect()
    }

    /// Create a new instance of an effect by its type id.
    /// Returns `None` if the type id is not registered.
    pub fn create_effect(&self, type_id: &str) -> Option<Box<dyn EffectDescriptor>> {
        self.factories
            .iter()
            .find(|f| f.type_id == type_id)
            .map(|f| (f.create)())
    }
}

impl Default for EffectRegistry {
    fn default() -> Self {
        Self::new()
    }
}

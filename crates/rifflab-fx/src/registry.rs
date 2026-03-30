use std::collections::HashMap;

use rifflab_core::audio::EffectDescriptor;

use crate::effects::cabinet::Cabinet;
use crate::effects::multiband::Multiband;
use crate::effects::transient::TransientShaper;
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
    name: String,
    category: String,
    create: Box<dyn Fn() -> Box<dyn EffectDescriptor> + Send + Sync>,
}

/// Registry of all available effect types.
///
/// Built-in effects are registered automatically on construction.
/// Additional factories (e.g. user scripts) can be added later.
/// Uses HashMap for O(1) lookup by type_id and duplicate prevention.
pub struct EffectRegistry {
    factories: HashMap<String, EffectFactory>,
    /// Insertion order preserved for UI listing.
    order: Vec<String>,
}

impl EffectRegistry {
    /// Create a new registry pre-populated with all built-in effects.
    pub fn new() -> Self {
        let mut reg = Self {
            factories: HashMap::new(),
            order: Vec::new(),
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

        reg.register(
            "builtin:transient",
            "Transient Shaper",
            "Built-in",
            || Box::new(TransientShaper::new(48000)),
        );

        reg
    }

    /// Register a new effect factory. Duplicates are silently ignored.
    pub fn register<F>(
        &mut self,
        type_id: &str,
        name: &str,
        category: &str,
        create: F,
    ) where
        F: Fn() -> Box<dyn EffectDescriptor> + Send + Sync + 'static,
    {
        use std::collections::hash_map::Entry;
        let key = type_id.to_string();
        if let Entry::Vacant(e) = self.factories.entry(key.clone()) {
            e.insert(EffectFactory {
                name: name.to_string(),
                category: category.to_string(),
                create: Box::new(create),
            });
            self.order.push(key);
        }
    }

    /// List all registered effects as `(type_id, name, category)` tuples,
    /// in registration order.
    pub fn list_effects(&self) -> Vec<(String, String, String)> {
        self.order
            .iter()
            .filter_map(|id| {
                self.factories
                    .get(id)
                    .map(|f| (id.clone(), f.name.clone(), f.category.clone()))
            })
            .collect()
    }

    /// Create a new instance of an effect by its type id.
    /// Returns `None` if the type id is not registered.
    pub fn create_effect(&self, type_id: &str) -> Option<Box<dyn EffectDescriptor>> {
        self.factories.get(type_id).map(|f| (f.create)())
    }
}

impl Default for EffectRegistry {
    fn default() -> Self {
        Self::new()
    }
}

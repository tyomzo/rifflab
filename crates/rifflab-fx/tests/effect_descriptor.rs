use rifflab_core::audio::{EffectDescriptor, ParamId, ParamKind};
use rifflab_fx::effects::compressor::Compressor;
use rifflab_fx::effects::eq::ParametricEq;
use rifflab_fx::effects::noise_gate::NoiseGate;
use rifflab_fx::effects::overdrive::Overdrive;
use rifflab_fx::effects::reverb::Reverb;
use rifflab_fx::effects::tuner::Tuner;
use rifflab_fx::registry::EffectRegistry;

// ---------------------------------------------------------------------------
// Helper: verify every param descriptor has valid ranges and that get_param
// returns a value within those ranges.
// ---------------------------------------------------------------------------
fn assert_descriptors_valid(effect: &dyn EffectDescriptor) {
    let descs = effect.param_descriptors();
    assert!(!descs.is_empty(), "{} should have at least one param", effect.name());
    for desc in &descs {
        assert!(
            desc.min <= desc.max,
            "{}: param '{}' has min ({}) > max ({})",
            effect.name(),
            desc.name,
            desc.min,
            desc.max,
        );
        assert!(
            desc.default >= desc.min && desc.default <= desc.max,
            "{}: param '{}' default ({}) outside [{}, {}]",
            effect.name(),
            desc.name,
            desc.default,
            desc.min,
            desc.max,
        );
        let val = effect.get_param(desc.id);
        assert!(
            val >= desc.min && val <= desc.max,
            "{}: param '{}' get_param returned {} outside [{}, {}]",
            effect.name(),
            desc.name,
            val,
            desc.min,
            desc.max,
        );
    }
}

// ---------------------------------------------------------------------------
// Per-effect param_descriptors count and valid ranges
// ---------------------------------------------------------------------------

#[test]
fn noise_gate_descriptors() {
    let ng = NoiseGate::new();
    assert_eq!(ng.effect_type_id(), "builtin:noise_gate");
    let descs = ng.param_descriptors();
    assert_eq!(descs.len(), 3);
    assert_descriptors_valid(&ng);
}

#[test]
fn compressor_descriptors() {
    let comp = Compressor::new();
    assert_eq!(comp.effect_type_id(), "builtin:compressor");
    let descs = comp.param_descriptors();
    assert_eq!(descs.len(), 5);
    assert_descriptors_valid(&comp);
}

#[test]
fn overdrive_descriptors() {
    let od = Overdrive::new();
    assert_eq!(od.effect_type_id(), "builtin:overdrive");
    let descs = od.param_descriptors();
    assert_eq!(descs.len(), 4);
    assert_descriptors_valid(&od);

    // Shaper type param should be an Enum
    let shaper = &descs[3];
    match &shaper.kind {
        ParamKind::Enum(variants) => {
            assert_eq!(variants.len(), 3);
            assert_eq!(variants[0], "Tanh");
            assert_eq!(variants[1], "HardClip");
            assert_eq!(variants[2], "SoftClip");
        }
        other => panic!("Expected Enum, got {:?}", other),
    }
}

#[test]
fn eq_descriptors() {
    let eq = ParametricEq::new(48000);
    assert_eq!(eq.effect_type_id(), "builtin:eq");
    let descs = eq.param_descriptors();
    // 4 bands * 3 params each = 12
    assert_eq!(descs.len(), 12);
    assert_descriptors_valid(&eq);
}

#[test]
fn reverb_descriptors() {
    let rev = Reverb::new(48000);
    assert_eq!(rev.effect_type_id(), "builtin:reverb");
    let descs = rev.param_descriptors();
    assert_eq!(descs.len(), 4);
    assert_descriptors_valid(&rev);
}

#[test]
fn tuner_descriptors() {
    let tuner = Tuner::new();
    assert_eq!(tuner.effect_type_id(), "builtin:tuner");
    let descs = tuner.param_descriptors();
    assert_eq!(descs.len(), 1);
    assert_descriptors_valid(&tuner);
    // Should be a Bool param
    match &descs[0].kind {
        ParamKind::Bool => {}
        other => panic!("Expected Bool, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// get_param returns default values for freshly constructed effects
// ---------------------------------------------------------------------------

#[test]
fn get_param_returns_defaults() {
    let ng = NoiseGate::new();
    for desc in ng.param_descriptors() {
        let val = ng.get_param(desc.id);
        assert!(
            (val - desc.default).abs() < 1e-6,
            "NoiseGate param '{}': expected default {}, got {}",
            desc.name,
            desc.default,
            val,
        );
    }

    let comp = Compressor::new();
    for desc in comp.param_descriptors() {
        let val = comp.get_param(desc.id);
        assert!(
            (val - desc.default).abs() < 1e-6,
            "Compressor param '{}': expected default {}, got {}",
            desc.name,
            desc.default,
            val,
        );
    }

    let od = Overdrive::new();
    for desc in od.param_descriptors() {
        let val = od.get_param(desc.id);
        assert!(
            (val - desc.default).abs() < 1e-6,
            "Overdrive param '{}': expected default {}, got {}",
            desc.name,
            desc.default,
            val,
        );
    }

    let rev = Reverb::new(48000);
    for desc in rev.param_descriptors() {
        let val = rev.get_param(desc.id);
        assert!(
            (val - desc.default).abs() < 1e-6,
            "Reverb param '{}': expected default {}, got {}",
            desc.name,
            desc.default,
            val,
        );
    }
}

// ---------------------------------------------------------------------------
// get_param tracks set_param for Overdrive shaper enum
// ---------------------------------------------------------------------------

#[test]
fn overdrive_shaper_roundtrip() {
    use rifflab_core::audio::AudioProcessor;
    let mut od = Overdrive::new();
    // Default is Tanh = 0.0
    assert_eq!(od.get_param(ParamId(3)), 0.0);
    od.set_param(ParamId(3), 2.0); // SoftClip
    assert_eq!(od.get_param(ParamId(3)), 2.0);
    od.set_param(ParamId(3), 1.0); // HardClip
    assert_eq!(od.get_param(ParamId(3)), 1.0);
}

// ---------------------------------------------------------------------------
// EffectRegistry
// ---------------------------------------------------------------------------

#[test]
fn registry_lists_all_builtins() {
    let reg = EffectRegistry::new();
    let list = reg.list_effects();
    assert_eq!(list.len(), 11, "Expected 11 built-in effects, got {}", list.len());

    let type_ids: Vec<&str> = list.iter().map(|(id, _, _)| id.as_str()).collect();
    assert!(type_ids.contains(&"builtin:noise_gate"));
    assert!(type_ids.contains(&"builtin:compressor"));
    assert!(type_ids.contains(&"builtin:overdrive"));
    assert!(type_ids.contains(&"builtin:eq"));
    assert!(type_ids.contains(&"builtin:reverb"));
    assert!(type_ids.contains(&"builtin:tuner"));
    assert!(type_ids.contains(&"builtin:delay"));
    assert!(type_ids.contains(&"builtin:chorus"));
    assert!(type_ids.contains(&"builtin:flanger"));
    assert!(type_ids.contains(&"builtin:phaser"));
    assert!(type_ids.contains(&"builtin:cabinet"));

    // All should be "Built-in" category
    for (_, _, cat) in &list {
        assert_eq!(cat, "Built-in");
    }
}

#[test]
fn registry_creates_instances() {
    let reg = EffectRegistry::new();

    for (type_id, _name, _cat) in reg.list_effects() {
        let effect = reg
            .create_effect(&type_id)
            .unwrap_or_else(|| panic!("Failed to create effect '{}'", type_id));
        assert_eq!(effect.effect_type_id(), type_id);
        assert_descriptors_valid(effect.as_ref());
    }
}

#[test]
fn registry_returns_none_for_unknown() {
    let reg = EffectRegistry::new();
    assert!(reg.create_effect("nonexistent:foo").is_none());
}

// ---------------------------------------------------------------------------
// EffectChain with EffectDescriptor
// ---------------------------------------------------------------------------

#[test]
fn effect_chain_aggregates_descriptors() {
    use rifflab_fx::chain::EffectChain;

    let mut chain = EffectChain::new();
    chain.add(Box::new(NoiseGate::new()));
    chain.add(Box::new(Compressor::new()));

    assert_eq!(chain.effect_type_id(), "builtin:chain");

    let descs = chain.param_descriptors();
    // NoiseGate has 3, Compressor has 5 = 8 total
    assert_eq!(descs.len(), 8);
}

#[test]
fn effect_chain_processes_audio() {
    use rifflab_core::audio::AudioProcessor;
    use rifflab_fx::chain::EffectChain;

    let mut chain = EffectChain::new();
    chain.add(Box::new(Tuner::new())); // passthrough
    let mut buf = vec![0.5f32; 256];
    chain.process(&mut buf, 48000);
    // Tuner is passthrough, buffer should be unchanged
    assert!((buf[0] - 0.5).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// Preset round-trip with EffectDescriptor metadata
// ---------------------------------------------------------------------------

#[test]
fn preset_roundtrip_with_descriptors() {
    use rifflab_core::preset::{EffectPreset, EffectState, ParamValue};

    let reg = EffectRegistry::new();

    // Build a preset from live effects
    let effects: Vec<Box<dyn EffectDescriptor>> = vec![
        reg.create_effect("builtin:noise_gate").unwrap(),
        reg.create_effect("builtin:compressor").unwrap(),
    ];

    let states: Vec<EffectState> = effects
        .iter()
        .map(|e| {
            let descs = e.param_descriptors();
            EffectState {
                effect_type: e.effect_type_id().to_string(),
                active: !e.is_bypassed(),
                params: descs
                    .iter()
                    .map(|d| ParamValue {
                        name: d.name.clone(),
                        id: d.id,
                        value: e.get_param(d.id),
                        min: d.min,
                        max: d.max,
                    })
                    .collect(),
            }
        })
        .collect();

    let preset = EffectPreset {
        name: "Test Preset".into(),
        effects: states,
    };

    // Serialize to TOML and back
    let toml_str = toml::to_string_pretty(&preset).expect("serialize");
    let loaded: EffectPreset = toml::from_str(&toml_str).expect("deserialize");

    assert_eq!(loaded.name, "Test Preset");
    assert_eq!(loaded.effects.len(), 2);
    assert_eq!(loaded.effects[0].effect_type, "builtin:noise_gate");
    assert_eq!(loaded.effects[0].params.len(), 3);
    assert_eq!(loaded.effects[1].effect_type, "builtin:compressor");
    assert_eq!(loaded.effects[1].params.len(), 5);

    // Restore: create effects from registry and apply saved params
    for state in &loaded.effects {
        let mut effect = reg
            .create_effect(&state.effect_type)
            .expect("known effect type");
        for pv in &state.params {
            effect.set_param(pv.id, pv.value);
        }
        // Verify params match
        for pv in &state.params {
            let val = effect.get_param(pv.id);
            assert!(
                (val - pv.value).abs() < 1e-4,
                "Param '{}' mismatch after restore: expected {}, got {}",
                pv.name,
                pv.value,
                val,
            );
        }
    }
}

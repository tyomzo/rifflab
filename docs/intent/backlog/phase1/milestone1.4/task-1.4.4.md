---
id: "1.4.4"
title: "Create EffectRegistry"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-09"
---

Create a registry module for dynamic effect instantiation. `EffectRegistry` holds a `HashMap<String, Box<dyn Fn() -> Box<dyn EffectDescriptor>>>` mapping effect type IDs to factory functions. Register all five built-in effects (NoiseGate, Compressor, Overdrive, ParametricEq, Reverb) at construction time. Provide methods: `list_effects() -> Vec<(String, String)>` returning (type_id, display_name) pairs, and `create_effect(type_id: &str) -> Option<Box<dyn EffectDescriptor>>` to instantiate an effect by its type ID. File: `crates/rifflab-fx/src/registry.rs` (new). Acceptance: registry lists all 5 built-in effects and creates working instances on demand.

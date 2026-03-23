---
id: "1.4.1"
title: "Add EffectDescriptor trait to core"
status: pending
crate: "rifflab-core"
requirement: "FR-M2-08"
---

Add effect metadata types and the `EffectDescriptor` trait to the core audio module. Define `ParamDescriptor` struct with fields: `id: String`, `name: String`, `unit: String`, `min: f64`, `max: f64`, `default: f64`, `step: f64`, `kind: ParamKind`. Define `ParamKind` enum with variants: `Float`, `Int`, `Bool`, `Enum(Vec<String>)`. Define the `EffectDescriptor` trait extending `AudioProcessor` with methods: `effect_type_id() -> &str`, `param_descriptors() -> Vec<ParamDescriptor>`, `get_param(id: &str) -> Option<f64>`. File: `crates/rifflab-core/src/audio.rs`. Acceptance: compiles cleanly and the trait is usable from the rifflab-fx crate.

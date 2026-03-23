---
id: "1.4.7"
title: "Preset save/load with EffectDescriptor"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-04"
---

Update preset serialization to leverage `EffectDescriptor` metadata. When saving, auto-populate `ParamValue` entries by iterating `param_descriptors()` and calling `get_param()` for each, paired with the `effect_type_id()`. When loading, use the `EffectRegistry` to reconstruct effects by `effect_type_id`, then apply saved parameter values. Write a round-trip test: save a preset with known effect parameters, load it back, and verify all parameters match. File: `crates/rifflab-fx/src/preset.rs`. Acceptance: save preset followed by load preset produces an effect chain with all parameter values identical to the original.

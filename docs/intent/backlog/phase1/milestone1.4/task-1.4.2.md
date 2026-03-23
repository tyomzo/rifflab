---
id: "1.4.2"
title: "Implement EffectDescriptor for built-in effects"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-08"
---

Implement the `EffectDescriptor` trait for all five built-in effects: `NoiseGate`, `Compressor`, `Overdrive`, `ParametricEq`, and `Reverb`. Each implementation returns correct `ParamDescriptor` entries with accurate names, units, min/max ranges, defaults, and step sizes matching the effect's actual parameters. Implement `get_param()` to return the current value of any parameter by ID. Files: all five effect source files in `crates/rifflab-fx/src/effects/`. Acceptance: querying any built-in effect returns correct metadata that matches its actual parameter set.

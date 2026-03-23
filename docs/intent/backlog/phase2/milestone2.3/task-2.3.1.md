---
id: "2.3.1"
title: "Rhai ScriptedEffect wrapper"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-10"
---

Add the rhai dependency to rifflab-fx. Implement `ScriptedEffect` struct that loads a `.rhai` script file and wraps it as an EffectDescriptor. The script must export `fn name() -> String`, `fn params() -> Vec<ParamDescriptor>`, and `fn process(samples, params) -> samples`. ScriptedEffect calls these functions via the Rhai engine, translating between Rhai types and the effect system types.

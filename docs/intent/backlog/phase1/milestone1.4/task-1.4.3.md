---
id: "1.4.3"
title: "Update EffectChain to Box<dyn EffectDescriptor>"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-08"
---

Change the internal storage of `EffectChain` from `Vec<Box<dyn AudioProcessor>>` to `Vec<Box<dyn EffectDescriptor>>`. Update the signatures of `add()`, `insert()`, `remove()`, and `effects()` to use the new type. `EffectChain` itself should also implement the `EffectDescriptor` trait, aggregating its children's parameter descriptors (prefixed by index or effect ID for uniqueness). File: `crates/rifflab-fx/src/chain.rs`. Acceptance: chain compiles with the new type and all existing tests continue to pass.

---
id: "2.2.3"
title: "Chorus effect"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-03"
---

Implement a chorus effect using an LFO-modulated delay line. Parameters: rate (LFO frequency in Hz), depth (modulation amount in ms), mix (dry/wet blend 0.0-1.0). The LFO modulates the delay time around a center point to create the characteristic pitch modulation. Implement EffectDescriptor with param_descriptors() returning the parameter definitions for auto-generated UI.

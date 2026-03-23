---
id: "2.2.5"
title: "Flanger effect"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-03"
---

Implement a flanger effect using an LFO-modulated short delay line (0.1-10ms range). Parameters: rate (LFO frequency in Hz), depth (modulation depth), feedback (0.0-1.0, allows negative for jet-plane effect), mix (dry/wet blend 0.0-1.0). The short modulated delay creates comb-filtering that sweeps through the spectrum. Implement EffectDescriptor with param_descriptors().

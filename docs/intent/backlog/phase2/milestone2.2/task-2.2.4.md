---
id: "2.2.4"
title: "Phaser effect"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-03"
---

Implement a phaser effect using an LFO-modulated allpass filter cascade. Parameters: rate (LFO frequency in Hz), depth (modulation depth), stages (number of allpass stages, e.g. 4/6/8/12), feedback (0.0-1.0), mix (dry/wet blend 0.0-1.0). The LFO sweeps the allpass center frequencies to create moving notches in the spectrum. Implement EffectDescriptor with param_descriptors().

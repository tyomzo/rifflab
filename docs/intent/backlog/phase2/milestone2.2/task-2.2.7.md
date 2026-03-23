---
id: "2.2.7"
title: "Cabinet simulation"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-03"
---

Implement a cabinet simulation effect using a convolution engine. Load a WAV impulse response file and apply it via FFT-based convolution. Parameters: IR file path (path to the impulse response WAV), mix (dry/wet blend 0.0-1.0). Use partitioned convolution for low-latency operation suitable for real-time audio. Implement EffectDescriptor with param_descriptors().

---
id: "1.4.5"
title: "Implement tuner effect"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-02"
---

Implement a chromatic tuner as an audio effect. The tuner passes audio through unchanged (unity gain) but internally runs lightweight pitch detection (or reads from a shared `PitchFrame` if available) to determine the current note being played. Implement both `AudioProcessor` (process is pass-through) and `EffectDescriptor` (exposes read-only parameters for detected note name, cents deviation, and frequency). File: `crates/rifflab-fx/src/effects/tuner.rs` (new). Acceptance: audio passes through unmodified and tuner data (note, cents, frequency) is accessible via the descriptor interface.

---
id: "1.1.1"
title: "Complete ALSA backend via cpal"
status: pending
crate: "rifflab-audio"
requirement: "FR-M1-01"
---

Implement the cpal-based ALSA backend. In `start()`, create both input and output streams using cpal's default host and device selection. Wire the `AudioCallback` into cpal's stream callback so that the audio graph is driven each buffer cycle. Handle sample format conversion (i16/u16/f32) transparently so the rest of the engine always works with f32. File: `crates/rifflab-audio/src/backend/alsa_backend.rs`. Acceptance: audio plays through ALSA when JACK is unavailable.

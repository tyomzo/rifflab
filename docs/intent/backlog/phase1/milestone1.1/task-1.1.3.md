---
id: "1.1.3"
title: "Wire AudioEngine into main.rs"
status: pending
crate: "rifflab-app"
requirement: "—"
---

Initialize `AudioConfig` with sensible defaults (48 kHz, 256-frame buffer), create an `AudioEngine`, and call `start()`. Load a hardcoded test WAV file into a `StemPlayer` and add it to the audio graph so that audio is routed to the output. File: `crates/rifflab-app/src/main.rs`. Acceptance: `cargo run` produces audible audio output through the default backend.

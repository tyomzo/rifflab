---
id: "1.8.7"
title: "Full integration wiring"
status: pending
crate: "rifflab-app"
requirement: "—"
---

Wire all modules together in main.rs: AudioEngine with EffectChain -> YinDetector producing PitchFrames -> Comparator producing ComparisonFrames -> all ring buffers connected to UI app state -> meters, pitch, comparison, transport all flowing. File: crates/rifflab-app/src/main.rs. Acceptance: full practice workflow works end-to-end (import, play, see accuracy).

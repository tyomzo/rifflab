---
id: "2.1.1"
title: "Create cue types and CueEngine struct"
status: pending
crate: "rifflab-cue"
requirement: "FR-M6-01"
---

Create the rifflab-cue crate with core cue types and the CueEngine struct. Define `Cue` as the top-level type containing a position and an action. Define `CuePosition` enum with variants `BarBeat` (bar, beat, tick) and `AbsoluteTime` (sample offset or seconds). Define `CueAction` enum with variants `EffectPresetSwitch`, `LoopRegion`, `SectionMarker`, and `TempoOverride`. Implement `CueEngine` struct that holds a sorted `Vec<Cue>` and tracks the current playback position for crossing detection.

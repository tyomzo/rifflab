---
id: "2.1.3"
title: "Effect preset switch cue dispatch"
status: pending
crate: "rifflab-cue"
requirement: "FR-M6-04"
---

When an `EffectPresetSwitch` cue is crossed during playback, call M2's `crossfade_to_preset()` to smoothly transition the effect chain to the target preset. The cue action carries the target preset identifier. Coordinate with the effect chain crossfade system to ensure glitch-free transitions at the cue point.

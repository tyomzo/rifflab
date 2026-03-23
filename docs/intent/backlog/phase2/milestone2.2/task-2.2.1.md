---
id: "2.2.1"
title: "Parameter crossfade engine"
status: pending
crate: "rifflab-fx"
requirement: "FR-M2-05"
---

Implement a 10ms crossfade on parameter changes: when a parameter value changes, interpolate from the old value to the new value over N samples (calculated from sample rate) to avoid clicks and discontinuities. The crossfade engine tracks pending transitions per parameter and produces smoothed values each audio block. Multiple rapid changes should retarget the interpolation from the current intermediate value.

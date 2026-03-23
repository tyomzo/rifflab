---
id: "1.5.3"
title: "Wire Comparator to PitchFrame stream"
status: pending
crate: "rifflab-practice"
requirement: "FR-M5-02"
---

In the app wiring layer: consume PitchFrame from ring buffer, feed to Comparator.compare() with current SongPosition, push resulting ComparisonFrame to another ring buffer for UI consumption. Acceptance: ComparisonFrames flowing during playback.

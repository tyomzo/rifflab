---
id: "2.1.2"
title: "CueEngine tick and dispatch"
status: pending
crate: "rifflab-cue"
requirement: "FR-M6-03"
---

Implement `tick()` on CueEngine: called once per audio block with the current playback position (sample range for this block). Detect all cue crossings within the range, dispatch the corresponding CueAction for each crossed cue. The tick method must complete within one audio buffer period to avoid xruns. Use a sorted cue list and binary search to efficiently find relevant cues for each block.

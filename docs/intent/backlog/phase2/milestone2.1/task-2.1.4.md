---
id: "2.1.4"
title: "Loop region cue dispatch"
status: pending
crate: "rifflab-cue"
requirement: "FR-M6-05"
---

When a `LoopRegion` cue is crossed during playback, instruct the transport to set loop boundaries (start and end positions). The cue action carries the loop start and end positions. The transport should begin looping between these boundaries until the loop is explicitly cleared or overridden by another LoopRegion cue.

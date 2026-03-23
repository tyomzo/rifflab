---
id: "2.1.5"
title: "Bar/beat cue positioning"
status: pending
crate: "rifflab-cue"
requirement: "FR-M6-02"
---

Resolve `CuePosition::BarBeat` to a sample position using the BeatGrid from the analysis module. Given a bar, beat, and optional tick, compute the exact sample offset. Fall back to `CuePosition::AbsoluteTime` if the BeatGrid is unavailable or the bar/beat reference is out of range. Ensure resolution is recalculated when tempo or BeatGrid changes.

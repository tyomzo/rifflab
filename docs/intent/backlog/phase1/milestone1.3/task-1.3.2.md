---
id: "1.3.2"
title: "Wire BeatTracker to WorkerClient"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4-07"
---

Connect `BeatTracker` to a live `WorkerClient` instance. Submit a `DetectBeats` job, poll for completion, then load the resulting `beat_grid.json` file and deserialize it into the `BeatGrid` struct. Expose a method to retrieve the beat grid after detection completes. File: `crates/rifflab-analysis/src/beats.rs`. Acceptance: beat grid is loaded from the worker output and accessible as a `BeatGrid` struct.

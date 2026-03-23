---
id: "1.3.1"
title: "Write madmom_worker.py"
status: pending
crate: "workers/"
requirement: "FR-M4-07"
---

Create a Python worker for beat detection. It listens on a Unix socket (path provided via `--socket` argument), accepts a connection, and receives length-prefixed MessagePack requests. On a `DetectBeats` request, run the madmom `BeatDetectionProcessor` (with `DBNBeatTrackingProcessor`) on the input audio file. Write the results to `beat_grid.json` in the `BeatGrid` format containing beat timestamps, estimated BPM, and time signature. Send a `JobCompleted` response with the output path. File: `workers/madmom_worker.py`. Acceptance: produces a valid `beat_grid.json` with accurate beat timestamps and BPM for the input audio.

---
id: "1.3.4"
title: "Offline batch pitch detection"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4-05"
---

Run the YIN pitch detection algorithm over a full stem file in batch (offline) mode. Process the audio in chunks, collecting `(time_seconds, frequency_hz, confidence)` tuples for each analysis frame. This complements the existing real-time YIN path by processing an entire file at once for use in transcription and analysis workflows. File: `crates/rifflab-analysis/src/pitch/yin.rs` (add batch method). Acceptance: offline pitch track output matches the real-time pitch detection output when run on the same audio file.

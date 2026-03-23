---
id: "1.5.1"
title: "Integrate YinDetector into audio callback"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4-04"
---

Create a PitchDetectorNode that wraps YinDetector, runs in the audio callback, and pushes PitchFrame to an rtrb ring buffer for consumption by the practice engine and UI. Must be RT-safe (YIN detect() is already allocation-free except for diff/cmnd vectors which need pre-allocation). File: new integration code in rifflab-app or rifflab-audio. Acceptance: PitchFrames flow on ring buffer during playback.

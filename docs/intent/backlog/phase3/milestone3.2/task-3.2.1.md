---
id: "3.2.1"
title: "Polyphonic pitch detection"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4 (Phase 3)"
---

Detect multiple simultaneous pitches (chords) in audio. Consider using HPSS (Harmonic-Percussive Source Separation) as a preprocessing step followed by iterative f0 estimation: detect the strongest fundamental, subtract its harmonics from the spectrum, and repeat. Output multiple concurrent NoteEvents with individual confidence scores.

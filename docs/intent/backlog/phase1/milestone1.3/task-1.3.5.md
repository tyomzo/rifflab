---
id: "1.3.5"
title: "Note transcription (pitch + onset -> NoteEvent)"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4-09"
---

Combine offline pitch detection with onset detection to produce note-level transcription. Segment the pitch track at onset boundaries, compute the median pitch within each segment, determine the closest MIDI note number and cents deviation from equal temperament, and compute note duration from segment length. Output `Vec<NoteEvent>` where each event contains onset time, duration, MIDI note, cents offset, and confidence. File: `crates/rifflab-analysis/src/transcribe.rs`. Acceptance: transcribed notes match a manual annotation on a test monophonic stem within acceptable tolerance.

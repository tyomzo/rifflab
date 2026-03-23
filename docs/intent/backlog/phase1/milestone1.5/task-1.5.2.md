---
id: "1.5.2"
title: "Load reference from transcription or MIDI"
status: pending
crate: "rifflab-practice"
requirement: "FR-M5-01"
---

Wire reference loading in the app: after transcription completes, pass Vec<NoteEvent> to Comparator. Alternatively load from MIDI file via load_from_midi(). File: crates/rifflab-practice/src/reference.rs (already implemented, needs wiring in app). Acceptance: reference notes loaded and queryable by Comparator.

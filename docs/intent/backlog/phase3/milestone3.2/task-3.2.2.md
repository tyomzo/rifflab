---
id: "3.2.2"
title: "MIDI export"
status: pending
crate: "rifflab-analysis"
requirement: "FR-M4 (Phase 3)"
---

Convert `Vec<NoteEvent>` to a Standard MIDI File using the midly crate. Map note pitches to MIDI note numbers, onset/offset times to MIDI ticks, and velocities to MIDI velocity values. Write the resulting SMF to the library directory alongside the song. Support both monophonic and polyphonic note event sequences.

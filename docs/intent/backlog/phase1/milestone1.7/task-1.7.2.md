---
id: "1.7.2"
title: "Piano roll view"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-07"
---

Render piano roll grid: vertical axis = MIDI note numbers (pitch), horizontal axis = time (synchronized with arrangement timeline). Reference notes as semi-transparent rectangles. Played notes (from ComparisonFrame ring buffer) as solid rectangles. Horizontal scroll synced with arrangement. File: crates/rifflab-ui/src/views/piano_roll.rs. Acceptance: reference and played notes visible, scrolls with playhead.

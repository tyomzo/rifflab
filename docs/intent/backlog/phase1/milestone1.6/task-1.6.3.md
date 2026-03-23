---
id: "1.6.3"
title: "Waveform widget"
status: pending
crate: "rifflab-ui"
requirement: "—"
---

Pre-compute multi-resolution waveform overview: for each stem, compute peak values per pixel column at several zoom levels (mipmap). Render as filled line segments in egui Painter. File: crates/rifflab-ui/src/widgets/waveform.rs. Acceptance: waveform renders for any stem at any zoom level without lag.

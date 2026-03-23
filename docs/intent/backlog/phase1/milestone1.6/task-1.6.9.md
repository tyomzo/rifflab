---
id: "1.6.9"
title: "Status bar implementation"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-08"
---

Replace placeholder status bar: show audio latency (buffer_size/sample_rate), CPU usage (callback duration / buffer period), current pitch (note name + cents from PitchFrame ring buffer), running session score. File: crates/rifflab-ui/src/views/status_bar.rs. Acceptance: all metrics update in real time during playback.

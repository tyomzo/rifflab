---
id: "1.6.1"
title: "Implement toolbar with transport controls"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-05"
---

Replace placeholder toolbar with functional controls: Play/Pause/Stop buttons wired to AudioEngine.transport, BPM display (from beat grid), song selector (native file dialog via rfd or egui), loop toggle button. File: crates/rifflab-ui/src/views/toolbar.rs. Acceptance: buttons control playback, BPM displays, file dialog opens.

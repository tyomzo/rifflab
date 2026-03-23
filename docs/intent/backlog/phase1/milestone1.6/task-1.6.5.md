---
id: "1.6.5"
title: "Sidebar with track mixer"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-04"
---

Per-track controls in sidebar: track name, color indicator, Solo (S) toggle, Mute (M) toggle, volume slider. Input track with FX bypass toggle. Wire to AudioGraph stem_volumes/mutes/solos. File: crates/rifflab-ui/src/views/sidebar.rs. Acceptance: solo/mute/volume changes affect audio output.

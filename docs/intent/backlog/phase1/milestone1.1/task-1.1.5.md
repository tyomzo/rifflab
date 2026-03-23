---
id: "1.1.5"
title: "Transport play/pause/stop/seek test"
status: pending
crate: "rifflab-audio"
requirement: "FR-M1-05"
---

Write an integration test exercising all transport states. Load audio into the engine, then: play and verify the playback position advances over time; pause and verify the position holds steady; seek to a known frame and verify the position matches; stop and verify the position resets to 0. Acceptance: all four transport states are verified programmatically.

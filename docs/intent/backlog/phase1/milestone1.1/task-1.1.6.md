---
id: "1.1.6"
title: "Loop region integration test"
status: pending
crate: "rifflab-audio"
requirement: "FR-M1-09"
---

Write a test that sets a loop region with defined start and end points, plays through, and verifies the transport wraps from the end position back to the start position. Confirm the position is continuous and that no audio glitch occurs at the loop boundary. Acceptance: loop wraps correctly and position reports are continuous across the boundary.

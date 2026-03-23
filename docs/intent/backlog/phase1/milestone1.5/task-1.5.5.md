---
id: "1.5.5"
title: "Timing offset computation"
status: pending
crate: "rifflab-practice"
requirement: "FR-M5-04"
---

Implement onset timing comparison: track when the player starts a new note (pitch change or silence->pitch), compare onset time against reference note onset. Compute timing_offset_ms. Update ComparisonFrame. File: crates/rifflab-practice/src/compare.rs. Acceptance: timing_offset_ms is non-zero and accurate within +/-5ms.

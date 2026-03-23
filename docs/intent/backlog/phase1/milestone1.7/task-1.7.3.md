---
id: "1.7.3"
title: "Piano roll color coding"
status: pending
crate: "rifflab-ui"
requirement: "FR-M7-07"
---

Color played notes by accuracy: green (correct + in tune, <=5 cents), yellow (correct + slightly off, <=15 cents), orange (correct + significantly off, <=25 cents), red (wrong note), grey outline (missed -- reference note with no matching played note). Use AccuracyBucket from ComparisonFrame. File: crates/rifflab-ui/src/views/piano_roll.rs. Acceptance: colors match accuracy buckets.

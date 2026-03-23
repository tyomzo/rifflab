---
id: "2.5.2"
title: "Pass model selection through separator"
status: pending
crate: "rifflab-stems"
requirement: "FR-M3-02"
---

Update StemSeparator to pass the selected DemucsModel variant to the worker process. Cache separated stems keyed by both model and song ID, so switching between 4-stem and 6-stem results for the same song does not require re-separation. Validate that the requested model is available before starting separation.

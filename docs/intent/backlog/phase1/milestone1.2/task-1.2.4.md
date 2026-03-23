---
id: "1.2.4"
title: "Verify stem caching"
status: pending
crate: "rifflab-stems"
requirement: "FR-M3-05"
---

Test that re-importing the same song with the same model and parameters skips the separation step because stems already exist in the library cache. The cache key should incorporate the source file hash and model identifier. File: `crates/rifflab-stems/src/cache.rs`. Acceptance: the second import of the same file returns immediately without spawning a worker process.

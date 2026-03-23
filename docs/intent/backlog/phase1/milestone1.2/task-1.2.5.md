---
id: "1.2.5"
title: "Job cancellation"
status: pending
crate: "rifflab-stems"
requirement: "FR-M3-06"
---

Implement `cancel()` on an in-flight separation job. Send a `Cancel` request to the worker via the IPC channel, verify the worker acknowledges and stops processing, and clean up any partial output files that may have been written. Acceptance: cancelling mid-separation stops the worker and leaves no partial stem files on disk.

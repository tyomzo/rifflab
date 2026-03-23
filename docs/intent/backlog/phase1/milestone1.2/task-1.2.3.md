---
id: "1.2.3"
title: "Wire StemSeparator to WorkerClient"
status: pending
crate: "rifflab-stems"
requirement: "FR-M3-04"
---

Connect `StemSeparator` to a live `WorkerClient` instance. When `separate()` is called, submit a separation job to the worker, poll the progress channel for status updates (forwarding them to any registered progress callback), and handle completion by verifying the expected stem files exist on disk. File: `crates/rifflab-stems/src/separator.rs`. Acceptance: end-to-end stem separation initiated from Rust code produces valid stem files.

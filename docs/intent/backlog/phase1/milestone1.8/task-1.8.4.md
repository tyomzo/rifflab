---
id: "1.8.4"
title: "Worker process management"
status: pending
crate: "rifflab-app"
requirement: "—"
---

On startup: spawn demucs_worker.py and madmom_worker.py as long-lived daemon processes. Connect WorkerClients. On shutdown: send Shutdown command, wait for process exit with timeout, force kill if needed. Handle crash recovery (detect broken pipe, respawn). File: crates/rifflab-app/src/workers.rs (new). Acceptance: workers start/stop cleanly, crash recovery works.

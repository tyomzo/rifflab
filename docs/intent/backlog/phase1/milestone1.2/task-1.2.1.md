---
id: "1.2.1"
title: "Implement WorkerClient IPC"
status: pending
crate: "rifflab-ipc"
requirement: "FR-M3-01"
---

Implement the full `WorkerClient`: spawn the Python worker process, connect via a tokio `UnixStream`, and communicate using length-prefixed MessagePack encoding. Send requests as serialized MessagePack with a 4-byte big-endian length prefix. Run a background reader task that reads incoming responses and dispatches them to per-job oneshot or mpsc channels keyed by job ID. File: `crates/rifflab-ipc/src/client.rs`. Acceptance: send a request to the worker and receive a valid response over the Unix socket.

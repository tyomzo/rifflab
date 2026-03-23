---
id: "1.2.2"
title: "Write demucs_worker.py"
status: pending
crate: "workers/"
requirement: "FR-M3-01"
---

Create the Python worker process for stem separation. It listens on a Unix socket (path provided via `--socket` argument), accepts a single connection, and receives length-prefixed MessagePack `WorkerRequest` messages. On a `SeparateStems` request, run htdemucs on the input file using the torch hub or demucs CLI. Emit `Progress` responses during inference to report percentage complete. Write the resulting stem WAV files (vocals, drums, bass, other) to the specified output directory. Send a `JobCompleted` response with the stem file paths. Handle errors gracefully by sending a `JobFailed` response. File: `workers/demucs_worker.py`. Acceptance: receives a separation request, produces 4 stem files, and sends progress updates followed by a completion message.

---
id: "1.8.1"
title: "CLI argument parsing"
status: pending
crate: "rifflab-app"
requirement: "—"
---

Use clap derive to parse: optional positional song file path, --config path, --library path, --backend (jack|alsa|auto). File: crates/rifflab-app/src/main.rs. Acceptance: `rifflab song.mp3` and `rifflab --backend alsa` work.

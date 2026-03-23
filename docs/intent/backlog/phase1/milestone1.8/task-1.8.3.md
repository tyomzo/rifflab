---
id: "1.8.3"
title: "Config file loading"
status: pending
crate: "rifflab-app"
requirement: "—"
---

Load config from ~/.config/rifflab/config.toml (XDG config dir). Merge with CLI args (CLI takes precedence). Write default config if file doesn't exist. Fields: audio.backend, audio.sample_rate, audio.buffer_size, library.path, ui.theme. File: crates/rifflab-app/src/config.rs (new). Acceptance: config loads, defaults applied, CLI overrides work.

---
id: "1.8.2"
title: "Library directory and SQLite init"
status: pending
crate: "rifflab-app"
requirement: "—"
---

On startup: create library directory structure at XDG data dir (~/.local/share/rifflab/) using directories crate. Create songs/, effects/, presets/, sessions/ subdirs. Open/create rifflab.db with SQLite schema (songs, sessions, note_results tables). File: crates/rifflab-app/src/library.rs (new). Acceptance: directories created, DB schema initialized on first run.

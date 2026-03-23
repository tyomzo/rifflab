---
id: "1.8.5"
title: "Import pipeline"
status: pending
crate: "rifflab-app"
requirement: "—"
---

Full import flow: decode audio (symphonia) -> generate song_id -> copy to library/songs/{id}/original.wav -> trigger stem separation -> on completion, trigger beat tracking -> on completion, trigger note transcription on selected stem -> persist metadata.json -> load stems into AudioGraph -> load beat grid -> load reference notes. File: crates/rifflab-app/src/import.rs (new). Acceptance: end-to-end import from MP3 to ready-to-practice state.

---
id: "1.1.4"
title: "Symphonia audio file decoder"
status: pending
crate: "rifflab-app"
requirement: "—"
---

Create a decode module using the symphonia crate to decode WAV, FLAC, and MP3 files into `Vec<f32>` interleaved audio samples. Return a struct containing the sample data, sample rate, and channel count. Handle format probing, codec selection, and packet decoding internally. File: `crates/rifflab-app/src/decode.rs` (new). Acceptance: any common audio format (WAV, FLAC, MP3) is decoded to f32 samples with correct sample rate and channel info.

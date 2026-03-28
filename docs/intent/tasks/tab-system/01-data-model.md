# Task 01: Tab Data Model Crate

## Goal
Create `rifflab-tab` crate with core data types from spec sections 2.1–2.3.

## Deliverables
1. New crate `crates/rifflab-tab/` with `Cargo.toml` (deps: serde, serde_json, uuid, chrono)
2. `src/lib.rs` — re-exports
3. `src/model.rs` — all types:
   - `TabNote` (id, string, fret, time_secs, duration_secs, beat, measure, technique, confidence, source)
   - `Technique` enum (Normal, HammerOn, PullOff, SlideUp, SlideDown, Slap, Pop, Mute, Bend, Vibrato, Ghost, Harmonic, TapOn)
   - `NoteSource` enum (AsciiParse, AudioTranscription, Fused, UserEdit)
   - `TabDocument` (id, title, artist, tuning, tempo, time_signature, notes, measures, provenance)
   - `TempoMap`, `TimeSignature`, `MeasureMarker`, `TabProvenance`, `PipelineMode`
4. `src/io.rs` — save/load `TabDocument` as `.rltab` JSON files
5. Basic tests: round-trip serialization, default construction

## Files
- `crates/rifflab-tab/Cargo.toml`
- `crates/rifflab-tab/src/lib.rs`
- `crates/rifflab-tab/src/model.rs`
- `crates/rifflab-tab/src/io.rs`

## Verification
- `cargo build --workspace` passes
- `cargo test -p rifflab-tab` passes

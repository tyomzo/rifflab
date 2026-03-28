# Task 05: Audio-to-Tab Transcription Pipeline

## Goal
Implement the audio transcription pipeline from spec section 3.3. Convert a bass audio stem to timed TabNote events using onset detection + pitch estimation (YIN fallback first, Basic Pitch ONNX later).

## Deliverables
1. `crates/rifflab-tab/src/audio.rs` — audio transcription module
2. Structs:
   - `TranscribedNote { onset_secs, offset_secs, midi_note, confidence }`
   - `BeatGrid { bpm, beat_times, downbeat_indices }`
3. Functions:
   - `transcribe_audio(audio_path: &Path, tuning: &[u8; 4]) -> Result<Vec<TabNote>>`
   - Step 1: Load audio (reuse rifflab decode or symphonia)
   - Step 2: Onset detection (spectral flux, simple peak picker — reuse rifflab-analysis if possible)
   - Step 3: Per-onset pitch estimation (YIN from rifflab-analysis)
   - Step 4: Note segmentation (onset to next onset or silence = one note)
   - Step 5: `midi_to_fret()` mapping (task 04)
   - Step 6: Beat tracking (simple autocorrelation BPM estimator or reuse existing)
   - Step 7: Assign beat/measure to each note based on beat grid
4. Reuse existing Demucs stem separation if input is a full mix (check for cached stems)
5. Tests:
   - Synthetic test: generate known sine tones at bass frequencies, verify correct pitch/timing
   - Integration: use `test_tone.wav` if applicable

## Dependencies
- Task 01 (data model)
- Task 04 (fret mapper)
- Existing `rifflab-analysis` (YIN pitch detector, onset detection)
- Existing `rifflab-stems` (Demucs, for full-mix input)

## Files
- `crates/rifflab-tab/src/audio.rs`

## Notes
- YIN fallback is the v1 approach. Basic Pitch ONNX (spec section 3.3.3) is Phase 2.
- Onset detection: spectral flux with adaptive threshold, ~10ms resolution
- Run on background thread (not audio RT thread)

## Verification
- `cargo test -p rifflab-tab` passes
- Can transcribe a known bass stem and get reasonable note events

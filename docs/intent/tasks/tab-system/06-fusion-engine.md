# Task 06: Alignment & Fusion Engine (DTW)

## Goal
Implement the fusion engine from spec section 3.4. When both ASCII-parsed and audio-transcribed notes exist, align them using DTW and merge into a high-accuracy TabDocument.

## Deliverables
1. `crates/rifflab-tab/src/fusion.rs` — alignment and fusion module
2. Functions:
   - `fuse(ascii_notes: &[TabNote], audio_notes: &[TabNote], tuning: &[u8; 4]) -> Vec<TabNote>`
   - DTW implementation:
     - Convert both sequences to pitch (tuning[string] + fret for ASCII, midi_note for audio)
     - Distance metric: absolute pitch difference in semitones
     - Sakoe-Chiba band constraint (window = ±10% of sequence length)
     - Output: alignment mapping ascii_index → audio_index
   - Merge aligned pairs:
     - time_secs/duration_secs from audio (accurate timing)
     - string/fret from ASCII (explicit fingering)
     - technique from ASCII
     - confidence = average of both + 0.1 if pitch matches exactly
     - source = NoteSource::Fused
   - Handle unmatched notes:
     - Audio-only: include with AudioTranscription source, confidence × 0.7
     - ASCII-only: include with AsciiParse source, confidence = 0.3
   - Conflict resolution: if pitches differ > 2 semitones, flag for user review
3. `build_tab_document()` — assemble final TabDocument from any pipeline mode
4. Tests:
   - Perfect alignment: identical pitch sequences, verify merge
   - Shifted alignment: ASCII has extra notes, verify DTW handles it
   - Octave error: audio detects wrong octave, ASCII corrects

## Dependencies
- Task 01 (data model)
- Task 04 (fret mapper — for pitch computation)

## Files
- `crates/rifflab-tab/src/fusion.rs`

## Verification
- `cargo test -p rifflab-tab` passes

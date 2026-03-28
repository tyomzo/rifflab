# Task 04: Pitch-to-Fret Mapper

## Goal
Implement the pitch-to-fret mapping from spec section 3.3.5. Convert MIDI note numbers to (string, fret) pairs on bass guitar.

## Deliverables
1. `crates/rifflab-tab/src/fret_map.rs` — fret mapping module
2. Functions:
   - `midi_to_fret(midi_note: u8, tuning: &[u8; 4], prev_fret: Option<(u8, u8)>) -> (u8, u8)`
     - Returns (string, fret) choosing position that minimizes hand movement from `prev_fret`
     - Prefers lower positions (closer to nut) when tied
   - `all_positions(midi_note: u8, tuning: &[u8; 4]) -> Vec<(u8, u8)>` — list all valid string/fret combos for a pitch
   - `fret_to_midi(string: u8, fret: u8, tuning: &[u8; 4]) -> u8` — reverse mapping
3. Standard tuning constants:
   - `STANDARD_TUNING: [u8; 4] = [28, 33, 38, 43]` (E1, A1, D2, G2)
   - `DROP_D_TUNING: [u8; 4] = [26, 33, 38, 43]` (D1, A1, D2, G2)
4. Tests:
   - Known positions: E1 open = (0, 0), A2 = (1, 0) or (0, 5)
   - Min-movement greedy: sequential notes prefer staying in position
   - Drop D tuning edge cases
   - Max fret = 24

## Dependencies
- Task 01 (data model)

## Files
- `crates/rifflab-tab/src/fret_map.rs`

## Verification
- `cargo test -p rifflab-tab` passes

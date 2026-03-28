#[cfg(test)]
use crate::model::STANDARD_TUNING;

/// All valid (string, fret) positions for a given MIDI note on bass.
pub fn all_positions(midi_note: u8, tuning: &[u8; 4]) -> Vec<(u8, u8)> {
    let mut positions = Vec::new();
    for (string, &open) in tuning.iter().enumerate() {
        if midi_note >= open {
            let fret = midi_note - open;
            if fret <= 24 {
                positions.push((string as u8, fret));
            }
        }
    }
    positions
}

/// Choose the (string, fret) position that minimizes hand movement from `prev`.
/// Prefers lower positions (closer to nut) when tied.
pub fn midi_to_fret(midi_note: u8, tuning: &[u8; 4], prev: Option<(u8, u8)>) -> (u8, u8) {
    let positions = all_positions(midi_note, tuning);
    if positions.is_empty() {
        return (0, 0); // out of range fallback
    }
    if positions.len() == 1 {
        return positions[0];
    }
    match prev {
        Some((_, prev_fret)) => {
            // Minimize distance from previous fret position
            *positions.iter()
                .min_by_key(|&&(_, fret)| {
                    let dist = (fret as i16 - prev_fret as i16).unsigned_abs();
                    // Tie-break: prefer lower fret
                    (dist, fret)
                })
                .unwrap()
        }
        None => {
            // No previous — prefer lowest position
            *positions.iter().min_by_key(|&&(_, fret)| fret).unwrap()
        }
    }
}

/// Reverse mapping: (string, fret) → MIDI note number.
pub fn fret_to_midi(string: u8, fret: u8, tuning: &[u8; 4]) -> u8 {
    tuning[string as usize] + fret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DROP_D_TUNING;

    #[test]
    fn open_strings_standard() {
        // E1 open = string 0 fret 0
        assert_eq!(all_positions(28, &STANDARD_TUNING), vec![(0, 0)]);
        // A1 open = string 1 fret 0, or string 0 fret 5
        let positions = all_positions(33, &STANDARD_TUNING);
        assert!(positions.contains(&(1, 0)));
        assert!(positions.contains(&(0, 5)));
    }

    #[test]
    fn min_movement_greedy() {
        // Playing around fret 7: G2 (43) on string 2 fret 5, then A2 (45)
        // A2 can be string 2 fret 7 or string 1 fret 12 or string 3 fret 2
        // Should prefer string 2 fret 7 (closest to fret 5)
        let pos = midi_to_fret(45, &STANDARD_TUNING, Some((2, 5)));
        assert_eq!(pos, (2, 7));
    }

    #[test]
    fn no_prev_prefers_low_fret() {
        // G2 (43) = string 3 fret 0, string 2 fret 5, string 1 fret 10, string 0 fret 15
        let pos = midi_to_fret(43, &STANDARD_TUNING, None);
        assert_eq!(pos, (3, 0)); // open G string
    }

    #[test]
    fn drop_d_tuning() {
        // D1 (26) on drop D = string 0 fret 0
        assert_eq!(all_positions(26, &DROP_D_TUNING), vec![(0, 0)]);
        // E1 (28) on drop D = string 0 fret 2
        let positions = all_positions(28, &DROP_D_TUNING);
        assert!(positions.contains(&(0, 2)));
    }

    #[test]
    fn reverse_mapping() {
        assert_eq!(fret_to_midi(0, 0, &STANDARD_TUNING), 28); // E1
        assert_eq!(fret_to_midi(1, 5, &STANDARD_TUNING), 38); // D2
        assert_eq!(fret_to_midi(3, 12, &STANDARD_TUNING), 55); // G3
    }

    #[test]
    fn max_fret_limit() {
        // MIDI note 67 = G4, on G string (43) would need fret 24
        let positions = all_positions(67, &STANDARD_TUNING);
        assert!(positions.iter().any(|&(s, f)| s == 3 && f == 24));
        // MIDI note 68 = G#4, on G string would need fret 25 → out of range
        let positions = all_positions(68, &STANDARD_TUNING);
        assert!(!positions.iter().any(|&(s, _)| s == 3));
    }
}

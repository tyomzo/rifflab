//! Alignment & Fusion Engine using Dynamic Time Warping.
//!
//! When both ASCII-parsed and audio-transcribed notes exist, align them
//! and merge into a high-accuracy TabDocument.

use crate::model::{NoteSource, TabNote};
use uuid::Uuid;

/// Fuse ASCII-parsed and audio-transcribed notes using DTW alignment.
///
/// - Timing from audio (accurate)
/// - String/fret/technique from ASCII (explicit)
/// - Confidence boosted when both agree
pub fn fuse(
    ascii_notes: &[TabNote],
    audio_notes: &[TabNote],
    tuning: &[u8; 4],
) -> Vec<TabNote> {
    use crate::constants::{MIN_CONFIDENCE, CONFIDENCE_DAMPING};

    if ascii_notes.is_empty() {
        return audio_notes.iter().map(|n| {
            let mut n = n.clone();
            n.confidence *= CONFIDENCE_DAMPING;
            n
        }).collect();
    }
    if audio_notes.is_empty() {
        return ascii_notes.iter().map(|n| {
            let mut n = n.clone();
            n.confidence = MIN_CONFIDENCE;
            n
        }).collect();
    }

    // Convert to pitch sequences for DTW
    let ascii_pitches: Vec<u8> = ascii_notes.iter()
        .map(|n| n.midi_note(tuning))
        .collect();
    let audio_pitches: Vec<u8> = audio_notes.iter()
        .map(|n| n.midi_note(tuning))
        .collect();

    // Run DTW to find alignment
    let alignment = dtw_align(&ascii_pitches, &audio_pitches);

    // Track which notes have been matched
    let mut ascii_matched = vec![false; ascii_notes.len()];
    let mut audio_matched = vec![false; audio_notes.len()];
    let mut result = Vec::new();

    // Merge aligned pairs
    for (ai, bi) in &alignment {
        ascii_matched[*ai] = true;
        audio_matched[*bi] = true;

        let ascii = &ascii_notes[*ai];
        let audio = &audio_notes[*bi];
        let pitch_diff = (ascii_pitches[*ai] as i16 - audio_pitches[*bi] as i16).unsigned_abs();

        let mut fused = TabNote {
            id: Uuid::new_v4(),
            // String/fret from ASCII (explicit fingering)
            string: ascii.string,
            fret: ascii.fret,
            // Timing from audio (accurate)
            time_secs: audio.time_secs,
            duration_secs: audio.duration_secs,
            beat: audio.beat,
            measure: audio.measure,
            // Technique from ASCII
            technique: ascii.technique,
            // Confidence: average + bonus if pitch matches
            confidence: (ascii.confidence + audio.confidence) / 2.0
                + if pitch_diff == 0 { 0.1 } else { 0.0 },
            source: NoteSource::Fused,
            section: ascii.section.clone(),
        };
        fused.confidence = fused.confidence.min(1.0);

        // If pitches differ by more than 2 semitones, prefer audio pitch
        // (ASCII might have wrong octave or transcription error)
        if pitch_diff > 2 {
            fused.string = audio.string;
            fused.fret = audio.fret;
            fused.confidence *= 0.6; // Lower confidence for conflict
        }

        result.push(fused);
    }

    // Add unmatched audio notes (with lower confidence)
    for (i, note) in audio_notes.iter().enumerate() {
        if !audio_matched[i] {
            let mut n = note.clone();
            n.source = NoteSource::AudioTranscription;
            n.confidence *= CONFIDENCE_DAMPING;
            result.push(n);
        }
    }

    // Add unmatched ASCII notes (timing unknown, low confidence)
    for (i, note) in ascii_notes.iter().enumerate() {
        if !ascii_matched[i] {
            let mut n = note.clone();
            n.source = NoteSource::AsciiParse;
            n.confidence = MIN_CONFIDENCE;
            result.push(n);
        }
    }

    // Sort by time
    result.sort_by(|a, b| a.time_secs.partial_cmp(&b.time_secs).unwrap_or(std::cmp::Ordering::Equal));
    result
}

/// DTW alignment: returns pairs of (ascii_index, audio_index).
fn dtw_align(seq_a: &[u8], seq_b: &[u8]) -> Vec<(usize, usize)> {
    let n = seq_a.len();
    let m = seq_b.len();
    if n == 0 || m == 0 {
        return Vec::new();
    }

    // Sakoe-Chiba band: window = 10% of max sequence length
    let window = ((n.max(m) as f64 * 0.1).ceil() as usize).max(crate::constants::DTW_MIN_WINDOW);

    // Cost matrix (use f32 to save memory)
    let mut cost = vec![vec![f32::INFINITY; m + 1]; n + 1];
    cost[0][0] = 0.0;

    for i in 1..=n {
        let j_min = if i > window { i - window } else { 1 };
        let j_max = (i + window).min(m);
        for j in j_min..=j_max {
            let d = (seq_a[i - 1] as f32 - seq_b[j - 1] as f32).abs();
            cost[i][j] = d + cost[i - 1][j - 1]
                .min(cost[i - 1][j])
                .min(cost[i][j - 1]);
        }
    }

    // Backtrack to find alignment path
    let mut path = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 && j > 0 {
        path.push((i - 1, j - 1));
        let diag = cost[i - 1][j - 1];
        let up = cost[i - 1][j];
        let left = cost[i][j - 1];
        if diag <= up && diag <= left {
            i -= 1;
            j -= 1;
        } else if up <= left {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    path.reverse();

    // Deduplicate: keep only 1:1 matches (first occurrence of each index)
    let mut seen_a = vec![false; n];
    let mut seen_b = vec![false; m];
    let mut unique = Vec::new();
    for (ai, bi) in path {
        if !seen_a[ai] && !seen_b[bi] {
            seen_a[ai] = true;
            seen_b[bi] = true;
            unique.push((ai, bi));
        }
    }

    unique
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn make_note(string: u8, fret: u8, time: f64, source: NoteSource) -> TabNote {
        let mut n = TabNote::new(string, fret, source);
        n.time_secs = time;
        n.duration_secs = 0.25;
        n.confidence = 0.8;
        n
    }

    #[test]
    fn fuse_identical_sequences() {
        let tuning = STANDARD_TUNING;
        let ascii = vec![
            make_note(0, 0, 0.0, NoteSource::AsciiParse), // E1
            make_note(1, 0, 1.0, NoteSource::AsciiParse), // A1
            make_note(2, 0, 2.0, NoteSource::AsciiParse), // D2
        ];
        let audio = vec![
            make_note(0, 0, 0.1, NoteSource::AudioTranscription),
            make_note(1, 0, 1.1, NoteSource::AudioTranscription),
            make_note(2, 0, 2.1, NoteSource::AudioTranscription),
        ];

        let fused = fuse(&ascii, &audio, &tuning);
        assert_eq!(fused.len(), 3);
        for n in &fused {
            assert_eq!(n.source, NoteSource::Fused);
            assert!(n.confidence > 0.8); // Boosted by pitch match
        }
        // Timing from audio
        assert!((fused[0].time_secs - 0.1).abs() < 0.01);
    }

    #[test]
    fn fuse_ascii_only() {
        let ascii = vec![make_note(0, 5, 0.0, NoteSource::AsciiParse)];
        let audio: Vec<TabNote> = vec![];
        let fused = fuse(&ascii, &audio, &STANDARD_TUNING);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].confidence, 0.3);
    }

    #[test]
    fn fuse_audio_only() {
        let ascii: Vec<TabNote> = vec![];
        let audio = vec![make_note(1, 3, 0.5, NoteSource::AudioTranscription)];
        let fused = fuse(&ascii, &audio, &STANDARD_TUNING);
        assert_eq!(fused.len(), 1);
        assert!(fused[0].confidence < 0.8 * 0.71); // 0.7 factor
    }

    #[test]
    fn dtw_handles_different_lengths() {
        let a = vec![28, 33, 38, 43, 45]; // 5 notes
        let b = vec![28, 33, 38]; // 3 notes
        let alignment = dtw_align(&a, &b);
        // Should align first 3 notes, leave 2 unmatched in a
        assert!(alignment.len() >= 3);
    }
}

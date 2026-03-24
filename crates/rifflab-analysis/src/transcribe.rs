use rifflab_core::analysis::NoteEvent;

use crate::onset::detect_onsets;
use crate::pitch::yin::YinDetector;

/// Combine offline pitch detection + onset detection into discrete NoteEvents.
///
/// Pipeline:
/// 1. Run YIN pitch detection over the full buffer to get a pitch track
/// 2. Run onset detection to get onset boundary times
/// 3. Segment the pitch track at onset boundaries
/// 4. For each segment, compute median pitch, map to MIDI, compute duration and cents deviation
/// 5. Filter out low-confidence or very short segments
pub fn transcribe_notes(samples: &[f32], sample_rate: u32) -> Vec<NoteEvent> {
    if samples.is_empty() {
        return Vec::new();
    }

    // Step 1: Offline pitch detection
    let mut detector = YinDetector::new(sample_rate);
    let pitch_track = detector.detect_batch(samples, sample_rate);

    if pitch_track.is_empty() {
        return Vec::new();
    }

    // Step 2: Onset detection
    let mut onset_times = detect_onsets(samples, sample_rate);

    // Ensure we have a start boundary at 0.0
    if onset_times.is_empty() || onset_times[0] > 0.01 {
        onset_times.insert(0, 0.0);
    }

    // Add end-of-file as final boundary
    let total_duration = samples.len() as f64 / sample_rate as f64;
    onset_times.push(total_duration);

    // Step 3: Segment pitch track at onset boundaries and compute note events
    let min_duration = 0.05; // 50ms minimum note duration
    let min_confidence = 0.3; // Minimum average confidence
    let mut notes = Vec::new();

    for window in onset_times.windows(2) {
        let seg_start = window[0];
        let seg_end = window[1];
        let duration = seg_end - seg_start;

        if duration < min_duration {
            continue;
        }

        // Collect pitch frames within this segment
        let segment_frames: Vec<(f32, f32)> = pitch_track
            .iter()
            .filter(|&&(t, _, _)| t >= seg_start && t < seg_end)
            .map(|&(_, freq, conf)| (freq, conf))
            .collect();

        if segment_frames.is_empty() {
            continue;
        }

        // Filter to voiced frames (frequency > 0 and reasonable confidence)
        let voiced: Vec<(f32, f32)> = segment_frames
            .iter()
            .filter(|&&(freq, conf)| freq > 20.0 && conf > 0.2)
            .cloned()
            .collect();

        if voiced.is_empty() {
            continue;
        }

        // Compute average confidence
        let avg_confidence: f32 =
            voiced.iter().map(|&(_, c)| c).sum::<f32>() / voiced.len() as f32;

        if avg_confidence < min_confidence {
            continue;
        }

        // Compute median pitch (in Hz)
        let mut freqs: Vec<f32> = voiced.iter().map(|&(f, _)| f).collect();
        freqs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_freq = freqs[freqs.len() / 2];

        // Convert to MIDI note and cents
        let midi_float = 69.0 + 12.0 * (median_freq / 440.0).log2();
        let midi_note = midi_float.round() as u8;
        let cents = (midi_float - midi_note as f32) * 100.0;

        notes.push(NoteEvent {
            midi_note,
            onset_seconds: seg_start,
            duration_seconds: duration,
            average_cents: cents,
            confidence: avg_confidence,
        });
    }

    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a sine wave at a given frequency for a given duration.
    fn generate_tone(freq: f32, duration_secs: f64, sample_rate: u32, offset: usize) -> Vec<f32> {
        let num_samples = (sample_rate as f64 * duration_secs) as usize;
        (0..num_samples)
            .map(|i| {
                let t = (offset + i) as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8
            })
            .collect()
    }

    #[test]
    fn test_transcribe_three_tones() {
        let sample_rate = 48000u32;
        let tone_duration = 0.5; // 500ms per tone
        let gap_duration = 0.1; // 100ms silence between tones

        // C4=261.63Hz, E4=329.63Hz, G4=392.00Hz
        let freqs = [261.63f32, 329.63, 392.00];
        let expected_midi = [60u8, 64, 67];

        let gap_samples = (sample_rate as f64 * gap_duration) as usize;
        let mut samples = Vec::new();
        let mut offset = 0;

        // Small leading silence
        let lead_silence = vec![0.0f32; (sample_rate as f64 * 0.1) as usize];
        samples.extend_from_slice(&lead_silence);
        offset += lead_silence.len();

        for &freq in &freqs {
            let tone = generate_tone(freq, tone_duration, sample_rate, offset);
            offset += tone.len();
            samples.extend_from_slice(&tone);

            // Add gap
            let gap = vec![0.0f32; gap_samples];
            offset += gap.len();
            samples.extend_from_slice(&gap);
        }

        // Add trailing silence
        let trailing = vec![0.0f32; (sample_rate as f64 * 0.1) as usize];
        samples.extend_from_slice(&trailing);

        let notes = transcribe_notes(&samples, sample_rate);

        // We should get at least 3 notes (possibly more due to onset detection nuances)
        assert!(
            notes.len() >= 3,
            "Expected at least 3 notes, got {}: {:?}",
            notes.len(),
            notes
        );

        // Check that each expected MIDI note appears in the results
        for &expected in &expected_midi {
            let found = notes.iter().any(|n| {
                // Allow ±1 semitone tolerance for the test
                (n.midi_note as i32 - expected as i32).unsigned_abs() <= 1
            });
            assert!(
                found,
                "Expected MIDI note {expected} not found in results: {:?}",
                notes.iter().map(|n| n.midi_note).collect::<Vec<_>>()
            );
        }

        // Check durations are reasonable (at least 50ms)
        for note in &notes {
            assert!(
                note.duration_seconds >= 0.05,
                "Note duration too short: {}",
                note.duration_seconds
            );
        }
    }

    #[test]
    fn test_transcribe_empty() {
        let notes = transcribe_notes(&[], 44100);
        assert!(notes.is_empty());
    }

    #[test]
    fn test_transcribe_silence() {
        let samples = vec![0.0f32; 48000]; // 1 second of silence
        let notes = transcribe_notes(&samples, 48000);
        assert!(
            notes.is_empty(),
            "Silence should produce no notes, got: {:?}",
            notes
        );
    }
}

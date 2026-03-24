use rifflab_core::analysis::{NoteEvent, PitchFrame};
use rifflab_core::practice::{AccuracyBucket, ComparisonFrame, TimingBucket};
use rifflab_core::transport::SongPosition;

/// Minimum confidence threshold below which we treat a frame as silence.
const CONFIDENCE_THRESHOLD: f32 = 0.5;

/// Real-time comparison engine.
/// Compares live PitchFrame input against a reference note sequence.
///
/// Tracks note onsets: detects when the player transitions from silence
/// (or a different note) to a new note, recording the onset time so we
/// can compute timing offset against the reference.
pub struct Comparator {
    reference: Vec<NoteEvent>,
    /// The MIDI note that was active in the previous frame (0 = silence).
    prev_midi_note: u8,
    /// Whether the previous frame was detected as silence / no pitch.
    prev_was_silent: bool,
    /// The song-position (seconds) at which the current played note began.
    /// `None` if the player is currently silent.
    current_onset_seconds: Option<f64>,
    /// Index of the last matched reference note (for dedup / tracking).
    last_matched_ref_idx: Option<usize>,
    /// Onset time recorded per reference note index, so each note only
    /// gets one timing measurement even across many frames.
    onset_recorded: std::collections::HashMap<usize, f64>,
}

impl Comparator {
    pub fn new(reference: Vec<NoteEvent>) -> Self {
        Self {
            reference,
            prev_midi_note: 0,
            prev_was_silent: true,
            current_onset_seconds: None,
            last_matched_ref_idx: None,
            onset_recorded: std::collections::HashMap::new(),
        }
    }

    /// Return a reference to the loaded reference note sequence.
    pub fn reference_notes(&self) -> &[NoteEvent] {
        &self.reference
    }

    /// Reset tracker state (e.g. when transport seeks or stops).
    pub fn reset(&mut self) {
        self.prev_midi_note = 0;
        self.prev_was_silent = true;
        self.current_onset_seconds = None;
        self.last_matched_ref_idx = None;
        self.onset_recorded.clear();
    }

    /// Compare the current pitch frame against the reference at the given song position.
    pub fn compare(&mut self, frame: &PitchFrame, position: &SongPosition) -> ComparisonFrame {
        let current_time = position.seconds();

        // --- Onset detection ---
        let is_silent = frame.frequency_hz <= 0.0 || frame.confidence < CONFIDENCE_THRESHOLD;
        let is_new_note = if is_silent {
            false
        } else {
            self.prev_was_silent || frame.midi_note != self.prev_midi_note
        };

        if is_new_note {
            self.current_onset_seconds = Some(current_time);
        } else if is_silent {
            self.current_onset_seconds = None;
        }

        // Update previous-frame state
        if is_silent {
            self.prev_was_silent = true;
            self.prev_midi_note = 0;
        } else {
            self.prev_was_silent = false;
            self.prev_midi_note = frame.midi_note;
        }

        // --- Reference lookup (returns owned clone to avoid borrow conflict) ---
        let (ref_idx, reference_note) = self.find_active_note_cloned(current_time);

        let (cents_deviation, note_correct, accuracy) = match &reference_note {
            Some(ref_note) => {
                let expected_freq = midi_to_freq(ref_note.midi_note);
                let cents = if frame.frequency_hz > 0.0 {
                    1200.0 * (frame.frequency_hz / expected_freq).log2()
                } else {
                    // Silence during a reference note: treat as maximum deviation.
                    // Use 100 cents (one semitone) as a bounded penalty rather
                    // than f32::MAX which would destroy score averages.
                    100.0
                };
                let correct = cents.abs() < 50.0;
                let bucket = AccuracyBucket::from_cents(cents);
                (cents, correct, bucket)
            }
            None => (0.0, false, AccuracyBucket::Off),
        };

        // --- Timing offset ---
        let timing_offset_ms = self.compute_timing_offset(ref_idx, &reference_note);
        let timing_bucket = TimingBucket::from_ms(timing_offset_ms);

        ComparisonFrame {
            reference_note,
            played_pitch: frame.clone(),
            cents_deviation,
            timing_offset_ms,
            note_correct,
            accuracy_bucket: accuracy,
            timing_bucket,
        }
    }

    /// Compute the timing offset in milliseconds for the current frame.
    ///
    /// When we detect a new played note onset that matches a reference note,
    /// we record the offset. For subsequent frames within the same reference
    /// note, we replay the recorded offset.
    fn compute_timing_offset(
        &mut self,
        ref_idx: Option<usize>,
        reference_note: &Option<NoteEvent>,
    ) -> f32 {
        let (Some(idx), Some(ref_note)) = (ref_idx, reference_note) else {
            return 0.0;
        };

        // If we already recorded an onset for this reference note, reuse it
        if let Some(&played_onset) = self.onset_recorded.get(&idx) {
            return ((played_onset - ref_note.onset_seconds) * 1000.0) as f32;
        }

        // If we have a current played onset, record it for this reference note
        if let Some(played_onset) = self.current_onset_seconds {
            self.onset_recorded.insert(idx, played_onset);
            return ((played_onset - ref_note.onset_seconds) * 1000.0) as f32;
        }

        0.0
    }

    /// Find the active reference note at the given time, returning a clone
    /// so we don't hold a borrow on `self.reference`.
    fn find_active_note_cloned(&self, time_seconds: f64) -> (Option<usize>, Option<NoteEvent>) {
        for (i, n) in self.reference.iter().enumerate() {
            if time_seconds >= n.onset_seconds
                && time_seconds < n.onset_seconds + n.duration_seconds
            {
                return (Some(i), Some(n.clone()));
            }
        }
        (None, None)
    }
}

fn midi_to_freq(midi_note: u8) -> f32 {
    440.0 * 2.0f32.powf((midi_note as f32 - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifflab_core::practice::TimingBucket;

    /// Helper: create a NoteEvent at the given onset with a given MIDI note.
    fn note(midi: u8, onset: f64, duration: f64) -> NoteEvent {
        NoteEvent {
            midi_note: midi,
            onset_seconds: onset,
            duration_seconds: duration,
            average_cents: 0.0,
            confidence: 1.0,
        }
    }

    /// Helper: create a PitchFrame for a given MIDI note (perfect pitch).
    fn pitch(midi: u8) -> PitchFrame {
        let freq = midi_to_freq(midi);
        PitchFrame {
            frequency_hz: freq,
            confidence: 0.95,
            midi_note: midi,
            cents_deviation: 0.0,
        }
    }

    /// Helper: create a silent PitchFrame.
    fn silence() -> PitchFrame {
        PitchFrame {
            frequency_hz: 0.0,
            confidence: 0.0,
            midi_note: 0,
            cents_deviation: 0.0,
        }
    }

    /// Helper: create a SongPosition at the given time in seconds (48kHz).
    fn pos(seconds: f64) -> SongPosition {
        let frame = (seconds * 48000.0) as u64;
        SongPosition::new(frame, 48000)
    }

    #[test]
    fn test_timing_offset_exact() {
        // Reference note A4 at 1.0s, duration 0.5s
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Silence before the note
        let cf = comp.compare(&silence(), &pos(0.5));
        assert!(cf.reference_note.is_none());

        // Player hits A4 exactly at 1.0s
        let cf = comp.compare(&pitch(69), &pos(1.0));
        assert!(cf.reference_note.is_some());
        assert!(cf.note_correct);
        // Timing offset should be ~0ms (played at 1.0s, reference at 1.0s)
        assert!(
            cf.timing_offset_ms.abs() < 1.0,
            "Expected ~0ms offset, got {}ms",
            cf.timing_offset_ms
        );
        assert_eq!(cf.timing_bucket, TimingBucket::Tight);
    }

    #[test]
    fn test_timing_offset_early() {
        // Reference note at 1.0s
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Player hits A4 at 0.97s — 30ms early, but reference hasn't started yet
        // so no match at this point
        let cf = comp.compare(&pitch(69), &pos(0.97));
        assert!(cf.reference_note.is_none());

        // At 1.0s the note is now in range, but the onset was already detected at 0.97s
        // (the note is still the same MIDI note, so no new onset is detected)
        let cf = comp.compare(&pitch(69), &pos(1.0));
        assert!(cf.reference_note.is_some());
        // Onset was at 0.97s, reference at 1.0s → -30ms
        assert!(
            (cf.timing_offset_ms - (-30.0)).abs() < 2.0,
            "Expected ~-30ms, got {}ms",
            cf.timing_offset_ms
        );
        assert_eq!(cf.timing_bucket, TimingBucket::Good);
    }

    #[test]
    fn test_timing_offset_late() {
        // Reference note at 1.0s
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Silence until 1.05s
        let _ = comp.compare(&silence(), &pos(1.0));
        let _ = comp.compare(&silence(), &pos(1.02));

        // Player hits A4 at 1.05s — 50ms late
        let cf = comp.compare(&pitch(69), &pos(1.05));
        assert!(cf.reference_note.is_some());
        assert!(cf.note_correct);
        assert!(
            (cf.timing_offset_ms - 50.0).abs() < 2.0,
            "Expected ~50ms, got {}ms",
            cf.timing_offset_ms
        );
        assert_eq!(cf.timing_bucket, TimingBucket::Good);
    }

    #[test]
    fn test_timing_offset_persists_across_frames() {
        // Reference note at 1.0s, duration 0.5s
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Player hits at 1.02s (20ms late)
        let _ = comp.compare(&silence(), &pos(1.0));
        let cf1 = comp.compare(&pitch(69), &pos(1.02));
        let offset1 = cf1.timing_offset_ms;

        // Subsequent frames should report the same timing offset
        let cf2 = comp.compare(&pitch(69), &pos(1.1));
        let cf3 = comp.compare(&pitch(69), &pos(1.2));

        assert!(
            (cf2.timing_offset_ms - offset1).abs() < 0.01,
            "Expected same offset, got {} vs {}",
            cf2.timing_offset_ms,
            offset1
        );
        assert!(
            (cf3.timing_offset_ms - offset1).abs() < 0.01,
            "Expected same offset, got {} vs {}",
            cf3.timing_offset_ms,
            offset1
        );
    }

    #[test]
    fn test_correct_note_detection() {
        // Reference: A4 (MIDI 69) at 1.0s
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Player plays A4 — correct
        let cf = comp.compare(&pitch(69), &pos(1.0));
        assert!(cf.note_correct);

        // Player plays C5 (MIDI 72) — wrong note
        let mut comp2 = Comparator::new(vec![note(69, 1.0, 0.5)]);
        let cf = comp2.compare(&pitch(72), &pos(1.0));
        // 3 semitones off = ~300 cents, way more than 50
        assert!(!cf.note_correct);
    }

    #[test]
    fn test_full_pipeline_two_notes() {
        use crate::scoring::SessionScorer;

        // Reference: A4 at 1.0s (0.5s), then C5 at 1.5s (0.5s)
        // Start reference late enough that silence frames before it don't count.
        let reference = vec![
            note(69, 1.0, 0.5), // A4
            note(72, 1.5, 0.5), // C5
        ];
        let mut comp = Comparator::new(reference);
        let mut scorer = SessionScorer::new();

        // Silence before the first note — these frames have no reference, so
        // they do not affect scoring.
        for t in (0..1000).step_by(10) {
            let s = t as f64 / 1000.0;
            scorer.feed(comp.compare(&silence(), &pos(s)));
        }

        // Player plays A4 starting at 1.01s (10ms late)
        let cf = comp.compare(&pitch(69), &pos(1.01));
        assert!(cf.note_correct);
        assert!(
            (cf.timing_offset_ms - 10.0).abs() < 2.0,
            "Expected ~10ms offset, got {}ms",
            cf.timing_offset_ms
        );
        scorer.feed(cf);

        // Continue A4 through the rest of the reference window
        for t in (1020..1500).step_by(10) {
            let s = t as f64 / 1000.0;
            scorer.feed(comp.compare(&pitch(69), &pos(s)));
        }

        // Brief silence at note boundary
        scorer.feed(comp.compare(&silence(), &pos(1.50)));

        // Player plays C5 starting at 1.52s (20ms late)
        let cf = comp.compare(&pitch(72), &pos(1.52));
        assert!(cf.note_correct);
        assert!(
            (cf.timing_offset_ms - 20.0).abs() < 2.0,
            "Expected ~20ms offset, got {}ms",
            cf.timing_offset_ms
        );
        scorer.feed(cf);

        // Continue C5
        for t in (1530..2000).step_by(10) {
            let s = t as f64 / 1000.0;
            scorer.feed(comp.compare(&pitch(72), &pos(s)));
        }

        // Most frames during reference windows had correct notes with perfect pitch.
        // The only "bad" frames are the silence at 1.50s (during C5 region)
        // and the first silence at 1.00s (before player starts note 1).
        // The score should be high.
        assert!(
            scorer.score() > 80.0,
            "Expected high score, got {} (correct={}, total={})",
            scorer.score(),
            scorer.notes_correct(),
            scorer.notes_total(),
        );
        assert!(scorer.notes_correct() > 0);
        assert!(scorer.notes_total() > 0);
    }

    #[test]
    fn test_reset_clears_state() {
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Play a note
        comp.compare(&silence(), &pos(0.9));
        comp.compare(&pitch(69), &pos(1.0));

        // Reset
        comp.reset();

        // After reset, onset tracking should be cleared
        assert!(comp.prev_was_silent);
        assert_eq!(comp.prev_midi_note, 0);
        assert!(comp.current_onset_seconds.is_none());
        assert!(comp.onset_recorded.is_empty());
    }

    #[test]
    fn test_silence_produces_no_timing() {
        let reference = vec![note(69, 1.0, 0.5)];
        let mut comp = Comparator::new(reference);

        // Silence during reference note — no played onset
        let cf = comp.compare(&silence(), &pos(1.0));
        assert_eq!(cf.timing_offset_ms, 0.0);
        assert!(!cf.note_correct);
    }
}

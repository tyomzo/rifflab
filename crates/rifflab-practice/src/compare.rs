use rifflab_core::analysis::{NoteEvent, PitchFrame};
use rifflab_core::practice::{AccuracyBucket, ComparisonFrame, TimingBucket};
use rifflab_core::transport::SongPosition;

/// Real-time comparison engine.
/// Compares live PitchFrame input against a reference note sequence.
pub struct Comparator {
    reference: Vec<NoteEvent>,
}

impl Comparator {
    pub fn new(reference: Vec<NoteEvent>) -> Self {
        Self { reference }
    }

    /// Compare the current pitch frame against the reference at the given song position.
    pub fn compare(&self, frame: &PitchFrame, position: &SongPosition) -> ComparisonFrame {
        let current_time = position.seconds();
        let reference_note = self.find_active_note(current_time);

        let (cents_deviation, note_correct, accuracy) = match &reference_note {
            Some(ref_note) => {
                let expected_freq = midi_to_freq(ref_note.midi_note);
                let cents = if frame.frequency_hz > 0.0 {
                    1200.0 * (frame.frequency_hz / expected_freq).log2()
                } else {
                    f32::MAX
                };
                let correct = cents.abs() < 50.0;
                let bucket = AccuracyBucket::from_cents(cents);
                (cents, correct, bucket)
            }
            None => (0.0, false, AccuracyBucket::Off),
        };

        ComparisonFrame {
            reference_note: reference_note.cloned(),
            played_pitch: frame.clone(),
            cents_deviation,
            timing_offset_ms: 0.0, // TODO: compute from onset alignment
            note_correct,
            accuracy_bucket: accuracy,
            timing_bucket: TimingBucket::Missed,
        }
    }

    fn find_active_note(&self, time_seconds: f64) -> Option<&NoteEvent> {
        self.reference.iter().find(|n| {
            time_seconds >= n.onset_seconds
                && time_seconds < n.onset_seconds + n.duration_seconds
        })
    }
}

fn midi_to_freq(midi_note: u8) -> f32 {
    440.0 * 2.0f32.powf((midi_note as f32 - 69.0) / 12.0)
}

use serde::{Deserialize, Serialize};

/// Output of real-time pitch detection (one per analysis frame).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PitchFrame {
    /// Detected fundamental frequency in Hz. 0.0 if no pitch detected.
    pub frequency_hz: f32,
    /// Detection confidence (0.0–1.0).
    pub confidence: f32,
    /// Nearest MIDI note number.
    pub midi_note: u8,
    /// Cents deviation from the nearest MIDI note (-50.0 to +50.0).
    pub cents_deviation: f32,
}

impl PitchFrame {
    /// Returns the note name (e.g., "A4", "C#3").
    pub fn note_name(&self) -> String {
        if self.frequency_hz <= 0.0 {
            return "--".to_string();
        }
        let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let octave = (self.midi_note as i32 / 12) - 1;
        let note_idx = (self.midi_note % 12) as usize;
        format!("{}{}", names[note_idx], octave)
    }
}

/// A discrete note event from transcription (offline or real-time segmented).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEvent {
    /// MIDI note number (0–127).
    pub midi_note: u8,
    /// Note onset time in seconds from song start.
    pub onset_seconds: f64,
    /// Note duration in seconds.
    pub duration_seconds: f64,
    /// Average cents deviation from perfect pitch during this note.
    pub average_cents: f32,
    /// Detection confidence (0.0–1.0).
    pub confidence: f32,
}

impl NoteEvent {
    /// End time in seconds.
    pub fn end_seconds(&self) -> f64 {
        self.onset_seconds + self.duration_seconds
    }

    /// Frequency in Hz.
    pub fn frequency_hz(&self) -> f32 {
        440.0 * 2.0f32.powf((self.midi_note as f32 - 69.0) / 12.0)
    }
}

/// Trait for pitch detection algorithms, allowing the audio engine to be
/// independent of any specific detector implementation.
pub trait PitchDetector: Send {
    /// Detect pitch from a buffer of mono audio samples.
    fn detect(&mut self, samples: &[f32]) -> PitchFrame;
}

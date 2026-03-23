use serde::{Deserialize, Serialize};
use crate::analysis::{NoteEvent, PitchFrame};

/// Pitch accuracy bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccuracyBucket {
    /// Within ±5 cents.
    Perfect,
    /// Within ±15 cents.
    Good,
    /// Within ±25 cents.
    Acceptable,
    /// More than 25 cents off, or wrong note.
    Off,
}

impl AccuracyBucket {
    pub fn from_cents(cents: f32) -> Self {
        let abs = cents.abs();
        if abs <= 5.0 {
            Self::Perfect
        } else if abs <= 15.0 {
            Self::Good
        } else if abs <= 25.0 {
            Self::Acceptable
        } else {
            Self::Off
        }
    }
}

/// Timing accuracy bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimingBucket {
    /// Within ±20ms.
    Tight,
    /// Within ±50ms.
    Good,
    /// Within ±100ms.
    Loose,
    /// More than 100ms off, or no note played.
    Missed,
}

impl TimingBucket {
    pub fn from_ms(offset_ms: f32) -> Self {
        let abs = offset_ms.abs();
        if abs <= 20.0 {
            Self::Tight
        } else if abs <= 50.0 {
            Self::Good
        } else if abs <= 100.0 {
            Self::Loose
        } else {
            Self::Missed
        }
    }
}

/// Output of the real-time comparison engine (one per analysis frame).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonFrame {
    /// The reference note active at this position (if any).
    pub reference_note: Option<NoteEvent>,
    /// The player's detected pitch.
    pub played_pitch: PitchFrame,
    /// Cents deviation from the reference note.
    pub cents_deviation: f32,
    /// Timing offset in milliseconds from the reference onset.
    pub timing_offset_ms: f32,
    /// Whether the player hit the correct note (within ±50 cents).
    pub note_correct: bool,
    /// Pitch accuracy bucket.
    pub accuracy_bucket: AccuracyBucket,
    /// Timing accuracy bucket.
    pub timing_bucket: TimingBucket,
}

/// Aggregate result of a practice session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResult {
    pub session_id: uuid::Uuid,
    pub song_id: uuid::Uuid,
    pub timestamp: String,
    pub duration_seconds: f64,
    pub notes_total: u32,
    pub notes_correct: u32,
    pub notes_missed: u32,
    pub notes_extra: u32,
    pub avg_cents_deviation: f32,
    pub avg_timing_offset_ms: f32,
    pub score: f32,
}

/// Maximum fret number for bass guitar.
pub const MAX_FRET: u8 = 24;

/// Default tempo when none is specified.
pub const DEFAULT_BPM: f64 = 120.0;

/// Minimum confidence threshold below which notes are discarded.
pub const MIN_CONFIDENCE: f32 = 0.3;

/// Default confidence for new notes.
pub const DEFAULT_CONFIDENCE: f32 = 0.8;

/// Confidence multiplier applied to unmatched notes during fusion.
pub const CONFIDENCE_DAMPING: f32 = 0.7;

/// Maximum input size (bytes) sent to the LLM API.
pub const LLM_INPUT_TRUNCATE: usize = 51200;

/// Minimum DTW alignment window size.
pub const DTW_MIN_WINDOW: usize = 5;

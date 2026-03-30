use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single note event in a bass tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabNote {
    pub id: Uuid,
    /// String index: 0=E (lowest), 1=A, 2=D, 3=G.
    pub string: u8,
    /// Fret number (0 = open string, max 24).
    pub fret: u8,
    /// Onset time in seconds from track start.
    pub time_secs: f64,
    /// Duration in seconds.
    pub duration_secs: f64,
    /// Beat position (1-indexed beat within the measure).
    pub beat: Option<f64>,
    /// Measure number (1-indexed).
    pub measure: Option<u32>,
    /// Playing technique, if detected.
    pub technique: Option<Technique>,
    /// Confidence score from transcription/parsing (0.0–1.0).
    pub confidence: f32,
    /// Source that produced this note event.
    pub source: NoteSource,
    /// Section label (e.g., "Intro", "Verse") — set on the first note of a section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

impl TabNote {
    pub fn new(string: u8, fret: u8, source: NoteSource) -> Self {
        Self {
            id: Uuid::new_v4(),
            string,
            fret,
            time_secs: 0.0,
            duration_secs: 0.0,
            beat: None,
            measure: None,
            technique: None,
            confidence: crate::constants::DEFAULT_CONFIDENCE,
            source,
            section: None,
        }
    }

    /// MIDI note number for this note given a tuning.
    pub fn midi_note(&self, tuning: &[u8; 4]) -> u8 {
        tuning[self.string as usize] + self.fret
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Technique {
    Normal,
    HammerOn,
    PullOff,
    SlideUp,
    SlideDown,
    Slap,
    Pop,
    Mute,
    Bend,
    Vibrato,
    Ghost,
    Harmonic,
    TapOn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteSource {
    AsciiParse,
    AudioTranscription,
    Fused,
    UserEdit,
}

/// A complete tab for a single track/song.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabDocument {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    /// Tuning as MIDI note numbers for each string, low to high.
    /// Standard 4-string bass: [28, 33, 38, 43] (E1, A1, D2, G2).
    pub tuning: [u8; 4],
    pub tempo: TempoMap,
    pub time_signature: TimeSignature,
    /// All note events, ordered by time_secs.
    pub notes: Vec<TabNote>,
    /// Measure markers (start time of each measure).
    #[serde(default)]
    pub measures: Vec<MeasureMarker>,
    pub provenance: TabProvenance,
}

impl TabDocument {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            artist: None,
            tuning: STANDARD_TUNING,
            tempo: TempoMap::constant(crate::constants::DEFAULT_BPM),
            time_signature: TimeSignature { beats_per_measure: 4, beat_unit: 4 },
            notes: Vec::new(),
            measures: Vec::new(),
            provenance: TabProvenance {
                ascii_source: None,
                audio_source: None,
                mode: PipelineMode::AsciiOnly,
                created_at: String::new(),
            },
        }
    }

    /// Total duration based on the last note's end time.
    pub fn duration_secs(&self) -> f64 {
        self.notes.iter()
            .map(|n| n.time_secs + n.duration_secs)
            .fold(0.0, f64::max)
    }

    /// Assign synthetic timing to notes that have no timing (from ASCII import).
    /// Distributes notes evenly at the given tempo, one note per eighth note.
    pub fn assign_timing(&mut self) {
        let beat_duration = 60.0 / self.tempo.initial_bpm;
        let note_spacing = beat_duration / 2.0; // eighth notes
        for (i, note) in self.notes.iter_mut().enumerate() {
            if note.time_secs <= 0.0 || note.duration_secs <= 0.0 {
                note.time_secs = i as f64 * note_spacing;
                note.duration_secs = note_spacing * 0.9;
            }
        }
    }

    /// Generate measure markers from tempo map and time signature.
    pub fn generate_measures(&mut self) {
        self.measures.clear();
        let beat_duration = 60.0 / self.tempo.initial_bpm;
        let measure_duration = beat_duration * self.time_signature.beats_per_measure as f64;
        let total = self.duration_secs();
        let mut time = 0.0;
        let mut num = 1u32;
        while time <= total + measure_duration {
            self.measures.push(MeasureMarker { measure_number: num, time_secs: time });
            time += measure_duration;
            num += 1;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoMap {
    pub initial_bpm: f64,
    /// Tempo changes: (time_secs, new_bpm). Empty if constant tempo.
    #[serde(default)]
    pub changes: Vec<(f64, f64)>,
}

impl TempoMap {
    pub fn constant(bpm: f64) -> Self {
        Self { initial_bpm: bpm, changes: Vec::new() }
    }

    pub fn bpm_at(&self, time_secs: f64) -> f64 {
        let mut bpm = self.initial_bpm;
        for &(t, b) in &self.changes {
            if t <= time_secs { bpm = b; } else { break; }
        }
        bpm
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimeSignature {
    pub beats_per_measure: u8,
    /// 4 = quarter note, 8 = eighth note.
    pub beat_unit: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasureMarker {
    pub measure_number: u32,
    pub time_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabProvenance {
    pub ascii_source: Option<String>,
    pub audio_source: Option<String>,
    pub mode: PipelineMode,
    /// ISO 8601 timestamp string.
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineMode {
    AudioOnly,
    AsciiOnly,
    Fused,
}

// ─── Standard tunings ────────────────────────────────────────────────────────

/// Standard 4-string bass tuning: E1, A1, D2, G2.
pub const STANDARD_TUNING: [u8; 4] = [28, 33, 38, 43];
/// Drop D 4-string bass tuning: D1, A1, D2, G2.
pub const DROP_D_TUNING: [u8; 4] = [26, 33, 38, 43];

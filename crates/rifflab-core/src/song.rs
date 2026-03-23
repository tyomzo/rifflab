use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unique song identifier.
pub type SongId = uuid::Uuid;

/// Stem type from source separation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StemType {
    Vocals,
    Drums,
    Bass,
    Guitar,
    Piano,
    Other,
}

impl StemType {
    pub fn filename(&self) -> &str {
        match self {
            Self::Vocals => "vocals.wav",
            Self::Drums => "drums.wav",
            Self::Bass => "bass.wav",
            Self::Guitar => "guitar.wav",
            Self::Piano => "piano.wav",
            Self::Other => "other.wav",
        }
    }

    /// Standard 4-stem types.
    pub fn four_stems() -> &'static [StemType] {
        &[Self::Vocals, Self::Drums, Self::Bass, Self::Other]
    }

    /// Standard 6-stem types.
    pub fn six_stems() -> &'static [StemType] {
        &[Self::Vocals, Self::Drums, Self::Bass, Self::Guitar, Self::Piano, Self::Other]
    }
}

/// Information about a single stem file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StemInfo {
    pub stem_type: StemType,
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f64,
    pub num_frames: u64,
}

/// Musical key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicalKey {
    pub root: String,
    pub quality: KeyQuality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyQuality {
    Major,
    Minor,
}

/// Beat grid: a sequence of beat timestamps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeatGrid {
    pub beats: Vec<BeatMarker>,
}

impl BeatGrid {
    /// Get the BPM at a given time (supports variable tempo).
    pub fn bpm_at(&self, time_seconds: f64) -> Option<f64> {
        self.beats
            .iter()
            .rev()
            .find(|b| b.time_seconds <= time_seconds)
            .map(|b| b.bpm)
    }

    /// Find the (bar, beat) at a given time.
    pub fn bar_beat_at(&self, time_seconds: f64) -> Option<(u32, u32)> {
        self.beats
            .iter()
            .rev()
            .find(|b| b.time_seconds <= time_seconds)
            .map(|b| (b.bar, b.beat))
    }
}

/// A single beat marker in the beat grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatMarker {
    pub time_seconds: f64,
    pub bar: u32,
    pub beat: u32,
    /// Local BPM (supports tempo changes).
    pub bpm: f64,
}

/// Full song metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub id: SongId,
    pub title: String,
    pub artist: Option<String>,
    pub bpm: Option<f64>,
    pub key: Option<MusicalKey>,
    pub time_signature: (u8, u8),
    pub stems: Vec<StemInfo>,
    pub beat_grid: Option<BeatGrid>,
}

impl Song {
    pub fn new(title: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            title,
            artist: None,
            bpm: None,
            key: None,
            time_signature: (4, 4),
            stems: Vec::new(),
            beat_grid: None,
        }
    }
}

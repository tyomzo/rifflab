use serde::{Deserialize, Serialize};

/// Transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportState {
    Stopped,
    Playing,
    Paused,
}

impl Default for TransportState {
    fn default() -> Self {
        Self::Stopped
    }
}

/// Commands sent from the UI thread to the audio thread.
#[derive(Debug, Clone)]
pub enum TransportCommand {
    Play,
    Pause,
    Stop,
    Seek(u64),
    SetLoop(Option<LoopRegion>),
}

/// A loop region defined by sample positions.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct LoopRegion {
    pub start_frame: u64,
    pub end_frame: u64,
}

/// Current song position, updated by the audio thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct SongPosition {
    /// Current position in samples from the start.
    pub frame: u64,
    /// Sample rate used for time conversion.
    pub sample_rate: u32,
}

impl SongPosition {
    pub fn new(frame: u64, sample_rate: u32) -> Self {
        Self { frame, sample_rate }
    }

    /// Position in seconds.
    pub fn seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frame as f64 / self.sample_rate as f64
    }

    /// Convert to (bar, beat, tick) given a BPM and time signature.
    pub fn bar_beat_tick(&self, bpm: f64, beats_per_bar: u8) -> (u32, u32, u32) {
        if bpm <= 0.0 || beats_per_bar == 0 {
            return (0, 0, 0);
        }
        let seconds = self.seconds();
        let total_beats = seconds * bpm / 60.0;
        let bar = (total_beats / beats_per_bar as f64).floor() as u32;
        let beat = (total_beats % beats_per_bar as f64).floor() as u32;
        let tick = ((total_beats.fract()) * 960.0) as u32; // 960 ticks per beat
        (bar + 1, beat + 1, tick)
    }
}

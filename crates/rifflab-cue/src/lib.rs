use rifflab_core::transport::LoopRegion;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Position of a cue on the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CuePosition {
    /// Absolute position in audio frames.
    Frame(u64),
    /// Musical position (requires a beat grid).
    BarBeat { bar: u32, beat: f32 },
}

impl CuePosition {
    /// Resolve to an absolute frame position.
    /// For BarBeat, requires bpm and sample_rate. Returns None if beat grid is needed but unavailable.
    pub fn to_frame(&self, bpm: Option<f64>, sample_rate: u32) -> Option<u64> {
        match self {
            CuePosition::Frame(f) => Some(*f),
            CuePosition::BarBeat { bar, beat } => {
                let bpm = bpm?;
                let beats_per_bar = 4.0; // assume 4/4 for now
                let total_beats = (*bar as f64 - 1.0) * beats_per_bar + (*beat as f64 - 1.0);
                let seconds = total_beats * 60.0 / bpm;
                Some((seconds * sample_rate as f64) as u64)
            }
        }
    }
}

/// Action to perform when a cue is triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CueAction {
    /// Set a loop region.
    SetLoop(LoopRegion),
    /// Clear the active loop.
    ClearLoop,
    /// Switch to a named effect preset.
    SwitchPreset(String),
    /// Section marker (visual only, no action).
    Marker,
}

/// A single cue point on the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cue {
    /// Position on the timeline.
    pub position: CuePosition,
    /// Action to perform when triggered.
    pub action: CueAction,
    /// Display label.
    pub label: String,
    /// Color as [r, g, b].
    pub color: [u8; 3],
}

/// Ordered list of cues for a song.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CueList {
    pub cues: Vec<Cue>,
}

impl CueList {
    pub fn new() -> Self {
        Self { cues: Vec::new() }
    }

    /// Add a cue and keep the list sorted by position (frame-based cues first).
    pub fn add(&mut self, cue: Cue) {
        self.cues.push(cue);
        // Sort by resolved frame position (BarBeat without BPM sorts last)
        self.cues.sort_by_key(|c| match &c.position {
            CuePosition::Frame(f) => *f,
            CuePosition::BarBeat { bar, beat } => {
                // Approximate sort order: bar * 10000 + beat * 100
                (*bar as u64) * 10000 + (*beat * 100.0) as u64
            }
        });
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.cues.len() {
            self.cues.remove(index);
        }
    }

    pub fn len(&self) -> usize {
        self.cues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }
}

/// The cue engine: tracks playback position and fires cue actions.
pub struct CueEngine {
    cue_list: CueList,
    /// Last processed transport position (to detect crossing).
    last_frame: u64,
    /// Sample rate for BarBeat resolution.
    sample_rate: u32,
    /// BPM for BarBeat resolution (None if no beat grid).
    bpm: Option<f64>,
}

/// Action dispatched by the cue engine.
#[derive(Debug, Clone)]
pub enum DispatchedAction {
    SetLoop(LoopRegion),
    ClearLoop,
    SwitchPreset(String),
}

impl CueEngine {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            cue_list: CueList::new(),
            last_frame: 0,
            sample_rate,
            bpm: None,
        }
    }

    pub fn set_cue_list(&mut self, list: CueList) {
        self.cue_list = list;
    }

    pub fn cue_list(&self) -> &CueList {
        &self.cue_list
    }

    pub fn cue_list_mut(&mut self) -> &mut CueList {
        &mut self.cue_list
    }

    pub fn set_bpm(&mut self, bpm: Option<f64>) {
        self.bpm = bpm;
    }

    /// Tick the cue engine. Call this each audio buffer with the current transport position.
    /// Returns any actions that should be dispatched.
    pub fn tick(&mut self, current_frame: u64) -> Vec<DispatchedAction> {
        let mut actions = Vec::new();

        // Detect cues between last_frame (exclusive) and current_frame (inclusive)
        let prev = self.last_frame;
        self.last_frame = current_frame;

        // Don't fire on seek backwards or first frame
        if current_frame <= prev {
            return actions;
        }

        for cue in &self.cue_list.cues {
            if let Some(cue_frame) = cue.position.to_frame(self.bpm, self.sample_rate) {
                if cue_frame > prev && cue_frame <= current_frame {
                    match &cue.action {
                        CueAction::SetLoop(region) => {
                            actions.push(DispatchedAction::SetLoop(region.clone()));
                        }
                        CueAction::ClearLoop => {
                            actions.push(DispatchedAction::ClearLoop);
                        }
                        CueAction::SwitchPreset(name) => {
                            actions.push(DispatchedAction::SwitchPreset(name.clone()));
                        }
                        CueAction::Marker => {} // visual only
                    }
                }
            }
        }

        actions
    }

    /// Reset the engine (e.g., on transport stop/seek).
    pub fn reset(&mut self, frame: u64) {
        self.last_frame = frame;
    }
}

// ─── Persistence ─────────────────────────────────────────────────────────────

/// Save cue list to a JSON file.
pub fn save_cues(path: &Path, cues: &CueList) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(cues)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)?;
    Ok(())
}

/// Load cue list from a JSON file.
pub fn load_cues(path: &Path) -> Result<CueList, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(path)?;
    let cues: CueList = serde_json::from_str(&json)?;
    Ok(cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cue_position_frame() {
        let pos = CuePosition::Frame(48000);
        assert_eq!(pos.to_frame(None, 48000), Some(48000));
    }

    #[test]
    fn test_cue_position_bar_beat() {
        let pos = CuePosition::BarBeat { bar: 2, beat: 1.0 };
        // Bar 2, beat 1 at 120 BPM, 48kHz = 4 beats in, 2 seconds
        let frame = pos.to_frame(Some(120.0), 48000).unwrap();
        assert_eq!(frame, 96000); // 2.0s * 48000
    }

    #[test]
    fn test_cue_engine_tick() {
        let mut engine = CueEngine::new(48000);
        let mut list = CueList::new();
        list.add(Cue {
            position: CuePosition::Frame(1000),
            action: CueAction::Marker,
            label: "Intro".into(),
            color: [255, 200, 50],
        });
        list.add(Cue {
            position: CuePosition::Frame(5000),
            action: CueAction::SetLoop(LoopRegion { start_frame: 5000, end_frame: 10000 }),
            label: "Loop A".into(),
            color: [50, 200, 255],
        });
        engine.set_cue_list(list);

        // Advance past first cue
        let actions = engine.tick(2000);
        assert!(actions.is_empty() || matches!(actions[0], DispatchedAction::SetLoop(_)) == false);
        // The marker fires but produces no DispatchedAction

        // Advance past second cue
        let actions = engine.tick(6000);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], DispatchedAction::SetLoop(r) if r.start_frame == 5000));
    }

    #[test]
    fn test_cue_persistence_roundtrip() {
        let mut list = CueList::new();
        list.add(Cue {
            position: CuePosition::Frame(1000),
            action: CueAction::SwitchPreset("Clean".into()),
            label: "Clean Section".into(),
            color: [80, 200, 120],
        });
        list.add(Cue {
            position: CuePosition::Frame(50000),
            action: CueAction::ClearLoop,
            label: "End Loop".into(),
            color: [200, 80, 80],
        });

        let tmp = std::env::temp_dir().join(format!("rifflab_cue_test_{}.json", std::process::id()));
        save_cues(&tmp, &list).unwrap();

        let loaded = load_cues(&tmp).unwrap();
        assert_eq!(loaded.cues.len(), 2);
        assert_eq!(loaded.cues[0].label, "Clean Section");
        assert!(matches!(&loaded.cues[1].action, CueAction::ClearLoop));

        let _ = std::fs::remove_file(&tmp);
    }
}

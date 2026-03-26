//! Preset bank: multiple graph presets loaded in memory, one active at a time.
//! Footswitch presses (MIDI CC) instantly switch the active preset.

use crate::node_editor::{self, FxGraph};
use std::path::PathBuf;

/// A single preset slot in the bank.
#[derive(Clone)]
pub struct PresetSlot {
    pub name: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub graph: FxGraph,
    /// MIDI CC number that activates this preset (default: 80, 81, 82...).
    pub midi_cc: u8,
}

/// The preset bank holds multiple presets, one is active.
pub struct PresetBank {
    pub presets: Vec<PresetSlot>,
    pub active_index: Option<usize>,
}

impl PresetBank {
    pub fn new() -> Self {
        Self {
            presets: Vec::new(),
            active_index: None,
        }
    }

    /// Add a preset from a graph JSON file. Returns the slot index.
    pub fn add_from_file(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let graph = node_editor::load_graph(path)
            .map_err(|e| format!("Load failed: {e}"))?;

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        // Default CC: 80 + slot index
        let cc = 80 + self.presets.len() as u8;

        let idx = self.presets.len();
        self.presets.push(PresetSlot {
            name,
            path: path.to_path_buf(),
            graph,
            midi_cc: cc,
        });

        log::info!("Preset bank: added '{}' at slot {} (CC#{})", self.presets[idx].name, idx, cc);
        Ok(idx)
    }

    /// Remove a preset by index.
    pub fn remove(&mut self, index: usize) {
        if index < self.presets.len() {
            self.presets.remove(index);
            // Fix active index
            match self.active_index {
                Some(i) if i == index => self.active_index = None,
                Some(i) if i > index => self.active_index = Some(i - 1),
                _ => {}
            }
        }
    }

    /// Find a preset by MIDI CC number.
    pub fn find_by_cc(&self, cc: u8) -> Option<usize> {
        self.presets.iter().position(|p| p.midi_cc == cc)
    }

    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }

        #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.presets.len()
    }
}

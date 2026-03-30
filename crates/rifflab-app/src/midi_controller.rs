//! MIDI controller subsystem: connection, event polling, learn modes, and mapping persistence.

use crate::midi_input;
use crate::preset_graph;
use std::collections::HashMap;

/// Arturia MiniLab 3 default knob CC numbers (Arturia preset).
const MINILAB3_KNOB_CCS: [u8; 8] = [74, 71, 76, 77, 93, 18, 19, 16];
/// Arturia MiniLab 3 default fader CC numbers.
const MINILAB3_FADER_CCS: [u8; 4] = [82, 83, 85, 17];

/// Saved MIDI controller mapping (knob + fader CCs).
#[derive(serde::Serialize, serde::Deserialize)]
struct MidiMapping {
    knob_ccs: Vec<u8>,
    fader_ccs: Vec<u8>,
}

impl MidiMapping {
    fn mappings_dir() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "rifflab")
            .map(|dirs| dirs.config_dir().join("midi_mappings"))
    }

    fn device_filename(port_name: &str) -> String {
        let name = port_name.split(':').next().unwrap_or(port_name);
        name.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    fn save(port_name: &str, knobs: &[u8], faders: &[u8]) -> Result<(), String> {
        let dir = Self::mappings_dir().ok_or("No config dir")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{e}"))?;
        let path = dir.join(format!("{}.json", Self::device_filename(port_name)));
        let mapping = MidiMapping {
            knob_ccs: knobs.to_vec(),
            fader_ccs: faders.to_vec(),
        };
        let json = serde_json::to_string_pretty(&mapping).map_err(|e| format!("{e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("{e}"))?;
        Ok(())
    }

    fn load(port_name: &str) -> Option<MidiMapping> {
        let dir = Self::mappings_dir()?;
        let path = dir.join(format!("{}.json", Self::device_filename(port_name)));
        let json = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&json).ok()
    }
}

/// Manages MIDI input connection, event polling, learn modes, and controller mapping.
pub struct MidiController {
    pub connection: Option<midi_input::MidiConnection>,
    pub rx: Option<std::sync::mpsc::Receiver<midi_input::MidiEvent>>,
    pub port_names: Vec<String>,
    pub selected_port: usize,
    /// MIDI CC numbers for physical knobs.
    pub knob_ccs: Vec<u8>,
    /// When true, next incoming CCs teach knob slots sequentially.
    pub knob_learning: bool,
    /// Last raw CC value per knob CC (for endless encoder delta computation).
    pub knob_last: HashMap<u8, u8>,
    /// MIDI CC numbers for physical faders.
    pub fader_ccs: Vec<u8>,
    /// When true, next incoming CCs teach fader slots sequentially.
    pub fader_learning: bool,
    /// Effect node MIDI learn: waiting for a MIDI event to bind to this node.
    pub learn_node: Option<u64>,
    /// MIDI learn target for preset graph.
    pub learn: preset_graph::MidiLearnTarget,
}

impl MidiController {
    pub fn new() -> Self {
        let ports = midi_input::list_midi_ports();
        log::info!("MIDI ports found: {:?}", ports);
        Self {
            connection: None,
            rx: None,
            port_names: ports,
            selected_port: 0,
            knob_ccs: MINILAB3_KNOB_CCS.to_vec(),
            knob_learning: false,
            knob_last: HashMap::new(),
            fader_ccs: MINILAB3_FADER_CCS.to_vec(),
            fader_learning: false,
            learn_node: None,
            learn: preset_graph::MidiLearnTarget::None,
        }
    }

    /// Drain all pending MIDI events from the receiver.
    pub fn drain_events(&self) -> Vec<midi_input::MidiEvent> {
        let mut events = Vec::new();
        if let Some(ref rx) = self.rx {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        events
    }

    /// Connect to the currently selected MIDI port.
    pub fn connect(&mut self) -> Result<(), String> {
        match midi_input::connect(self.selected_port) {
            Ok((conn, rx)) => {
                if let Some(mapping) = MidiMapping::load(&conn.port_name) {
                    self.knob_ccs = mapping.knob_ccs;
                    self.fader_ccs = mapping.fader_ccs;
                }
                self.knob_last.clear();
                self.connection = Some(conn);
                self.rx = Some(rx);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Disconnect the current MIDI connection.
    pub fn disconnect(&mut self) {
        self.connection = None;
        self.rx = None;
    }

    /// Refresh available MIDI ports.
    pub fn refresh_ports(&mut self) {
        self.port_names = midi_input::list_midi_ports();
    }

    /// Save the current knob/fader mapping for the connected device.
    pub fn save_mapping(&self) -> Result<(), String> {
        let port_name = self
            .connection
            .as_ref()
            .map(|c| c.port_name.as_str())
            .ok_or("No MIDI connection")?;
        MidiMapping::save(port_name, &self.knob_ccs, &self.fader_ccs)
    }

    /// Get the connected device filename for display.
    pub fn device_filename(&self) -> Option<String> {
        self.connection
            .as_ref()
            .map(|c| MidiMapping::device_filename(&c.port_name))
    }

    /// Process a knob CC and return the delta value and knob index.
    pub fn process_knob_delta(&mut self, cc: u8, value: u8) -> Option<(usize, i16)> {
        let knob_idx = self.knob_ccs.iter().position(|&c| c == cc)?;
        let last = self.knob_last.get(&cc).copied();
        self.knob_last.insert(cc, value);
        let delta = if let Some(prev) = last {
            let d = value as i16 - prev as i16;
            if d > 64 {
                d - 128
            } else if d < -64 {
                d + 128
            } else {
                d
            }
        } else {
            0
        };
        if delta != 0 {
            Some((knob_idx, delta))
        } else {
            None
        }
    }

    /// Check if a CC belongs to a fader and return its index.
    pub fn fader_index(&self, cc: u8) -> Option<usize> {
        self.fader_ccs.iter().position(|&c| c == cc)
    }

    /// Start learning knobs (clears existing assignments).
    pub fn start_knob_learn(&mut self) {
        self.knob_ccs.clear();
        self.knob_learning = true;
    }

    /// Start learning faders (clears existing assignments).
    pub fn start_fader_learn(&mut self) {
        self.fader_ccs.clear();
        self.fader_learning = true;
    }

    /// Process a CC during knob learning. Returns true if consumed.
    pub fn learn_knob_cc(&mut self, cc: u8) -> bool {
        if !self.knob_learning {
            return false;
        }
        if !self.knob_ccs.contains(&cc) {
            self.fader_ccs.retain(|&c| c != cc);
            self.knob_ccs.push(cc);
        }
        true
    }

    /// Process a CC during fader learning. Returns true if consumed.
    pub fn learn_fader_cc(&mut self, cc: u8) -> bool {
        if !self.fader_learning {
            return false;
        }
        if !self.fader_ccs.contains(&cc) {
            self.knob_ccs.retain(|&c| c != cc);
            self.fader_ccs.push(cc);
        }
        true
    }

    /// Check if a CC is in the knob or fader list (for filtering in node learn).
    pub fn is_mapped_cc(&self, cc: u8) -> bool {
        self.knob_ccs.contains(&cc) || self.fader_ccs.contains(&cc)
    }
}

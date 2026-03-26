//! MIDI input for external controller support (Boss GT-1B footswitches, etc.)

use std::sync::mpsc;

/// A MIDI event received from an external controller.
#[derive(Debug, Clone)]
pub enum MidiEvent {
    ControlChange { channel: u8, cc: u8, value: u8 },
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    ProgramChange { channel: u8, program: u8 },
}

/// List available MIDI input port names.
pub fn list_midi_ports() -> Vec<String> {
    let midi_in = match midir::MidiInput::new("rifflab-probe") {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Failed to create MIDI input for probing: {e}");
            return Vec::new();
        }
    };
    midi_in.ports().iter()
        .filter_map(|p| midi_in.port_name(p).ok())
        .collect()
}

/// Active MIDI input connection.
/// Receives events via mpsc channel (non-blocking from UI thread).
pub struct MidiConnection {
    // Hold the connection to keep it alive. Drop = disconnect.
    _connection: midir::MidiInputConnection<()>,
    pub port_name: String,
}

/// Connect to a MIDI input port by index.
/// Returns the connection and a receiver for MIDI events.
pub fn connect(port_index: usize) -> Result<(MidiConnection, mpsc::Receiver<MidiEvent>), String> {
    let midi_in = midir::MidiInput::new("rifflab-midi")
        .map_err(|e| format!("MIDI init failed: {e}"))?;

    let ports = midi_in.ports();
    let port = ports.get(port_index)
        .ok_or_else(|| format!("MIDI port index {} out of range ({})", port_index, ports.len()))?;

    let port_name = midi_in.port_name(port)
        .unwrap_or_else(|_| format!("Port {}", port_index));

    let (tx, rx) = mpsc::channel();

    let connection = midi_in.connect(
        port,
        "rifflab-input",
        move |_timestamp, message, _| {
            if message.len() >= 2 {
                let status = message[0] & 0xF0;
                let channel = message[0] & 0x0F;
                match status {
                    0xB0 if message.len() >= 3 => {
                        let _ = tx.send(MidiEvent::ControlChange {
                            channel, cc: message[1], value: message[2],
                        });
                    }
                    0x90 if message.len() >= 3 => {
                        if message[2] > 0 {
                            let _ = tx.send(MidiEvent::NoteOn {
                                channel, note: message[1], velocity: message[2],
                            });
                        } else {
                            let _ = tx.send(MidiEvent::NoteOff { channel, note: message[1] });
                        }
                    }
                    0x80 if message.len() >= 3 => {
                        let _ = tx.send(MidiEvent::NoteOff { channel, note: message[1] });
                    }
                    0xC0 => {
                        let _ = tx.send(MidiEvent::ProgramChange {
                            channel, program: message[1],
                        });
                    }
                    _ => {}
                }
            }
        },
        (),
    ).map_err(|e| format!("MIDI connect failed: {e}"))?;

    log::info!("MIDI connected to: {}", port_name);

    Ok((
        MidiConnection {
            _connection: connection,
            port_name,
        },
        rx,
    ))
}

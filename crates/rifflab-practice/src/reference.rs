use rifflab_core::analysis::NoteEvent;
use std::path::Path;

/// Load a reference note sequence from a JSON file (M4 transcription output).
pub fn load_from_json(path: &Path) -> Result<Vec<NoteEvent>, std::io::Error> {
    let data = std::fs::read_to_string(path)?;
    serde_json::from_str(&data).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Load a reference from a MIDI file.
pub fn load_from_midi(path: &Path) -> Result<Vec<NoteEvent>, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    let smf = midly::Smf::parse(&data)?;

    let mut notes = Vec::new();
    let ticks_per_beat = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int() as f64,
        _ => 480.0,
    };

    // Assume 120 BPM default, refine if tempo events found
    let mut tempo_us = 500_000.0; // microseconds per beat (120 BPM)

    for track in &smf.tracks {
        let mut time_ticks: u64 = 0;
        let mut active_notes: std::collections::HashMap<u8, f64> = std::collections::HashMap::new();

        for event in track {
            time_ticks += event.delta.as_int() as u64;
            let time_seconds = (time_ticks as f64 / ticks_per_beat) * (tempo_us / 1_000_000.0);

            match event.kind {
                midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) => {
                    tempo_us = t.as_int() as f64;
                }
                midly::TrackEventKind::Midi { message, .. } => match message {
                    midly::MidiMessage::NoteOn { key, vel } => {
                        if vel.as_int() > 0 {
                            active_notes.insert(key.as_int(), time_seconds);
                        } else if let Some(onset) = active_notes.remove(&key.as_int()) {
                            notes.push(NoteEvent {
                                midi_note: key.as_int(),
                                onset_seconds: onset,
                                duration_seconds: time_seconds - onset,
                                average_cents: 0.0,
                                confidence: 1.0,
                            });
                        }
                    }
                    midly::MidiMessage::NoteOff { key, .. } => {
                        if let Some(onset) = active_notes.remove(&key.as_int()) {
                            notes.push(NoteEvent {
                                midi_note: key.as_int(),
                                onset_seconds: onset,
                                duration_seconds: time_seconds - onset,
                                average_cents: 0.0,
                                confidence: 1.0,
                            });
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    notes.sort_by(|a, b| a.onset_seconds.partial_cmp(&b.onset_seconds).unwrap());
    Ok(notes)
}

// Re-export serde_json for reference loading
use serde_json;

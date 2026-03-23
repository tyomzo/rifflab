use rifflab_core::analysis::NoteEvent;

/// Combine offline pitch detection + onset detection into discrete NoteEvents.
pub fn transcribe_notes(samples: &[f32], sample_rate: u32) -> Vec<NoteEvent> {
    // TODO: run offline YIN + onset detection, merge into note events
    let _ = (samples, sample_rate);
    Vec::new()
}

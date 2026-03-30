//! Audio-to-Tab transcription pipeline.
//!
//! Steps: onset detection → per-onset pitch (YIN) → note segmentation → fret mapping → beat grid.

use crate::fret_map;
use crate::model::{NoteSource, TabNote, Technique, STANDARD_TUNING};

/// Transcribed note from audio (intermediate representation).
#[derive(Debug, Clone)]
pub struct TranscribedNote {
    pub onset_secs: f64,
    pub offset_secs: f64,
    pub midi_note: u8,
    pub confidence: f32,
}

/// Transcribe mono audio samples into TabNotes.
///
/// `samples`: mono f32 audio.
/// `sample_rate`: e.g., 44100.
/// `tuning`: bass string open MIDI notes, low to high.
pub fn transcribe_audio(
    samples: &[f32],
    sample_rate: u32,
    tuning: &[u8; 4],
) -> Vec<TabNote> {
    // Step 1: Onset detection
    let onsets = rifflab_analysis::onset::detect_onsets(samples, sample_rate);
    if onsets.is_empty() {
        return Vec::new();
    }

    // Step 2: Per-onset pitch estimation using YIN
    let mut yin = rifflab_analysis::pitch::yin::YinDetector::new(sample_rate);
    let transcribed = estimate_notes(&onsets, samples, sample_rate, &mut yin);

    // Step 3: Convert to TabNotes with fret mapping
    let mut tab_notes = Vec::new();
    let mut prev_fret: Option<(u8, u8)> = None;

    for tn in &transcribed {
        if tn.midi_note == 0 || tn.confidence < 0.3 {
            continue; // Skip silence/low-confidence
        }

        let (string, fret) = fret_map::midi_to_fret(tn.midi_note, tuning, prev_fret);
        prev_fret = Some((string, fret));

        let mut note = TabNote::new(string, fret, NoteSource::AudioTranscription);
        note.time_secs = tn.onset_secs;
        note.duration_secs = tn.offset_secs - tn.onset_secs;
        note.confidence = tn.confidence;
        tab_notes.push(note);
    }

    tab_notes
}

/// Estimate pitch for each onset region.
fn estimate_notes(
    onsets: &[f64],
    samples: &[f32],
    sample_rate: u32,
    yin: &mut rifflab_analysis::pitch::yin::YinDetector,
) -> Vec<TranscribedNote> {
    let total_duration = samples.len() as f64 / sample_rate as f64;
    let mut notes = Vec::new();

    for (i, &onset) in onsets.iter().enumerate() {
        let offset = if i + 1 < onsets.len() {
            onsets[i + 1]
        } else {
            total_duration
        };

        // Extract the segment for this note
        let start_sample = (onset * sample_rate as f64) as usize;
        let end_sample = ((offset * sample_rate as f64) as usize).min(samples.len());
        if end_sample <= start_sample || end_sample - start_sample < 512 {
            continue;
        }

        // Use YIN batch detection on this segment, take the mode/median pitch
        let segment = &samples[start_sample..end_sample];
        let frames = yin.detect_batch(segment, sample_rate);

        // Filter out silence frames and collect valid pitches
        let valid: Vec<(f32, f32)> = frames.iter()
            .filter(|&&(_, freq, conf)| freq > 20.0 && conf > 0.3)
            .map(|&(_, freq, conf)| (freq, conf))
            .collect();

        if valid.is_empty() {
            continue;
        }

        // Use median frequency (more robust than mean against octave errors)
        let mut freqs: Vec<f32> = valid.iter().map(|&(f, _)| f).collect();
        freqs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_freq = freqs[freqs.len() / 2];
        let avg_conf = valid.iter().map(|&(_, c)| c).sum::<f32>() / valid.len() as f32;

        // Convert frequency to MIDI note
        let midi_note = freq_to_midi(median_freq);

        notes.push(TranscribedNote {
            onset_secs: onset,
            offset_secs: offset,
            midi_note,
            confidence: avg_conf,
        });
    }

    notes
}

/// Convert frequency in Hz to nearest MIDI note number.
fn freq_to_midi(freq: f32) -> u8 {
    if freq <= 0.0 { return 0; }
    let midi = 69.0 + 12.0 * (freq / 440.0).log2();
    midi.round().clamp(0.0, 127.0) as u8
}

/// Estimate BPM from onset times using autocorrelation.
pub fn estimate_bpm(onsets: &[f64]) -> f64 {
    if onsets.len() < 4 {
        return 120.0; // default
    }

    // Compute inter-onset intervals
    let iois: Vec<f64> = onsets.windows(2).map(|w| w[1] - w[0]).collect();

    // Find the most common IOI (mode) using a histogram with 5ms bins
    let bin_size = 0.005;
    let mut bins: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for &ioi in &iois {
        if ioi > 0.1 && ioi < 2.0 { // reasonable range: 30-600 BPM
            let bin = (ioi / bin_size) as u32;
            *bins.entry(bin).or_default() += 1;
        }
    }

    if let Some((&best_bin, _)) = bins.iter().max_by_key(|&(_, &count)| count) {
        let beat_duration = best_bin as f64 * bin_size;
        if beat_duration > 0.0 {
            return (60.0 / beat_duration).clamp(40.0, 300.0);
        }
    }

    120.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freq_to_midi() {
        assert_eq!(freq_to_midi(440.0), 69); // A4
        assert_eq!(freq_to_midi(261.63), 60); // C4
        assert_eq!(freq_to_midi(82.41), 40); // E2
        assert_eq!(freq_to_midi(41.20), 28); // E1 (bass open)
    }

    #[test]
    fn test_estimate_bpm() {
        // 120 BPM = 0.5s per beat
        let onsets = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0];
        let bpm = estimate_bpm(&onsets);
        assert!((bpm - 120.0).abs() < 10.0, "Expected ~120 BPM, got {}", bpm);
    }

    #[test]
    fn test_transcribe_silence() {
        let silence = vec![0.0f32; 44100 * 2];
        let notes = transcribe_audio(&silence, 44100, &STANDARD_TUNING);
        assert!(notes.is_empty());
    }
}

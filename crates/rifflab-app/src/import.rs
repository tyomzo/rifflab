use anyhow::{Context, Result};
use rifflab_core::analysis::NoteEvent;
use rifflab_core::song::{BeatGrid, StemInfo, StemType};
use std::path::Path;

use crate::decode;
use crate::library::Library;

/// Result of importing a song into the library.
#[allow(dead_code)]
pub struct ImportResult {
    /// Unique song identifier (UUID string).
    pub song_id: String,
    /// Original file name (for display).
    pub original_name: String,
    /// Decoded audio data (resampled to engine rate), interleaved f32.
    pub audio_data: Vec<f32>,
    /// Number of channels.
    pub channels: u16,
    /// Sample rate of the decoded audio.
    pub sample_rate: u32,
    /// Total frames in the decoded audio.
    pub total_frames: u64,
    /// Stem info entries (one per available stem, or just the original).
    pub stems: Vec<StemInfo>,
    /// Beat grid from beat tracking (None if unavailable).
    pub beat_grid: Option<BeatGrid>,
    /// Transcribed reference notes (from offline pitch + onset detection).
    pub reference_notes: Vec<NoteEvent>,
}

/// Import a song file into the library.
///
/// Pipeline:
/// 1. Generate a song ID (UUID).
/// 2. Decode the audio file (WAV/FLAC/MP3) via symphonia.
/// 3. Resample to the target sample rate.
/// 4. Copy the decoded audio to the library song directory.
/// 5. Try stem separation (skipped in Phase 1 -- no Python deps).
/// 6. Try beat tracking (skipped in Phase 1).
/// 7. Run offline note transcription (pure Rust: YIN + onset detection).
/// 8. Return ImportResult with all data needed to load into the engine.
pub fn import_song(
    path: &Path,
    library: &Library,
    target_sample_rate: u32,
) -> Result<ImportResult> {
    let original_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    log::info!("Importing: {} ({})", original_name, path.display());

    // 1. Generate song ID
    let song_id = uuid::Uuid::new_v4().to_string();
    log::info!("Song ID: {}", song_id);

    // 2. Decode audio file
    let decoded = decode::decode_file(path)
        .with_context(|| format!("Failed to decode {}", path.display()))?;

    log::info!(
        "Decoded: {} frames, {}ch, {}Hz",
        decoded.frames,
        decoded.channels,
        decoded.sample_rate
    );

    // 3. Resample to target rate
    let decoded = decode::resample(decoded, target_sample_rate);

    // 4. Save to library (create song dir, write original.wav metadata)
    let song_dir = library.ensure_song_dir(&song_id)?;
    let metadata_path = song_dir.join("metadata.toml");
    let metadata = SongMetadata {
        original_file: path.to_string_lossy().to_string(),
        original_name: original_name.clone(),
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        frames: decoded.frames,
        duration_seconds: decoded.frames as f64 / decoded.sample_rate as f64,
    };
    let toml_str = toml::to_string_pretty(&metadata)
        .context("Failed to serialize song metadata")?;
    std::fs::write(&metadata_path, &toml_str)
        .with_context(|| format!("Failed to write {}", metadata_path.display()))?;
    log::info!("Wrote metadata to {}", metadata_path.display());

    // 5. Stem separation -- skipped (no Python deps in Phase 1)
    //    Use original file as a single "Other" stem.
    let stem_info = StemInfo {
        stem_type: StemType::Other,
        path: path.to_path_buf(),
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        duration_seconds: decoded.frames as f64 / decoded.sample_rate as f64,
        num_frames: decoded.frames,
    };
    let stems = vec![stem_info];
    log::info!("Stem separation skipped (Phase 1) -- using original as single stem");

    // 6. Beat tracking -- skipped (no madmom)
    let beat_grid = None;
    log::info!("Beat tracking skipped (Phase 1) -- no madmom");

    // 7. Offline note transcription (pure Rust)
    let reference_notes = run_transcription(&decoded.data, decoded.channels, decoded.sample_rate);
    log::info!(
        "Transcription complete: {} notes detected",
        reference_notes.len()
    );

    Ok(ImportResult {
        song_id,
        original_name,
        audio_data: decoded.data,
        channels: decoded.channels,
        sample_rate: decoded.sample_rate,
        total_frames: decoded.frames,
        stems,
        beat_grid,
        reference_notes,
    })
}

/// Run offline note transcription on audio data.
///
/// Downmixes to mono before running YIN pitch detection + onset detection.
fn run_transcription(data: &[f32], channels: u16, sample_rate: u32) -> Vec<NoteEvent> {
    let mono = downmix_to_mono(data, channels);
    rifflab_analysis::transcribe::transcribe_notes(&mono, sample_rate)
}

/// Downmix interleaved multi-channel audio to mono by averaging channels.
fn downmix_to_mono(data: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return data.to_vec();
    }
    let num_frames = data.len() / ch;
    let mut mono = Vec::with_capacity(num_frames);
    for frame in 0..num_frames {
        let mut sum = 0.0f32;
        for c in 0..ch {
            sum += data[frame * ch + c];
        }
        mono.push(sum / ch as f32);
    }
    mono
}

/// Song metadata persisted alongside the audio in the library.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SongMetadata {
    original_file: String,
    original_name: String,
    sample_rate: u32,
    channels: u16,
    frames: u64,
    duration_seconds: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downmix_mono_passthrough() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let mono = downmix_to_mono(&data, 1);
        assert_eq!(mono, data);
    }

    #[test]
    fn test_downmix_stereo() {
        // Stereo interleaved: (L=1, R=3), (L=2, R=4)
        let data = vec![1.0, 3.0, 2.0, 4.0];
        let mono = downmix_to_mono(&data, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 2.0).abs() < 1e-6); // (1+3)/2
        assert!((mono[1] - 3.0).abs() < 1e-6); // (2+4)/2
    }

    #[test]
    fn test_downmix_empty() {
        let data: Vec<f32> = vec![];
        let mono = downmix_to_mono(&data, 2);
        assert!(mono.is_empty());
    }

    #[test]
    fn test_import_nonexistent_file() {
        let tmp = std::env::temp_dir().join(format!("rifflab_import_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let lib = Library::init_at(tmp.clone()).unwrap();

        let result = import_song(Path::new("/nonexistent/file.wav"), &lib, 48000);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

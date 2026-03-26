use anyhow::{Context, Result};
use rifflab_core::preset::EffectPreset;
use rifflab_core::song::StemType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Session manifest saved as session.toml in the session directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    pub name: String,
    pub original_file: String,
    pub sample_rate: u32,
    pub stems: Vec<StemEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StemEntry {
    pub stem_type: String,
    pub filename: String,
    pub channels: u16,
    pub frames: u64,
}

/// Save a session: writes stem WAVs + manifest + effects preset to a directory.
pub fn save_session(
    dir: &Path,
    name: &str,
    original_file: &str,
    sample_rate: u32,
    stems: &[(StemType, &[f32], u16, u64)], // (type, data, channels, frames)
    effects_preset: Option<&EffectPreset>,
    cue_list: Option<&rifflab_cue::CueList>,
    fx_graph: Option<&crate::node_editor::FxGraph>,
) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create session dir: {}", dir.display()))?;

    let stems_dir = dir.join("stems");
    std::fs::create_dir_all(&stems_dir)?;

    let mut manifest_stems = Vec::new();

    for (stem_type, data, channels, frames) in stems {
        let filename = format!("{}.wav", stem_type_str(stem_type));
        let wav_path = stems_dir.join(&filename);

        write_wav(&wav_path, data, *channels, sample_rate)
            .with_context(|| format!("Failed to write {}", wav_path.display()))?;

        manifest_stems.push(StemEntry {
            stem_type: stem_type_str(stem_type).to_string(),
            filename,
            channels: *channels,
            frames: *frames,
        });
    }

    // Write manifest
    let manifest = SessionManifest {
        name: name.to_string(),
        original_file: original_file.to_string(),
        sample_rate,
        stems: manifest_stems,
    };
    let manifest_toml = toml::to_string_pretty(&manifest)?;
    std::fs::write(dir.join("session.toml"), manifest_toml)?;

    // Write effects preset if present
    if let Some(preset) = effects_preset {
        rifflab_fx::preset::save_preset(preset, &dir.join("effects.toml"))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    // Write cues if present
    if let Some(cues) = cue_list {
        if !cues.is_empty() {
            rifflab_cue::save_cues(&dir.join("cues.json"), cues)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
    }

    // Write effects graph if present
    if let Some(graph) = fx_graph {
        crate::node_editor::save_graph(&dir.join("graph.json"), graph)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    log::info!("Session saved to {}", dir.display());
    Ok(())
}

/// Load a session from a directory.
pub fn load_session(dir: &Path) -> Result<LoadedSession> {
    let manifest_path = dir.join("session.toml");
    let manifest_str = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("No session.toml in {}", dir.display()))?;
    let manifest: SessionManifest = toml::from_str(&manifest_str)?;

    let stems_dir = dir.join("stems");
    let mut stems = Vec::new();

    for entry in &manifest.stems {
        let wav_path = stems_dir.join(&entry.filename);
        let decoded = crate::decode::decode_file(&wav_path)
            .with_context(|| format!("Failed to decode stem {}", wav_path.display()))?;
        let stem_type = parse_stem_type(&entry.stem_type);
        stems.push((stem_type, decoded));
    }

    // Load effects preset if present
    let effects_path = dir.join("effects.toml");
    let effects_preset = if effects_path.exists() {
        rifflab_fx::preset::load_preset(&effects_path)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .ok()
    } else {
        None
    };

    // Load cues if present
    let cues_path = dir.join("cues.json");
    let cue_list = if cues_path.exists() {
        rifflab_cue::load_cues(&cues_path)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .ok()
    } else {
        None
    };

    // Load effects graph if present
    let graph_path = dir.join("graph.json");
    let fx_graph = if graph_path.exists() {
        crate::node_editor::load_graph(&graph_path)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .ok()
    } else {
        None
    };

    Ok(LoadedSession {
        manifest,
        stems,
        effects_preset,
        cue_list,
        fx_graph,
        session_dir: dir.to_path_buf(),
    })
}

pub struct LoadedSession {
    pub manifest: SessionManifest,
    pub stems: Vec<(StemType, crate::decode::DecodedAudio)>,
    pub effects_preset: Option<EffectPreset>,
    pub cue_list: Option<rifflab_cue::CueList>,
    pub fx_graph: Option<crate::node_editor::FxGraph>,
    #[allow(dead_code)]
    pub session_dir: PathBuf,
}

fn stem_type_str(st: &StemType) -> &'static str {
    match st {
        StemType::Vocals => "vocals",
        StemType::Drums => "drums",
        StemType::Bass => "bass",
        StemType::Guitar => "guitar",
        StemType::Piano => "piano",
        StemType::Other => "other",
    }
}

fn parse_stem_type(s: &str) -> StemType {
    match s {
        "vocals" => StemType::Vocals,
        "drums" => StemType::Drums,
        "bass" => StemType::Bass,
        "guitar" => StemType::Guitar,
        "piano" => StemType::Piano,
        _ => StemType::Other,
    }
}

/// Write interleaved f32 audio as a WAV file.
fn write_wav(path: &Path, data: &[f32], channels: u16, sample_rate: u32) -> Result<()> {
    use std::io::Write;

    let num_samples = data.len();
    let bytes_per_sample = 4u16; // f32
    let block_align = channels * bytes_per_sample;
    let byte_rate = sample_rate * block_align as u32;
    let data_size = (num_samples * 4) as u32;

    let mut file = std::fs::File::create(path)?;

    // RIFF header
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_size).to_le_bytes())?;
    file.write_all(b"WAVE")?;

    // fmt chunk (IEEE float)
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // chunk size
    file.write_all(&3u16.to_le_bytes())?; // format = IEEE float
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&(bytes_per_sample * 8).to_le_bytes())?; // bits per sample

    // data chunk
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    // Write samples as little-endian f32
    for &sample in data {
        file.write_all(&sample.to_le_bytes())?;
    }

    Ok(())
}

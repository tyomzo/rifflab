//! Background stem loading: decode audio, run Demucs, load separated stems.
//!
//! Encapsulates the subprocess management and audio decoding that was previously
//! inline in the app crate's `load_file()` method.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use rifflab_core::song::StemType;

/// Decoded audio data (interleaved f32) at its original sample rate.
pub struct DecodedStem {
    pub stem_type: StemType,
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub name: String,
}

/// Messages sent from the loading thread to the UI.
pub enum LoadMsg {
    /// Progress status text.
    Status(String),
    /// Loading finished successfully.
    Done {
        file_name: String,
        stems: Vec<DecodedStem>,
    },
    /// Loading failed.
    Error(String),
}

/// Spawn a background thread that decodes an audio file and optionally
/// separates it into stems using Demucs. Sends progress via the returned receiver.
///
/// Stems are returned at their original sample rate — the caller is responsible
/// for resampling to match the audio engine.
pub fn load_in_background(path: &Path) -> mpsc::Receiver<LoadMsg> {
    let (tx, rx) = mpsc::channel();
    let path_buf = path.to_path_buf();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown")
        .to_string();

    std::thread::spawn(move || {
        let _ = tx.send(LoadMsg::Status(format!("Decoding {}...", file_name)));

        let decoded = match decode_file(&path_buf) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(LoadMsg::Error(format!("Decode failed: {e}")));
                return;
            }
        };

        let duration_secs = decoded.frames as f64 / decoded.sample_rate as f64;
        let _ = tx.send(LoadMsg::Status(format!(
            "Decoded: {:.1}s, {}ch, {}Hz",
            duration_secs, decoded.channels, decoded.sample_rate,
        )));

        // Try stem separation with demucs
        let _ = tx.send(LoadMsg::Status("Separating stems with Demucs (GPU)...".into()));
        let stem_dir = std::env::temp_dir().join(format!("rifflab_stems_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&stem_dir);

        let worker_script = find_demucs_script();
        let demucs_out_dir = stem_dir.join("stems");

        let demucs_success = run_demucs(&worker_script, &path_buf, &demucs_out_dir, &tx);

        let stem_names = ["vocals", "drums", "bass", "other"];
        let stem_types = [StemType::Vocals, StemType::Drums, StemType::Bass, StemType::Other];

        let mut stems: Vec<DecodedStem> = Vec::new();

        if demucs_success && demucs_out_dir.exists() {
            let _ = tx.send(LoadMsg::Status("Loading separated stems...".into()));

            for (stem_name, stem_type) in stem_names.iter().zip(stem_types.iter()) {
                let stem_path = demucs_out_dir.join(format!("{}.wav", stem_name));
                if stem_path.exists() {
                    match decode_file(&stem_path) {
                        Ok(d) => {
                            let _ = tx.send(LoadMsg::Status(format!("Loaded stem: {}", stem_name)));
                            stems.push(DecodedStem {
                                stem_type: *stem_type,
                                data: d.data,
                                sample_rate: d.sample_rate,
                                channels: d.channels,
                                frames: d.frames,
                                name: stem_name.to_string(),
                            });
                        }
                        Err(e) => {
                            log::warn!("Failed to decode stem {}: {e}", stem_name);
                        }
                    }
                }
            }

            let _ = std::fs::remove_dir_all(&stem_dir);
        }

        // Fall back to single track if demucs failed or produced no stems
        if stems.is_empty() {
            if !demucs_success {
                let _ = tx.send(LoadMsg::Status("Demucs failed, loading as single track...".into()));
            } else {
                let _ = tx.send(LoadMsg::Status("No stems found, loading as single track...".into()));
            }

            stems.push(DecodedStem {
                stem_type: StemType::Other,
                name: file_name.clone(),
                sample_rate: decoded.sample_rate,
                channels: decoded.channels,
                frames: decoded.frames,
                data: decoded.data,
            });
        }

        let _ = tx.send(LoadMsg::Done { file_name, stems });
    });

    rx
}

// ─── Demucs subprocess ──────────────────────────────────────────────────────

fn find_demucs_script() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let workspace_root = exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));
    workspace_root.join("workers").join("run_demucs.py")
}

fn run_demucs(
    script: &Path,
    input: &Path,
    output_dir: &Path,
    tx: &mpsc::Sender<LoadMsg>,
) -> bool {
    use std::io::BufRead;

    let mut child = match std::process::Command::new("python3")
        .arg(script)
        .arg(input)
        .arg(output_dir)
        .arg("htdemucs_ft")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(LoadMsg::Status(format!("Demucs spawn failed: {e}")));
            return false;
        }
    };

    if let Some(stdout) = child.stdout.take() {
        let reader = std::io::BufReader::new(stdout);
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            for line in reader.lines().flatten() {
                if !line.trim().is_empty() {
                    let _ = tx2.send(LoadMsg::Status(format!("[demucs] {}", line.trim())));
                }
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let reader = std::io::BufReader::new(stderr);
        let tx3 = tx.clone();
        std::thread::spawn(move || {
            for line in reader.lines().flatten() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.contains("UserWarning") {
                    let _ = tx3.send(LoadMsg::Status(format!("[demucs] {}", trimmed)));
                }
            }
        });
    }

    match child.wait() {
        Ok(status) => status.success(),
        Err(e) => {
            let _ = tx.send(LoadMsg::Status(format!("Demucs wait failed: {e}")));
            false
        }
    }
}

// ─── Audio decode (self-contained, no dependency on app crate) ──────────────

struct RawAudio {
    data: Vec<f32>,
    sample_rate: u32,
    channels: u16,
    frames: u64,
}

fn decode_file(path: &Path) -> anyhow::Result<RawAudio> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("No audio track found"))?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow::anyhow!("No sample rate"))?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(2);
    let track_id = track.id;

    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let spec = *decoded.spec();
        let num_frames = decoded.capacity();
        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(sample_buf.samples());
    }

    let total_frames = all_samples.len() as u64 / channels.max(1) as u64;

    Ok(RawAudio {
        data: all_samples,
        sample_rate,
        channels,
        frames: total_frames,
    })
}

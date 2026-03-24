use anyhow::{Context, Result};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded audio data ready for playback.
pub struct DecodedAudio {
    /// Interleaved f32 samples.
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

/// Decode an audio file (WAV, FLAC, MP3) into interleaved f32 samples.
pub fn decode_file(path: &Path) -> Result<DecodedAudio> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open {}", path.display()))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Probe the format
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Failed to probe audio format")?;

    let mut format = probed.format;

    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .context("No audio track found")?;

    let codec_params = track.codec_params.clone();
    let track_id = track.id;

    let sample_rate = codec_params
        .sample_rate
        .context("No sample rate in codec params")?;
    let channels = codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(2);

    // Create decoder
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .context("Failed to create decoder")?;

    // Decode all packets
    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break; // End of stream
            }
            Err(e) => return Err(e.into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        let duration = decoded.capacity();

        let mut sample_buf = SampleBuffer::<f32>::new(duration as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(sample_buf.samples());
    }

    let frames = all_samples.len() as u64 / channels as u64;

    log::info!(
        "Decoded {}: {} frames, {}ch, {}Hz ({:.1}s)",
        path.display(),
        frames,
        channels,
        sample_rate,
        frames as f64 / sample_rate as f64,
    );

    Ok(DecodedAudio {
        data: all_samples,
        sample_rate,
        channels,
        frames,
    })
}

/// Resample decoded audio to a target sample rate using linear interpolation.
/// Returns a new DecodedAudio at the target rate.
pub fn resample(audio: DecodedAudio, target_rate: u32) -> DecodedAudio {
    if audio.sample_rate == target_rate {
        return audio;
    }

    let ratio = target_rate as f64 / audio.sample_rate as f64;
    let ch = audio.channels as usize;
    let src_frames = audio.frames as usize;
    let dst_frames = (src_frames as f64 * ratio).ceil() as usize;

    let mut output = vec![0.0f32; dst_frames * ch];

    for frame in 0..dst_frames {
        let src_pos = frame as f64 / ratio;
        let src_idx = src_pos.floor() as usize;
        let frac = (src_pos - src_idx as f64) as f32;

        for c in 0..ch {
            let i0 = src_idx * ch + c;
            let i1 = ((src_idx + 1).min(src_frames - 1)) * ch + c;
            let s0 = audio.data.get(i0).copied().unwrap_or(0.0);
            let s1 = audio.data.get(i1).copied().unwrap_or(0.0);
            output[frame * ch + c] = s0 + frac * (s1 - s0);
        }
    }

    log::info!(
        "Resampled {}Hz → {}Hz ({} → {} frames)",
        audio.sample_rate, target_rate, src_frames, dst_frames
    );

    DecodedAudio {
        data: output,
        sample_rate: target_rate,
        channels: audio.channels,
        frames: dst_frames as u64,
    }
}

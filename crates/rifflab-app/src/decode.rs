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

/// Resample a single channel of f64 samples using sinc interpolation.
fn resample_channel(input: &[f64], ratio: f64) -> Vec<f64> {
    use rubato::{
        Async, FixedAsync, Indexing, Resampler,
        SincInterpolationParameters, SincInterpolationType, WindowFunction,
        calculate_cutoff,
    };
    use audioadapter_buffers::direct::InterleavedSlice;

    let src_frames = input.len();
    let sinc_len = 128;
    let window = WindowFunction::Blackman2;
    let params = SincInterpolationParameters {
        sinc_len,
        f_cutoff: calculate_cutoff(sinc_len, window),
        interpolation: SincInterpolationType::Quadratic,
        oversampling_factor: 256,
        window,
    };

    let chunk_size = 1024;
    let mut resampler = Async::<f64>::new_sinc(
        ratio,
        1.1,
        &params,
        chunk_size,
        1, // single channel
        FixedAsync::Input,
    ).expect("Failed to create resampler");

    let out_capacity = (src_frames as f64 * ratio * 1.1) as usize + chunk_size;
    let mut outdata = vec![0.0f64; out_capacity];

    let input_adapter = InterleavedSlice::new(input, 1, src_frames).unwrap();
    let mut output_adapter =
        InterleavedSlice::new_mut(&mut outdata, 1, out_capacity).unwrap();

    let mut indexing = Indexing {
        input_offset: 0,
        output_offset: 0,
        active_channels_mask: None,
        partial_len: None,
    };

    let mut input_frames_left = src_frames;
    let mut input_frames_next = resampler.input_frames_next();

    while input_frames_left >= input_frames_next {
        let (nbr_in, nbr_out) = resampler
            .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
            .unwrap();
        indexing.input_offset += nbr_in;
        indexing.output_offset += nbr_out;
        input_frames_left -= nbr_in;
        input_frames_next = resampler.input_frames_next();
    }

    if input_frames_left > 0 {
        indexing.partial_len = Some(input_frames_left);
        let (_nbr_in, nbr_out) = resampler
            .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
            .unwrap();
        indexing.output_offset += nbr_out;
    }

    outdata.truncate(indexing.output_offset);
    outdata
}

/// Resample decoded audio to a target sample rate using sinc interpolation (rubato).
/// Each channel is resampled in parallel on its own thread.
pub fn resample(audio: DecodedAudio, target_rate: u32) -> DecodedAudio {
    if audio.sample_rate == target_rate {
        return audio;
    }

    let ch = audio.channels as usize;
    let src_frames = audio.frames as usize;
    let ratio = target_rate as f64 / audio.sample_rate as f64;

    // De-interleave into per-channel Vec<f64>
    let mut channels: Vec<Vec<f64>> = vec![Vec::with_capacity(src_frames); ch];
    for frame in 0..src_frames {
        for c in 0..ch {
            channels[c].push(audio.data[frame * ch + c] as f64);
        }
    }

    // Resample each channel in parallel
    let resampled: Vec<Vec<f64>> = std::thread::scope(|s| {
        let handles: Vec<_> = channels
            .into_iter()
            .map(|chan_data| s.spawn(move || resample_channel(&chan_data, ratio)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Re-interleave
    let dst_frames = resampled[0].len();
    let mut output = vec![0.0f32; dst_frames * ch];
    for frame in 0..dst_frames {
        for c in 0..ch {
            output[frame * ch + c] = resampled[c][frame] as f32;
        }
    }

    log::info!(
        "Resampled {}Hz -> {}Hz ({} -> {} frames, sinc, {}ch parallel)",
        audio.sample_rate, target_rate, src_frames, dst_frames, ch
    );

    DecodedAudio {
        data: output,
        sample_rate: target_rate,
        channels: audio.channels,
        frames: dst_frames as u64,
    }
}

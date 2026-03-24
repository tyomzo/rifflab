use super::{AudioBackend, AudioCallback, BackendError, BackendType};
use rifflab_core::audio::AudioConfig;
use std::sync::{Arc, Mutex};

/// ALSA/PipeWire audio backend via cpal.
///
/// cpal::Stream is !Send, but AudioBackend requires Send. The streams are
/// created and used on the same thread (the main thread), so this is safe.
pub struct AlsaBackend {
    sample_rate: Option<u32>,
    buffer_size: Option<usize>,
    output_stream: Option<SendStream>,
    input_stream: Option<SendStream>,
}

/// Wrapper to make cpal::Stream Send. Safe because we only access it
/// from the thread that created it (start/stop are called from main thread).
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}

impl AlsaBackend {
    pub fn new() -> Result<Self, BackendError> {
        use cpal::traits::HostTrait;
        let host = cpal::default_host();
        let _output = host
            .default_output_device()
            .ok_or_else(|| BackendError::Alsa("No default output device".into()))?;

        Ok(Self {
            sample_rate: None,
            buffer_size: None,
            output_stream: None,
            input_stream: None,
        })
    }
}

impl AudioBackend for AlsaBackend {
    fn start(
        &mut self,
        config: &AudioConfig,
        callback: AudioCallback,
    ) -> Result<(), BackendError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();

        let output_device = host
            .default_output_device()
            .ok_or_else(|| BackendError::Alsa("No default output device".into()))?;
        let input_device = host.default_input_device(); // Optional

        // Get output config
        let output_supported = output_device
            .default_output_config()
            .map_err(|e| BackendError::Alsa(format!("Output config error: {e}")))?;

        let out_sample_rate = output_supported.sample_rate().0;
        let out_channels = output_supported.channels() as usize;

        self.sample_rate = Some(out_sample_rate);
        self.buffer_size = Some(config.buffer_size.as_usize());

        log::info!(
            "cpal output: {} ({}ch @ {}Hz)",
            output_device.name().unwrap_or_default(),
            out_channels,
            out_sample_rate,
        );

        // Shared callback wrapped in Arc<Mutex>
        let callback = Arc::new(Mutex::new(callback));

        // Input ring buffer: input callback pushes, output callback reads
        let (mut input_tx, mut input_rx) = rifflab_core::rtrb::RingBuffer::<f32>::new(out_sample_rate as usize);

        // --- Output stream ---
        let cb_out = Arc::clone(&callback);
        let mut output_config: cpal::StreamConfig = output_supported.clone().into();
        output_config.buffer_size = cpal::BufferSize::Fixed(config.buffer_size.as_usize() as u32);

        let output_stream = output_device
            .build_output_stream(
                &output_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / out_channels.max(1);

                    // Read input samples from ring buffer (interleaved stereo)
                    let mut input_buf = vec![0.0f32; frames * 2];
                    for i in 0..frames * 2 {
                        input_buf[i] = input_rx.pop().unwrap_or(0.0);
                    }

                    // Prepare output buffer (interleaved stereo)
                    let mut output_buf = vec![0.0f32; frames * 2];

                    // Call the audio processing callback
                    if let Ok(mut cb) = cb_out.lock() {
                        cb(&input_buf, &mut output_buf, frames);
                    }

                    // Write to cpal output (may have different channel count)
                    for frame in 0..frames {
                        let l = output_buf[frame * 2];
                        let r = output_buf[frame * 2 + 1];
                        for ch in 0..out_channels {
                            let idx = frame * out_channels + ch;
                            if idx < data.len() {
                                data[idx] = if ch % 2 == 0 { l } else { r };
                            }
                        }
                    }
                },
                |err| log::error!("Output stream error: {err}"),
                None,
            )
            .map_err(|e| BackendError::Alsa(format!("Failed to build output stream: {e}")))?;

        output_stream
            .play()
            .map_err(|e| BackendError::Alsa(format!("Failed to start output: {e}")))?;

        self.output_stream = Some(SendStream(output_stream));

        // --- Input stream (optional) ---
        if let Some(input_device) = input_device {
            let input_supported = input_device.default_input_config();
            if let Ok(input_supported) = input_supported {
                let in_channels = input_supported.channels() as usize;
                let mut input_config: cpal::StreamConfig = input_supported.into();
                input_config.buffer_size = cpal::BufferSize::Fixed(config.buffer_size.as_usize() as u32);

                log::info!(
                    "cpal input: {} ({}ch)",
                    input_device.name().unwrap_or_default(),
                    in_channels,
                );

                let input_stream = input_device
                    .build_input_stream(
                        &input_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            // Convert to interleaved stereo and push to ring buffer
                            let frames = data.len() / in_channels.max(1);
                            for frame in 0..frames {
                                let sample = data[frame * in_channels]; // Take first channel
                                let _ = input_tx.push(sample);         // L
                                let _ = input_tx.push(sample);         // R (mono dup)
                            }
                        },
                        |err| log::error!("Input stream error: {err}"),
                        None,
                    )
                    .map_err(|e| BackendError::Alsa(format!("Failed to build input stream: {e}")))?;

                input_stream
                    .play()
                    .map_err(|e| BackendError::Alsa(format!("Failed to start input: {e}")))?;

                self.input_stream = Some(SendStream(input_stream));
            }
        }

        log::info!("cpal backend started: {}Hz", out_sample_rate);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.output_stream = None;
        self.input_stream = None;
        log::info!("cpal backend stopped");
        Ok(())
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Alsa
    }

    fn actual_sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    fn actual_buffer_size(&self) -> Option<usize> {
        self.buffer_size
    }
}

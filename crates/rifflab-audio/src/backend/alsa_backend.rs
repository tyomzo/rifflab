use super::{AudioBackend, AudioCallback, BackendError, BackendType};
use rifflab_core::audio::AudioConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// ALSA/PipeWire audio backend via cpal.
pub struct AlsaBackend {
    sample_rate: Option<u32>,
    buffer_size: Option<usize>,
    actual_buf_size: Arc<AtomicUsize>,
    output_stream: Option<SendStream>,
    input_stream: Option<SendStream>,
    output_device_name: String,
    input_device_name: String,
}

/// Wrapper to make cpal::Stream Send.
#[allow(dead_code)]
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}

impl AlsaBackend {
    pub fn new() -> Result<Self, BackendError> {
        Self::with_devices("", "")
    }

    pub fn with_devices(output_device_name: &str, input_device_name: &str) -> Result<Self, BackendError> {
        use cpal::traits::HostTrait;
        let host = cpal::default_host();
        let _output = host
            .default_output_device()
            .ok_or_else(|| BackendError::Alsa("No default output device".into()))?;
        Ok(Self {
            sample_rate: None,
            buffer_size: None,
            actual_buf_size: Arc::new(AtomicUsize::new(0)),
            output_stream: None,
            input_stream: None,
            output_device_name: output_device_name.to_string(),
            input_device_name: input_device_name.to_string(),
        })
    }

    fn find_output_device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
        use cpal::traits::{DeviceTrait, HostTrait};
        if !name.is_empty() {
            if let Ok(devices) = host.output_devices() {
                for dev in devices {
                    if dev.name().ok().as_deref() == Some(name) {
                        return Some(dev);
                    }
                }
            }
            log::warn!("Output device '{}' not found, using default", name);
        }
        host.default_output_device()
    }

    fn find_input_device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
        use cpal::traits::{DeviceTrait, HostTrait};
        if !name.is_empty() {
            if let Ok(devices) = host.input_devices() {
                for dev in devices {
                    if dev.name().ok().as_deref() == Some(name) {
                        return Some(dev);
                    }
                }
            }
            log::warn!("Input device '{}' not found, using default", name);
        }
        host.default_input_device()
    }
}

impl AudioBackend for AlsaBackend {
    fn start(
        &mut self,
        config: &AudioConfig,
        callback: AudioCallback,
    ) -> Result<(), BackendError> {
        use cpal::traits::{DeviceTrait, StreamTrait};

        let requested_rate = config.sample_rate.as_u32();
        let desired = config.buffer_size.as_usize().max(64);

        // On PipeWire, set default source/sink before cpal opens them.
        if !self.output_device_name.is_empty() {
            set_pw_default("sink", &self.output_device_name);
        }
        if !self.input_device_name.is_empty() && self.input_device_name != "(none)" {
            set_pw_default("source", &self.input_device_name);
        }

        let host = cpal::default_host();

        let output_device = Self::find_output_device(&host, "")
            .ok_or_else(|| BackendError::Alsa("No output device available".into()))?;
        let input_device = if self.input_device_name == "(none)" {
            None
        } else {
            Self::find_input_device(&host, "")
        };

        let output_supported = output_device
            .default_output_config()
            .map_err(|e| BackendError::Alsa(format!("Output config error: {e}")))?;
        let out_channels = output_supported.channels() as usize;

        // Try to use the requested rate. If the device doesn't support it,
        // fall back to the device default. PipeWire's ALSA layer only exposes
        // rates the hardware actually supports at the current graph rate.
        let out_sample_rate = {
            let requested_sr = cpal::SampleRate(requested_rate);
            let supported = output_device.supported_output_configs()
                .map(|cfgs| cfgs.into_iter().any(|r| r.min_sample_rate() <= requested_sr && requested_sr <= r.max_sample_rate()))
                .unwrap_or(false);
            if supported {
                requested_rate
            } else {
                let fallback = output_supported.sample_rate().0;
                log::warn!("Requested {}Hz not supported by device, using {}Hz", requested_rate, fallback);
                fallback
            }
        };

        self.sample_rate = Some(out_sample_rate);
        self.buffer_size = None;

        log::info!("cpal output: {} ({}ch @ {}Hz)",
            output_device.name().unwrap_or_default(), out_channels, out_sample_rate);

        // Input ring buffer: input callback pushes, output callback reads
        let (mut input_tx, mut input_rx) = rifflab_core::rtrb::RingBuffer::<f32>::new(out_sample_rate as usize);

        // Set PipeWire quantum to match desired buffer size.
        // Don't force rate — let PipeWire handle rate conversion.
        let pw_quantum = query_pipewire_quantum();
        if pw_quantum > 0 {
            set_pipewire_quantum(desired);
        }

        // Callback owns the processing directly (no Arc<Mutex>)
        let mut callback = callback;
        let mut output_config: cpal::StreamConfig = output_supported.clone().into();
        output_config.sample_rate = cpal::SampleRate(out_sample_rate);
        output_config.buffer_size = cpal::BufferSize::Fixed(desired as u32);

        log::info!("cpal stream config: {}ch, {}Hz, buffer=Fixed({})",
            output_config.channels, out_sample_rate, desired);

        // Pre-allocate RT buffers
        let max_frames = desired.max(8192);
        let mut input_buf = vec![0.0f32; max_frames * 2];
        let mut output_buf = vec![0.0f32; max_frames * 2];
        let buf_size_report = Arc::clone(&self.actual_buf_size);

        let output_stream = output_device
            .build_output_stream(
                &output_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / out_channels.max(1);
                    let stereo_samples = frames * 2;
                    buf_size_report.store(frames, Ordering::Relaxed);

                    // Always drain the ring buffer to keep input/output streams in sync.
                    // Read what's available, crossfade to silence for the rest.
                    let available = input_rx.slots();
                    let to_read = available.min(stereo_samples);
                    for i in 0..to_read {
                        input_buf[i] = input_rx.pop().unwrap_or(0.0);
                    }
                    if to_read < stereo_samples {
                        // Underflow: crossfade last valid sample to zero over remaining frames
                        let last_l = if to_read >= 2 { input_buf[to_read - 2] } else { 0.0 };
                        let last_r = if to_read >= 1 { input_buf[to_read - 1] } else { 0.0 };
                        let remaining = stereo_samples - to_read;
                        let remaining_frames = remaining / 2;
                        for f in 0..remaining_frames {
                            let t = 1.0 - (f + 1) as f32 / remaining_frames as f32;
                            input_buf[to_read + f * 2] = last_l * t;
                            input_buf[to_read + f * 2 + 1] = last_r * t;
                        }
                    } else if available > stereo_samples * 3 {
                        // Overflow: input is piling up — drain excess to keep streams in sync.
                        // Keep one buffer ahead for jitter headroom.
                        let excess = available - stereo_samples * 2;
                        for _ in 0..excess {
                            let _ = input_rx.pop();
                        }
                    }

                    // Zero output
                    for s in &mut output_buf[..stereo_samples] {
                        *s = 0.0;
                    }

                    // Process audio (no mutex, direct call)
                    callback(&input_buf[..stereo_samples], &mut output_buf[..stereo_samples], frames);

                    // Write to hardware
                    if out_channels == 2 {
                        data[..stereo_samples].copy_from_slice(&output_buf[..stereo_samples]);
                    } else {
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
                    }
                },
                |err| log::error!("Output stream error: {err}"),
                None,
            )
            .map_err(|e| BackendError::Alsa(format!("Failed to build output stream: {e}")))?;

        output_stream.play()
            .map_err(|e| BackendError::Alsa(format!("Failed to start output: {e}")))?;
        self.output_stream = Some(SendStream(output_stream));

        // --- Input stream (optional) ---
        if let Some(input_device) = input_device {
            if let Ok(input_supported) = input_device.default_input_config() {
                let in_channels = input_supported.channels() as usize;
                let mut input_config: cpal::StreamConfig = input_supported.into();
                input_config.sample_rate = cpal::SampleRate(out_sample_rate);
                input_config.buffer_size = cpal::BufferSize::Fixed(desired as u32);

                log::info!("cpal input: {} ({}ch)", input_device.name().unwrap_or_default(), in_channels);

                let input_stream = input_device
                    .build_input_stream(
                        &input_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let frames = data.len() / in_channels.max(1);
                            for frame in 0..frames {
                                let sample = data[frame * in_channels];
                                let _ = input_tx.push(sample); // L
                                let _ = input_tx.push(sample); // R (mono dup)
                            }
                        },
                        |err| log::error!("Input stream error: {err}"),
                        None,
                    )
                    .map_err(|e| BackendError::Alsa(format!("Failed to build input stream: {e}")))?;

                input_stream.play()
                    .map_err(|e| BackendError::Alsa(format!("Failed to start input: {e}")))?;
                self.input_stream = Some(SendStream(input_stream));
            }
        }

        log::info!("cpal backend started: {}Hz, buffer {}", out_sample_rate, desired);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.output_stream = None;
        self.input_stream = None;
        // Reset PipeWire quantum to server default
        set_pipewire_quantum(0);
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
        let reported = self.actual_buf_size.load(Ordering::Relaxed);
        if reported > 0 { Some(reported) } else { self.buffer_size }
    }
}

// ─── PipeWire helpers ────────────────────────────────────────────────────────

/// Set PipeWire default source or sink via pactl.
fn set_pw_default(kind: &str, name: &str) {
    let cmd = if kind == "source" { "set-default-source" } else { "set-default-sink" };
    let pw_name = resolve_pw_name(name, kind).unwrap_or_else(|| name.to_string());
    log::info!("Setting default {}: {}", kind, pw_name);
    let _ = std::process::Command::new("pactl").args([cmd, &pw_name]).status();
}

/// Resolve a device description to its PipeWire name.
fn resolve_pw_name(description: &str, kind: &str) -> Option<String> {
    let list_kind = if kind == "source" { "sources" } else { "sinks" };
    let output = std::process::Command::new("pactl").args(["list", list_kind]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut current_name: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(n) = trimmed.strip_prefix("Name: ") {
            current_name = Some(n.to_string());
        }
        if let Some(d) = trimmed.strip_prefix("Description: ") {
            if d == description { return current_name; }
        }
    }
    None
}

/// Query PipeWire's current quantum via pw-metadata.
fn query_pipewire_quantum() -> usize {
    let output = match std::process::Command::new("pw-metadata")
        .args(["-n", "settings", "0"]).output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return 0,
    };
    let mut quantum: usize = 0;
    let mut force_quantum: usize = 0;
    for line in output.lines() {
        if line.contains("clock.quantum") && !line.contains("force") && !line.contains("min") && !line.contains("max") && !line.contains("limit") && !line.contains("floor") {
            if let Some(val) = extract_pw_value(line) { quantum = val; }
        } else if line.contains("clock.force-quantum") {
            if let Some(val) = extract_pw_value(line) { force_quantum = val; }
        }
    }
    let effective = quantum.max(force_quantum);
    if effective > 0 {
        log::info!("PipeWire quantum: {} (force: {}), using: {}", quantum, force_quantum, effective);
    }
    effective
}

/// Set PipeWire's quantum via pw-metadata.
fn set_pipewire_quantum(quantum: usize) {
    log::info!("Setting PipeWire quantum to {}", quantum);
    let _ = std::process::Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.force-quantum", &quantum.to_string()])
        .status();
}



fn extract_pw_value(line: &str) -> Option<usize> {
    line.split("value:'").nth(1)
        .and_then(|s| s.split('\'').next())
        .and_then(|s| s.parse().ok())
}

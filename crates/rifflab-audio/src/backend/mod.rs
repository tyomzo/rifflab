#[cfg(feature = "jack-backend")]
pub mod jack_backend;
#[cfg(feature = "alsa-backend")]
pub mod alsa_backend;

use rifflab_core::audio::AudioConfig;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("JACK error: {0}")]
    Jack(String),
    #[error("ALSA/cpal error: {0}")]
    Alsa(String),
    #[error("No audio backend available")]
    NoBackend,
    #[error("Backend not started")]
    NotStarted,
}

/// Which audio backend is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    Jack,
    Alsa,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jack => write!(f, "JACK"),
            Self::Alsa => write!(f, "cpal (ALSA/PipeWire)"),
        }
    }
}

/// Information about an available audio device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Human-readable device name.
    pub name: String,
    /// Whether this device supports input.
    pub is_input: bool,
    /// Whether this device supports output.
    pub is_output: bool,
    /// Supported sample rates (may be empty if query fails).
    pub sample_rates: Vec<u32>,
    /// Number of channels available.
    pub channels: u16,
    /// Whether this is the system default device.
    pub is_default: bool,
}

/// Enumerate available audio devices.
/// On PipeWire systems, uses `pactl` for proper device names.
/// Falls back to cpal enumeration otherwise.
pub fn enumerate_devices() -> Vec<DeviceInfo> {
    // Try PipeWire/PulseAudio enumeration first (gives real device names)
    let pw_devices = enumerate_pipewire_devices();
    if !pw_devices.is_empty() {
        return pw_devices;
    }

    // Fallback: cpal enumeration
    let mut devices = Vec::new();

    #[cfg(feature = "alsa-backend")]
    {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let default_out_name = host
            .default_output_device()
            .and_then(|d| d.name().ok());
        let default_in_name = host
            .default_input_device()
            .and_then(|d| d.name().ok());

        if let Ok(output_devices) = host.output_devices() {
            for dev in output_devices {
                let name = dev.name().unwrap_or_else(|_| "Unknown".into());
                let is_default = default_out_name.as_deref() == Some(&name);
                let (channels, sample_rates) = query_device_caps(&dev, false);
                devices.push(DeviceInfo {
                    name,
                    is_input: false,
                    is_output: true,
                    sample_rates,
                    channels,
                    is_default,
                });
            }
        }

        if let Ok(input_devices) = host.input_devices() {
            for dev in input_devices {
                let name = dev.name().unwrap_or_else(|_| "Unknown".into());
                let is_default = default_in_name.as_deref() == Some(&name);
                let (channels, sample_rates) = query_device_caps(&dev, true);
                devices.push(DeviceInfo {
                    name,
                    is_input: true,
                    is_output: false,
                    sample_rates,
                    channels,
                    is_default,
                });
            }
        }
    }

    devices
}

/// Enumerate PipeWire/PulseAudio devices using pactl.
/// Returns human-readable names and PipeWire source/sink identifiers.
fn enumerate_pipewire_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();

    // Get default source/sink
    let default_source = run_pactl(&["get-default-source"]);
    let default_sink = run_pactl(&["get-default-sink"]);

    // List sinks (output devices)
    if let Some(output) = run_pactl_opt(&["list", "sinks", "short"]) {
        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let pw_name = parts[1].to_string();
                // Skip monitor sources
                if pw_name.contains(".monitor") {
                    continue;
                }
                let description = get_pw_description(&pw_name, "sink");
                let is_default = default_sink.as_deref() == Some(pw_name.as_str());
                let (channels, rate) = parse_pactl_format(parts.get(3).unwrap_or(&""));
                devices.push(DeviceInfo {
                    name: format_device_label(&description, &pw_name),
                    is_input: false,
                    is_output: true,
                    sample_rates: if rate > 0 { vec![rate] } else { vec![48000] },
                    channels,
                    is_default,
                });
            }
        }
    }

    // List sources (input devices)
    if let Some(output) = run_pactl_opt(&["list", "sources", "short"]) {
        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let pw_name = parts[1].to_string();
                // Skip monitor sources (output loopbacks)
                if pw_name.contains(".monitor") {
                    continue;
                }
                let description = get_pw_description(&pw_name, "source");
                let is_default = default_source.as_deref() == Some(pw_name.as_str());
                let (channels, rate) = parse_pactl_format(parts.get(3).unwrap_or(&""));
                devices.push(DeviceInfo {
                    name: format_device_label(&description, &pw_name),
                    is_input: true,
                    is_output: false,
                    sample_rates: if rate > 0 { vec![rate] } else { vec![48000] },
                    channels,
                    is_default,
                });
            }
        }
    }

    devices
}

fn run_pactl(args: &[&str]) -> Option<String> {
    std::process::Command::new("pactl")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn run_pactl_opt(args: &[&str]) -> Option<String> {
    run_pactl(args)
}

/// Get PipeWire node description via pactl.
fn get_pw_description(pw_name: &str, kind: &str) -> String {
    // Try pw-cli for node.description
    let output = std::process::Command::new("pactl")
        .args(["list", if kind == "sink" { "sinks" } else { "sources" }])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Parse the verbose output to find the description for this device
    let mut in_device = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Name: ") {
            in_device = trimmed.strip_prefix("Name: ") == Some(pw_name);
        }
        if in_device && trimmed.starts_with("Description: ") {
            return trimmed.strip_prefix("Description: ").unwrap_or(pw_name).to_string();
        }
    }
    pw_name.to_string()
}

fn format_device_label(description: &str, pw_name: &str) -> String {
    if description != pw_name && !description.is_empty() {
        description.to_string()
    } else {
        pw_name.to_string()
    }
}

/// Parse pactl format string like "s32le 2ch 48000Hz" or "float32le 1ch 48000Hz"
fn parse_pactl_format(format_str: &str) -> (u16, u32) {
    let mut channels: u16 = 2;
    let mut rate: u32 = 0;
    for part in format_str.split_whitespace() {
        if part.ends_with("ch") {
            channels = part.trim_end_matches("ch").parse().unwrap_or(2);
        } else if part.ends_with("Hz") {
            rate = part.trim_end_matches("Hz").parse().unwrap_or(0);
        }
    }
    (channels, rate)
}

/// Query supported channels and common sample rates for a device.
#[cfg(feature = "alsa-backend")]
fn query_device_caps(dev: &cpal::Device, is_input: bool) -> (u16, Vec<u32>) {
    use cpal::traits::DeviceTrait;

    let config = if is_input {
        dev.default_input_config()
    } else {
        dev.default_output_config()
    };
    match config {
        Ok(cfg) => {
            let channels = cfg.channels();
            let default_rate = cfg.sample_rate().0;
            // Collect supported sample rate ranges
            let common_rates = [44100u32, 48000, 96000, 192000];
            let mut supported = Vec::new();

            // Collect ranges into a common type: Vec<(SampleRate, SampleRate)>
            let ranges: Vec<(cpal::SampleRate, cpal::SampleRate)> = if is_input {
                dev.supported_input_configs()
                    .map(|iter| iter.map(|r| (r.min_sample_rate(), r.max_sample_rate())).collect())
                    .unwrap_or_default()
            } else {
                dev.supported_output_configs()
                    .map(|iter| iter.map(|r| (r.min_sample_rate(), r.max_sample_rate())).collect())
                    .unwrap_or_default()
            };

            for &rate in &common_rates {
                let sr = cpal::SampleRate(rate);
                if ranges.iter().any(|(min, max)| *min <= sr && sr <= *max) {
                    supported.push(rate);
                }
            }

            if supported.is_empty() {
                supported.push(default_rate);
            }
            (channels, supported)
        }
        Err(_) => (2, vec![48000]),
    }
}

/// Available backends that can be used.
pub fn available_backends() -> Vec<BackendType> {
    let mut backends = Vec::new();
    #[cfg(feature = "alsa-backend")]
    backends.push(BackendType::Alsa);
    #[cfg(feature = "jack-backend")]
    backends.push(BackendType::Jack);
    backends
}

/// Trait abstracting over JACK and ALSA backends.
pub trait AudioBackend: Send {
    /// Start the audio backend with the given config.
    /// The callback will be invoked for each audio buffer.
    fn start(
        &mut self,
        config: &AudioConfig,
        callback: AudioCallback,
    ) -> Result<(), BackendError>;

    /// Stop the audio backend.
    fn stop(&mut self) -> Result<(), BackendError>;

    /// The backend type.
    fn backend_type(&self) -> BackendType;

    /// Actual sample rate negotiated with the hardware.
    fn actual_sample_rate(&self) -> Option<u32>;

    /// Actual buffer size negotiated with the hardware.
    fn actual_buffer_size(&self) -> Option<usize>;
}

/// Audio processing callback.
/// Called from the real-time audio thread.
///
/// - `input`: interleaved input samples from hardware
/// - `output`: interleaved output buffer to fill
/// - `frames`: number of frames in this buffer
pub type AudioCallback = Box<
    dyn FnMut(&[f32], &mut [f32], usize) + Send + 'static
>;

/// Create an audio backend with optional device preferences.
pub fn create_backend_with_prefs(
    preferred_backend: &str,
    output_device_name: &str,
    input_device_name: &str,
) -> Result<Box<dyn AudioBackend>, BackendError> {
    match preferred_backend {
        "jack" => {
            #[cfg(feature = "jack-backend")]
            match jack_backend::JackBackend::new() {
                Ok(backend) => {
                    log::info!("Using JACK audio backend (requested)");
                    return Ok(Box::new(backend));
                }
                Err(e) => {
                    log::error!("JACK requested but failed: {e}");
                    return Err(e);
                }
            }
            #[cfg(not(feature = "jack-backend"))]
            return Err(BackendError::NoBackend);
        }
        "alsa" | "cpal" | "pipewire" => {
            #[cfg(feature = "alsa-backend")]
            match alsa_backend::AlsaBackend::with_devices(output_device_name, input_device_name) {
                Ok(backend) => {
                    log::info!("Using cpal audio backend (requested)");
                    return Ok(Box::new(backend));
                }
                Err(e) => {
                    log::error!("cpal requested but failed: {e}");
                    return Err(e);
                }
            }
            #[cfg(not(feature = "alsa-backend"))]
            return Err(BackendError::NoBackend);
        }
        _ => {
            // "auto" or unknown — try cpal first, then JACK
            create_backend_with_prefs("cpal", output_device_name, input_device_name)
                .or_else(|_| create_backend_with_prefs("jack", "", ""))
        }
    }
}

/// Create an audio backend. Tries cpal (ALSA/PipeWire) first, JACK as fallback.
pub fn create_backend() -> Result<Box<dyn AudioBackend>, BackendError> {
    create_backend_with_prefs("auto", "", "")
}

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

/// Try to create a JACK backend, falling back to ALSA.
pub fn create_backend() -> Result<Box<dyn AudioBackend>, BackendError> {
    // Try JACK first
    #[cfg(feature = "jack-backend")]
    match jack_backend::JackBackend::new() {
        Ok(backend) => {
            log::info!("Using JACK audio backend");
            return Ok(Box::new(backend));
        }
        Err(e) => {
            log::warn!("JACK not available ({e}), falling back to ALSA");
        }
    }

    #[cfg(feature = "alsa-backend")]
    match alsa_backend::AlsaBackend::new() {
        Ok(backend) => {
            log::info!("Using ALSA audio backend via cpal");
            return Ok(Box::new(backend));
        }
        Err(e) => {
            log::error!("ALSA also failed: {e}");
        }
    }

    Err(BackendError::NoBackend)
}

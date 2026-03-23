use super::{AudioBackend, AudioCallback, BackendError, BackendType};
use rifflab_core::audio::AudioConfig;

/// ALSA audio backend via cpal.
pub struct AlsaBackend {
    // TODO: cpal Host, Device, Stream handles
    sample_rate: Option<u32>,
    buffer_size: Option<usize>,
}

impl AlsaBackend {
    pub fn new() -> Result<Self, BackendError> {
        // Verify cpal can find a default device
        use cpal::traits::HostTrait;
        let host = cpal::default_host();
        let _output = host
            .default_output_device()
            .ok_or_else(|| BackendError::Alsa("No default output device".into()))?;

        Ok(Self {
            sample_rate: None,
            buffer_size: None,
        })
    }
}

impl AudioBackend for AlsaBackend {
    fn start(
        &mut self,
        config: &AudioConfig,
        _callback: AudioCallback,
    ) -> Result<(), BackendError> {
        self.sample_rate = Some(config.sample_rate.as_u32());
        self.buffer_size = Some(config.buffer_size.as_usize());
        // TODO: create cpal streams with the callback
        log::info!("ALSA backend started (stub)");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        log::info!("ALSA backend stopped (stub)");
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

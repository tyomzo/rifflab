use super::{AudioBackend, AudioCallback, BackendError, BackendType};
use rifflab_core::audio::AudioConfig;
use std::sync::{Arc, Mutex};

/// JACK audio backend.
pub struct JackBackend {
    client_name: String,
    active_client: Option<jack::AsyncClient<Notifications, JackProcess>>,
    sample_rate: Option<u32>,
    buffer_size: Option<usize>,
}

struct Notifications;

impl jack::NotificationHandler for Notifications {
    fn xrun(&mut self, _: &jack::Client) -> jack::Control {
        log::warn!("JACK xrun detected");
        jack::Control::Continue
    }

    fn sample_rate(&mut self, _: &jack::Client, srate: jack::Frames) -> jack::Control {
        log::info!("JACK sample rate changed to {srate}");
        jack::Control::Continue
    }
}

struct JackProcess {
    input_port: jack::Port<jack::AudioIn>,
    output_port_l: jack::Port<jack::AudioOut>,
    output_port_r: jack::Port<jack::AudioOut>,
    callback: Arc<Mutex<AudioCallback>>,
}

impl jack::ProcessHandler for JackProcess {
    fn process(&mut self, _: &jack::Client, ps: &jack::ProcessScope) -> jack::Control {
        let input = self.input_port.as_slice(ps);
        let out_l = self.output_port_l.as_mut_slice(ps);
        let out_r = self.output_port_r.as_mut_slice(ps);
        let frames = out_l.len();

        // Create interleaved input (mono input duplicated to stereo)
        let mut input_interleaved = vec![0.0f32; frames * 2];
        for (i, &s) in input.iter().enumerate().take(frames) {
            input_interleaved[i * 2] = s;
            input_interleaved[i * 2 + 1] = s;
        }

        // Output buffer (interleaved stereo)
        let mut output_interleaved = vec![0.0f32; frames * 2];

        // Call the audio processing callback
        if let Ok(mut cb) = self.callback.lock() {
            cb(&input_interleaved, &mut output_interleaved, frames);
        }

        // De-interleave to JACK ports
        for i in 0..frames {
            out_l[i] = output_interleaved[i * 2];
            out_r[i] = output_interleaved[i * 2 + 1];
        }

        jack::Control::Continue
    }
}

impl JackBackend {
    pub fn new() -> Result<Self, BackendError> {
        // Just verify JACK is available by attempting to create a client
        let (client, _) = jack::Client::new("rifflab_probe", jack::ClientOptions::NO_START_SERVER)
            .map_err(|e| BackendError::Jack(e.to_string()))?;
        let sr = client.sample_rate() as u32;
        let bs = client.buffer_size() as usize;
        // Drop the probe client
        drop(client);
        Ok(Self {
            client_name: "RiffLab".to_string(),
            active_client: None,
            sample_rate: Some(sr),
            buffer_size: Some(bs),
        })
    }
}

impl AudioBackend for JackBackend {
    fn start(
        &mut self,
        _config: &AudioConfig,
        callback: AudioCallback,
    ) -> Result<(), BackendError> {
        let (client, _) = jack::Client::new(&self.client_name, jack::ClientOptions::NO_START_SERVER)
            .map_err(|e| BackendError::Jack(e.to_string()))?;

        self.sample_rate = Some(client.sample_rate() as u32);
        self.buffer_size = Some(client.buffer_size() as usize);

        let input_port = client
            .register_port("input", jack::AudioIn::default())
            .map_err(|e| BackendError::Jack(e.to_string()))?;
        let output_port_l = client
            .register_port("output_L", jack::AudioOut::default())
            .map_err(|e| BackendError::Jack(e.to_string()))?;
        let output_port_r = client
            .register_port("output_R", jack::AudioOut::default())
            .map_err(|e| BackendError::Jack(e.to_string()))?;

        // Get port names before moving into process handler
        let out_l_name = output_port_l.name().unwrap_or_default().to_string();
        let out_r_name = output_port_r.name().unwrap_or_default().to_string();
        let in_name = input_port.name().unwrap_or_default().to_string();

        let process = JackProcess {
            input_port,
            output_port_l,
            output_port_r,
            callback: Arc::new(Mutex::new(callback)),
        };

        let active = client
            .activate_async(Notifications, process)
            .map_err(|e| BackendError::Jack(e.to_string()))?;

        // Auto-connect output ports to system playback
        let client_ref = active.as_client();
        let playback_ports = client_ref.ports(
            Some("system:playback_.*"),
            None,
            jack::PortFlags::IS_INPUT,
        );
        if playback_ports.len() >= 2 {
            let _ = client_ref.connect_ports_by_name(&out_l_name, &playback_ports[0]);
            let _ = client_ref.connect_ports_by_name(&out_r_name, &playback_ports[1]);
            log::info!("Connected output to {} and {}", playback_ports[0], playback_ports[1]);
        } else if playback_ports.len() == 1 {
            let _ = client_ref.connect_ports_by_name(&out_l_name, &playback_ports[0]);
            let _ = client_ref.connect_ports_by_name(&out_r_name, &playback_ports[0]);
            log::info!("Connected output (mono) to {}", playback_ports[0]);
        } else {
            log::warn!("No system playback ports found — output not connected");
        }

        // Auto-connect system capture to our input
        let capture_ports = client_ref.ports(
            Some("system:capture_.*"),
            None,
            jack::PortFlags::IS_OUTPUT,
        );
        if let Some(capture) = capture_ports.first() {
            let _ = client_ref.connect_ports_by_name(capture, &in_name);
            log::info!("Connected input from {}", capture);
        }

        self.active_client = Some(active);
        log::info!(
            "JACK backend started: {}Hz, {} frames",
            self.sample_rate.unwrap_or(0),
            self.buffer_size.unwrap_or(0)
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        if let Some(client) = self.active_client.take() {
            client
                .deactivate()
                .map_err(|e| BackendError::Jack(e.to_string()))?;
            log::info!("JACK backend stopped");
        }
        Ok(())
    }

    fn backend_type(&self) -> BackendType {
        BackendType::Jack
    }

    fn actual_sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    fn actual_buffer_size(&self) -> Option<usize> {
        self.buffer_size
    }
}

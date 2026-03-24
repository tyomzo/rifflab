use crate::backend::{self, AudioBackend, BackendError};
use crate::graph::AudioGraph;
use crate::transport::Transport;
use rifflab_core::analysis::PitchFrame;
use rifflab_core::audio::{AudioConfig, ProcessContext};
use rifflab_core::metering::MeterData;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Backend error: {0}")]
    Backend(#[from] BackendError),
    #[error("Engine already running")]
    AlreadyRunning,
}

/// The main audio engine. Owns the backend, graph, and transport.
pub struct AudioEngine {
    backend: Option<Box<dyn AudioBackend>>,
    graph: Arc<Mutex<AudioGraph>>,
    transport: Transport,
    config: AudioConfig,
    meter_tx: rifflab_core::rtrb::Producer<MeterData>,
    pitch_tx: rifflab_core::rtrb::Producer<PitchFrame>,
    running: bool,
}

impl AudioEngine {
    pub fn new(
        config: AudioConfig,
    ) -> (
        Self,
        rifflab_core::rtrb::Consumer<MeterData>,
        rifflab_core::rtrb::Consumer<PitchFrame>,
    ) {
        let (meter_tx, meter_rx) = rifflab_core::rtrb::RingBuffer::new(1024);
        let (pitch_tx, pitch_rx) = rifflab_core::rtrb::RingBuffer::new(1024);
        let mut graph = AudioGraph::new(config.buffer_size.as_usize());
        graph.enable_pitch_detection(config.sample_rate.as_u32());
        let engine = Self {
            backend: None,
            graph: Arc::new(Mutex::new(graph)),
            transport: Transport::new(config.sample_rate.as_u32()),
            config,
            meter_tx,
            pitch_tx,
            running: false,
        };
        (engine, meter_rx, pitch_rx)
    }

    /// Get a mutable reference to the graph for loading stems, setting FX, etc.
    pub fn graph(&self) -> &Arc<Mutex<AudioGraph>> {
        &self.graph
    }

    /// Get a mutable reference to the transport for UI control.
    pub fn transport_mut(&mut self) -> &mut Transport {
        &mut self.transport
    }

    /// Get the transport for reading state.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// Start the audio engine.
    pub fn start(&mut self) -> Result<(), EngineError> {
        if self.running {
            return Err(EngineError::AlreadyRunning);
        }

        let mut backend = backend::create_backend()?;

        // Set up the audio callback
        let graph = Arc::clone(&self.graph);
        let rt_handle = self.transport.rt_handle();
        let mut command_rx = self.transport.take_command_rx().expect("command_rx already taken");
        let sample_rate = self.config.sample_rate.as_u32();
        let mut meter_tx = {
            // We need to move the producer into the callback.
            // Create a new ring buffer pair and swap.
            let (new_tx, _) = rifflab_core::rtrb::RingBuffer::new(1);
            std::mem::replace(&mut self.meter_tx, new_tx)
        };
        let mut pitch_tx = {
            let (new_tx, _) = rifflab_core::rtrb::RingBuffer::new(1);
            std::mem::replace(&mut self.pitch_tx, new_tx)
        };

        let callback: backend::AudioCallback = Box::new(move |input, output, frames| {
            let is_playing = rt_handle.advance(frames, &mut command_rx);

            let context = ProcessContext {
                sample_rate,
                buffer_size: frames,
                transport_position: rt_handle.position_frames(),
                is_playing,
                bpm: None,
            };

            // Zero output
            for s in output.iter_mut() {
                *s = 0.0;
            }

            if let Ok(mut g) = graph.lock() {
                let (meter, pitch_frame) = g.process(input, output, frames, &context);
                let _ = meter_tx.push(meter);
                if let Some(pitch) = pitch_frame {
                    let _ = pitch_tx.push(pitch);
                }
            }
        });

        backend.start(&self.config, callback)?;
        self.backend = Some(backend);
        self.running = true;

        log::info!("Audio engine started");
        Ok(())
    }

    /// Stop the audio engine.
    pub fn stop(&mut self) -> Result<(), EngineError> {
        if let Some(mut backend) = self.backend.take() {
            backend.stop()?;
        }
        self.running = false;
        log::info!("Audio engine stopped");
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

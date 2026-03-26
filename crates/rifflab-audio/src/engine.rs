use crate::analysis_thread::AnalysisThread;
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
    /// Preferred backend: "auto", "jack", "alsa", "cpal", "pipewire"
    preferred_backend: String,
    /// Preferred output device name (empty = default).
    output_device_name: String,
    /// Preferred input device name (empty = default).
    input_device_name: String,
    /// Analysis thread for pitch detection (off the audio thread).
    analysis_thread: Option<AnalysisThread>,
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
        let graph = AudioGraph::new(config.buffer_size.as_usize());
        let engine = Self {
            backend: None,
            graph: Arc::new(Mutex::new(graph)),
            transport: Transport::new(config.sample_rate.as_u32()),
            config,
            meter_tx,
            pitch_tx,
            running: false,
            preferred_backend: "auto".to_string(),
            output_device_name: String::new(),
            input_device_name: String::new(),
            analysis_thread: None,
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

    /// Get a reference to the active backend (if running).
    pub fn backend_ref(&self) -> Option<&dyn backend::AudioBackend> {
        self.backend.as_deref()
    }

    /// Start the audio engine.
    pub fn start(&mut self) -> Result<(), EngineError> {
        if self.running {
            return Err(EngineError::AlreadyRunning);
        }

        let mut backend = backend::create_backend_with_prefs(
            &self.preferred_backend,
            &self.output_device_name,
            &self.input_device_name,
        )?;

        // Use actual backend sample rate (hardware may differ from config)
        let sample_rate = backend.actual_sample_rate()
            .unwrap_or_else(|| self.config.sample_rate.as_u32());

        // Start the analysis thread — it owns pitch_tx and receives mono audio
        let pitch_tx = {
            let (new_tx, _) = rifflab_core::rtrb::RingBuffer::new(1);
            std::mem::replace(&mut self.pitch_tx, new_tx)
        };
        let (analysis, mut audio_tx) = AnalysisThread::new(sample_rate, pitch_tx);
        self.analysis_thread = Some(analysis);

        // Set up the audio callback
        let graph = Arc::clone(&self.graph);
        let rt_handle = self.transport.rt_handle();
        let mut command_rx = self.transport.take_command_rx().expect("command_rx already taken");
        let mut meter_tx = {
            let (new_tx, _) = rifflab_core::rtrb::RingBuffer::new(1);
            std::mem::replace(&mut self.meter_tx, new_tx)
        };

        // Latency measurement state
        let mut latency_input_time: Option<std::time::Instant> = None;
        let mut latency_measured = false;
        let latency_threshold = 0.05f32; // detect signal above this

        let callback: backend::AudioCallback = Box::new(move |input, output, frames| {
            // 1. Process commands (play/pause/seek) BEFORE reading position
            let is_playing = rt_handle.process_commands(&mut command_rx);

            // 2. Read position for THIS buffer (not yet advanced)
            let pos = rt_handle.position_frames();
            let context = ProcessContext {
                sample_rate,
                buffer_size: frames,
                transport_position: pos,
                is_playing,
                bpm: None,
            };

            // 3. Zero output
            for s in output.iter_mut() {
                *s = 0.0;
            }

            // 4. Push mono input to analysis thread (lock-free, zero-alloc)
            let stereo_len = (frames * 2).min(input.len());
            let mono_frames = stereo_len / 2;
            for i in 0..mono_frames {
                let mono = (input[i * 2] + input[i * 2 + 1]) * 0.5;
                let _ = audio_tx.push(mono); // drop on overflow
            }

            // 5. Process audio graph (stems + effects + mix + meter — no pitch detection)
            if let Ok(mut g) = graph.try_lock() {
                let meter = g.process(input, output, frames, &context);
                let _ = meter_tx.push(meter);
            }

            // 6. Advance position AFTER processing for next buffer
            rt_handle.advance(frames);

            // 7. Latency measurement: detect when input signal appears, then when it hits output
            if !latency_measured {
                let input_peak: f32 = input.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                let output_peak: f32 = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

                if latency_input_time.is_none() && input_peak > latency_threshold {
                    latency_input_time = Some(std::time::Instant::now());
                    log::info!("[latency] Input signal detected (peak={:.3}), frames={}", input_peak, frames);
                }

                if let Some(t) = latency_input_time {
                    if output_peak > latency_threshold {
                        let elapsed = t.elapsed();
                        log::info!(
                            "[latency] Input->Output: {:.1}ms (frames={}, {}Hz)",
                            elapsed.as_secs_f64() * 1000.0,
                            frames,
                            sample_rate,
                        );
                        latency_measured = true;
                    }
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
        // Stop analysis thread first
        if let Some(mut at) = self.analysis_thread.take() {
            at.stop();
        }
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

    /// Get the current audio config.
    pub fn config(&self) -> &AudioConfig {
        &self.config
    }

    /// Update the analysis thread's noise floor threshold.
    pub fn set_analysis_noise_floor(&self, rms: f32) {
        if let Some(ref at) = self.analysis_thread {
            at.set_noise_floor(rms);
        }
    }

    /// Set backend preferences. Takes effect on next start().
    pub fn set_backend_prefs(
        &mut self,
        preferred_backend: &str,
        output_device: &str,
        input_device: &str,
    ) {
        self.preferred_backend = preferred_backend.to_string();
        self.output_device_name = output_device.to_string();
        self.input_device_name = input_device.to_string();
    }

    /// Update the audio config. Requires stop + start to take effect.
    pub fn set_config(&mut self, config: AudioConfig) {
        self.config = config;
    }

    /// Restart the engine with current config and preferences.
    /// Returns the new ring buffer consumers (old ones become invalid).
    pub fn restart(
        &mut self,
    ) -> Result<
        (
            rifflab_core::rtrb::Consumer<MeterData>,
            rifflab_core::rtrb::Consumer<PitchFrame>,
        ),
        EngineError,
    > {
        let _ = self.stop();

        // Recreate ring buffers
        let (meter_tx, meter_rx) = rifflab_core::rtrb::RingBuffer::new(1024);
        let (pitch_tx, pitch_rx) = rifflab_core::rtrb::RingBuffer::new(1024);
        self.meter_tx = meter_tx;
        self.pitch_tx = pitch_tx;

        // Rebuild graph with new buffer size
        let mut new_graph = AudioGraph::new(self.config.buffer_size.as_usize());

        // Transfer stems from old graph
        {
            let mut old_graph = self.graph.lock().unwrap();
            std::mem::swap(&mut new_graph.stem_players, &mut old_graph.stem_players);
            std::mem::swap(&mut new_graph.stem_volumes, &mut old_graph.stem_volumes);
            std::mem::swap(&mut new_graph.stem_mutes, &mut old_graph.stem_mutes);
            std::mem::swap(&mut new_graph.stem_solos, &mut old_graph.stem_solos);
            new_graph.master_volume = old_graph.master_volume;
            *old_graph = new_graph;
        }

        // Update transport sample rate
        self.transport = Transport::new(self.config.sample_rate.as_u32());

        self.start()?;
        Ok((meter_rx, pitch_rx))
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

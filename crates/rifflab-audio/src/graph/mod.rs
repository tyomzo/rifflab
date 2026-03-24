pub mod node;
pub mod bus;

use rifflab_core::audio::{AudioProcessor, ProcessContext};
use rifflab_core::metering::MeterData;

/// Fixed-topology audio graph.
///
/// ```text
/// StemPlayers[0..N] ──► stem buses ──► Mixer ──► master bus ──► Output
///                                         ▲
/// Input ──► FxInsert (effect chain) ──► fx_return bus
/// ```
pub struct AudioGraph {
    /// Stem player nodes (one per loaded stem).
    pub stem_players: Vec<node::StemPlayer>,
    /// Per-stem volume (0.0–1.0). Index matches stem_players.
    pub stem_volumes: Vec<f32>,
    /// Per-stem mute flags.
    pub stem_mutes: Vec<bool>,
    /// Per-stem solo flags.
    pub stem_solos: Vec<bool>,
    /// Effect chain applied to the live input.
    pub fx_chain: Option<Box<dyn AudioProcessor>>,
    /// Input gain.
    pub input_volume: f32,
    /// Master volume.
    pub master_volume: f32,
    /// Internal mix buffer (reused across calls to avoid allocation).
    mix_buffer: Vec<f32>,
    /// Input processing buffer.
    input_buffer: Vec<f32>,
}

impl AudioGraph {
    pub fn new(max_buffer_size: usize) -> Self {
        // Use a generous buffer size — JACK may use larger buffers than configured.
        // 8192 covers up to 8192-frame JACK buffers.
        let buf_size = max_buffer_size.max(8192);
        Self {
            stem_players: Vec::new(),
            stem_volumes: Vec::new(),
            stem_mutes: Vec::new(),
            stem_solos: Vec::new(),
            fx_chain: None,
            input_volume: 1.0,
            master_volume: 1.0,
            mix_buffer: vec![0.0; buf_size * 2], // stereo
            input_buffer: vec![0.0; buf_size * 2],
        }
    }

    /// Load stems into the graph.
    pub fn load_stems(&mut self, players: Vec<node::StemPlayer>) {
        let count = players.len();
        self.stem_players = players;
        self.stem_volumes = vec![1.0; count];
        self.stem_mutes = vec![false; count];
        self.stem_solos = vec![false; count];
    }

    /// Process one buffer. Reads from `input`, writes to `output`.
    /// Both are interleaved stereo.
    ///
    /// Returns `MeterData` computed on the master output.
    ///
    /// This is called from the real-time audio thread — NO allocations.
    /// Pitch detection runs on a separate analysis thread.
    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        frames: usize,
        context: &ProcessContext,
    ) -> MeterData {
        let stereo_frames = frames * 2;

        // Zero the mix buffer
        for s in &mut self.mix_buffer[..stereo_frames] {
            *s = 0.0;
        }

        // Only play stems when transport is playing
        if context.is_playing {
            let any_solo = self.stem_solos.iter().any(|&s| s);

            for (i, player) in self.stem_players.iter_mut().enumerate() {
                let muted = self.stem_mutes.get(i).copied().unwrap_or(false);
                let soloed = self.stem_solos.get(i).copied().unwrap_or(false);
                let volume = self.stem_volumes.get(i).copied().unwrap_or(1.0);

                if muted || (any_solo && !soloed) {
                    player.advance(frames);
                    continue;
                }

                player.fill_buffer(&mut self.mix_buffer[..stereo_frames], frames, volume, context);
            }
        }

        // Live input monitoring — always active when input_volume > 0
        if self.input_volume > 0.0 {
            self.input_buffer[..stereo_frames].copy_from_slice(&input[..stereo_frames.min(input.len())]);

            if let Some(ref mut fx) = self.fx_chain {
                fx.process(&mut self.input_buffer[..stereo_frames], context.sample_rate);
            }

            for i in 0..stereo_frames {
                self.mix_buffer[i] += self.input_buffer[i] * self.input_volume;
            }
        }

        // Apply master volume and write to output
        for i in 0..stereo_frames.min(output.len()) {
            output[i] = self.mix_buffer[i] * self.master_volume;
        }

        // Compute metering on master output
        MeterData::from_interleaved(&output[..stereo_frames.min(output.len())])
    }
}

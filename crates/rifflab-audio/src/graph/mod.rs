pub mod node;
pub mod bus;

use rifflab_core::audio::{AudioProcessor, ProcessContext};
use rifflab_core::metering::MeterData;
use rifflab_fx::chain::EffectChain;
use rifflab_fx::multichain::MultibandRouter;

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
    /// Effect chain applied to the live input (single-chain mode).
    pub fx_chain: EffectChain,
    /// Multiband crossover router (when enabled, replaces fx_chain for processing).
    pub fx_multiband: Option<MultibandRouter>,
    /// Whether multiband mode is active.
    pub fx_multiband_active: bool,
    /// Whether the node graph has a connected Input→Output path.
    /// When false, live input monitoring is muted.
    pub fx_input_connected: bool,
    /// Input gain.
    pub input_volume: f32,
    /// Master volume.
    pub master_volume: f32,
    /// Internal mix buffer (reused across calls to avoid allocation).
    mix_buffer: Vec<f32>,
    /// Input processing buffer.
    input_buffer: Vec<f32>,
    /// Per-stem effect chains (same length as stem_players, None = no effects).
    pub stem_fx: Vec<Option<EffectChain>>,
    /// Scratch buffer for per-stem effect processing.
    stem_fx_buffer: Vec<f32>,
    /// Recording state: captures raw input (pre-effects) when armed.
    pub recording: bool,
    /// Recorded audio data (interleaved stereo f32). Grows during recording.
    pub recorded_data: Vec<f32>,
    /// Number of channels being recorded.
    pub recorded_channels: u16,

    // ─── Smoothed gain state (avoids clicks on volume/mute changes) ───
    smooth_master: f32,
    smooth_input: f32,
    smooth_stem_gains: Vec<f32>, // effective gain per stem (volume * mute/solo)
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
            fx_chain: EffectChain::new(),
            fx_multiband: None,
            fx_multiband_active: false,
            fx_input_connected: true, // default: connected (backward compat)
            input_volume: 1.0,
            master_volume: 1.0,
            stem_fx: Vec::new(),
            stem_fx_buffer: vec![0.0; buf_size * 2],
            mix_buffer: vec![0.0; buf_size * 2], // stereo
            input_buffer: vec![0.0; buf_size * 2],
            recording: false,
            recorded_data: Vec::new(),
            recorded_channels: 2,
            smooth_master: 1.0,
            smooth_input: 1.0,
            smooth_stem_gains: Vec::new(),
        }
    }

    /// Load stems into the graph.
    pub fn load_stems(&mut self, players: Vec<node::StemPlayer>) {
        let count = players.len();
        self.stem_players = players;
        self.stem_volumes = vec![1.0; count];
        self.stem_mutes = vec![false; count];
        self.stem_solos = vec![false; count];
        self.stem_fx = (0..count).map(|_| None).collect();
        self.smooth_stem_gains = vec![1.0; count];
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

            // Ensure smooth_stem_gains has enough entries
            while self.smooth_stem_gains.len() < self.stem_players.len() {
                self.smooth_stem_gains.push(0.0);
            }

            for (i, player) in self.stem_players.iter_mut().enumerate() {
                let muted = self.stem_mutes.get(i).copied().unwrap_or(false);
                let soloed = self.stem_solos.get(i).copied().unwrap_or(false);
                let volume = self.stem_volumes.get(i).copied().unwrap_or(1.0);

                // Target gain: 0 if muted/not-soloed, otherwise volume
                let target_gain = if muted || (any_solo && !soloed) {
                    0.0
                } else {
                    volume
                };

                let prev_gain = self.smooth_stem_gains[i];

                // If both previous and target are zero, just advance — no audio to process
                if prev_gain == 0.0 && target_gain == 0.0 {
                    player.advance(frames);
                    continue;
                }

                // Check if this stem has per-track effects
                let has_fx = self.stem_fx.get(i).and_then(|f| f.as_ref()).map_or(false, |c| !c.is_empty());

                if has_fx {
                    // Fill into scratch buffer at full volume (gain applied after FX)
                    for s in &mut self.stem_fx_buffer[..stereo_frames] { *s = 0.0; }
                    player.fill_buffer(&mut self.stem_fx_buffer[..stereo_frames], frames, 1.0, context);
                    if let Some(Some(ref mut chain)) = self.stem_fx.get_mut(i) {
                        chain.process(&mut self.stem_fx_buffer[..stereo_frames], context.sample_rate);
                    }
                    // Apply ramped gain and add to mix
                    for frame in 0..frames {
                        let t = (frame + 1) as f32 / frames as f32;
                        let gain = prev_gain + (target_gain - prev_gain) * t;
                        self.mix_buffer[frame * 2] += self.stem_fx_buffer[frame * 2] * gain;
                        self.mix_buffer[frame * 2 + 1] += self.stem_fx_buffer[frame * 2 + 1] * gain;
                    }
                } else {
                    // Fill into scratch buffer at full volume, then ramp-add to mix
                    for s in &mut self.stem_fx_buffer[..stereo_frames] { *s = 0.0; }
                    player.fill_buffer(&mut self.stem_fx_buffer[..stereo_frames], frames, 1.0, context);
                    for frame in 0..frames {
                        let t = (frame + 1) as f32 / frames as f32;
                        let gain = prev_gain + (target_gain - prev_gain) * t;
                        self.mix_buffer[frame * 2] += self.stem_fx_buffer[frame * 2] * gain;
                        self.mix_buffer[frame * 2 + 1] += self.stem_fx_buffer[frame * 2 + 1] * gain;
                    }
                }

                self.smooth_stem_gains[i] = target_gain;
            }
        }

        // Record raw input (pre-effects) when armed
        if self.recording {
            let input_len = stereo_frames.min(input.len());
            self.recorded_data.extend_from_slice(&input[..input_len]);
        }

        // Live input monitoring — active when input_volume > 0 AND graph has a connected path
        let target_input = if self.fx_input_connected { self.input_volume } else { 0.0 };

        if self.smooth_input > 0.0 || target_input > 0.0 {
            self.input_buffer[..stereo_frames].copy_from_slice(&input[..stereo_frames.min(input.len())]);

            // Route through either multiband crossover or single chain
            if self.fx_multiband_active {
                if let Some(ref mut mb) = self.fx_multiband {
                    mb.process(&mut self.input_buffer[..stereo_frames], context.sample_rate);
                }
            } else if !self.fx_chain.is_empty() {
                self.fx_chain.process(&mut self.input_buffer[..stereo_frames], context.sample_rate);
            }

            // Ramp input volume across the buffer
            let prev_input = self.smooth_input;
            for frame in 0..frames {
                let t = (frame + 1) as f32 / frames as f32;
                let gain = prev_input + (target_input - prev_input) * t;
                self.mix_buffer[frame * 2] += self.input_buffer[frame * 2] * gain;
                self.mix_buffer[frame * 2 + 1] += self.input_buffer[frame * 2 + 1] * gain;
            }
            self.smooth_input = target_input;
        }

        // Apply master volume with ramp and write to output
        let target_master = self.master_volume;
        let prev_master = self.smooth_master;
        for frame in 0..frames.min(output.len() / 2) {
            let t = (frame + 1) as f32 / frames as f32;
            let gain = prev_master + (target_master - prev_master) * t;
            output[frame * 2] = self.mix_buffer[frame * 2] * gain;
            output[frame * 2 + 1] = self.mix_buffer[frame * 2 + 1] * gain;
        }
        self.smooth_master = target_master;

        // Compute metering on master output
        MeterData::from_interleaved(&output[..stereo_frames.min(output.len())])
    }
}

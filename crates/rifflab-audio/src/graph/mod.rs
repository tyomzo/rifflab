pub mod node;
pub mod bus;

use rifflab_core::analysis::PitchFrame;
use rifflab_core::audio::{AudioProcessor, ProcessContext};
use rifflab_core::metering::MeterData;
use rifflab_analysis::pitch::yin::YinDetector;

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
    /// Mono downmix buffer for pitch detection (reused across calls).
    mono_buffer: Vec<f32>,
    /// YIN pitch detector for live input analysis.
    pitch_detector: Option<YinDetector>,
}

impl AudioGraph {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            stem_players: Vec::new(),
            stem_volumes: Vec::new(),
            stem_mutes: Vec::new(),
            stem_solos: Vec::new(),
            fx_chain: None,
            input_volume: 1.0,
            master_volume: 1.0,
            mix_buffer: vec![0.0; max_buffer_size * 2], // stereo
            input_buffer: vec![0.0; max_buffer_size * 2],
            mono_buffer: vec![0.0; max_buffer_size],
            pitch_detector: None,
        }
    }

    /// Enable pitch detection on the live input at the given sample rate.
    pub fn enable_pitch_detection(&mut self, sample_rate: u32) {
        self.pitch_detector = Some(YinDetector::new(sample_rate));
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
    /// Returns `(MeterData, Option<PitchFrame>)`. The pitch frame is `Some`
    /// only when a YinDetector has been enabled via `enable_pitch_detection`.
    ///
    /// This is called from the real-time audio thread — NO allocations
    /// (except inside YinDetector which is a known TODO).
    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        frames: usize,
        context: &ProcessContext,
    ) -> (MeterData, Option<PitchFrame>) {
        let stereo_frames = frames * 2;

        // Zero the mix buffer
        for s in &mut self.mix_buffer[..stereo_frames] {
            *s = 0.0;
        }

        // Check if any solo is active
        let any_solo = self.stem_solos.iter().any(|&s| s);

        // Sum stem players into mix buffer
        for (i, player) in self.stem_players.iter_mut().enumerate() {
            let muted = self.stem_mutes.get(i).copied().unwrap_or(false);
            let soloed = self.stem_solos.get(i).copied().unwrap_or(false);
            let volume = self.stem_volumes.get(i).copied().unwrap_or(1.0);

            // Skip if muted, or if solo is active on other tracks
            if muted || (any_solo && !soloed) {
                // Still advance the player position
                player.advance(frames);
                continue;
            }

            player.fill_buffer(&mut self.mix_buffer[..stereo_frames], frames, volume, context);
        }

        // Process live input through effects chain
        self.input_buffer[..stereo_frames].copy_from_slice(&input[..stereo_frames.min(input.len())]);

        // Run pitch detection on the raw input BEFORE effects processing.
        // Downmix stereo to mono: average L+R channels.
        let pitch_frame = if let Some(ref mut detector) = self.pitch_detector {
            let input_samples = stereo_frames.min(input.len());
            let mono_frames = input_samples / 2;
            for i in 0..mono_frames {
                self.mono_buffer[i] = (input[i * 2] + input[i * 2 + 1]) * 0.5;
            }
            Some(detector.detect(&self.mono_buffer[..mono_frames]))
        } else {
            None
        };

        if let Some(ref mut fx) = self.fx_chain {
            fx.process(&mut self.input_buffer[..stereo_frames], context.sample_rate);
        }

        // Mix input (post-fx) into the mix buffer
        for i in 0..stereo_frames {
            self.mix_buffer[i] += self.input_buffer[i] * self.input_volume;
        }

        // Apply master volume and write to output
        for i in 0..stereo_frames.min(output.len()) {
            output[i] = self.mix_buffer[i] * self.master_volume;
        }

        // Compute metering on master output
        let meter = MeterData::from_interleaved(&output[..stereo_frames.min(output.len())]);
        (meter, pitch_frame)
    }
}

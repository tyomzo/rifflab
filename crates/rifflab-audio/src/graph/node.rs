use rifflab_core::audio::ProcessContext;
use rifflab_core::song::StemType;
use std::sync::Arc;

/// A stem player node that reads from pre-loaded audio data.
pub struct StemPlayer {
    /// Stem type.
    pub stem_type: StemType,
    /// Audio data: interleaved stereo f32 samples.
    data: Arc<Vec<f32>>,
    /// Number of channels in the data.
    channels: u16,
    /// Current read position in frames.
    position: u64,
    /// Total frames.
    total_frames: u64,
}

impl StemPlayer {
    /// Create a stem player from pre-loaded audio data.
    ///
    /// `data` is interleaved f32 samples.
    /// `channels` is typically 1 or 2.
    pub fn new(stem_type: StemType, data: Vec<f32>, channels: u16) -> Self {
        let total_frames = if channels > 0 {
            data.len() as u64 / channels as u64
        } else {
            0
        };
        Self {
            stem_type,
            data: Arc::new(data),
            channels,
            position: 0,
            total_frames,
        }
    }

    /// Advance the read position without producing output (for muted stems).
    pub fn advance(&mut self, frames: usize) {
        self.position += frames as u64;
    }

    /// Fill the interleaved stereo output buffer, mixing into it (additive).
    pub fn fill_buffer(
        &mut self,
        output: &mut [f32],
        frames: usize,
        volume: f32,
        context: &ProcessContext,
    ) {
        // Sync position with transport
        self.position = context.transport_position;

        let ch = self.channels as u64;
        if ch == 0 {
            return;
        }

        for frame in 0..frames {
            let read_pos = self.position + frame as u64;
            if read_pos >= self.total_frames {
                break;
            }

            let data_idx = (read_pos * ch) as usize;
            let out_idx = frame * 2;

            if self.channels == 1 {
                // Mono: duplicate to both channels
                let sample = self.data.get(data_idx).copied().unwrap_or(0.0) * volume;
                if out_idx + 1 < output.len() {
                    output[out_idx] += sample;
                    output[out_idx + 1] += sample;
                }
            } else {
                // Stereo
                let l = self.data.get(data_idx).copied().unwrap_or(0.0) * volume;
                let r = self.data.get(data_idx + 1).copied().unwrap_or(0.0) * volume;
                if out_idx + 1 < output.len() {
                    output[out_idx] += l;
                    output[out_idx + 1] += r;
                }
            }
        }
    }

    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    pub fn set_position(&mut self, frame: u64) {
        self.position = frame;
    }
}

use serde::{Deserialize, Serialize};

/// Sample rates supported by the audio engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SampleRate {
    Hz44100,
    Hz48000,
    Hz96000,
}

impl SampleRate {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Hz44100 => 44100,
            Self::Hz48000 => 48000,
            Self::Hz96000 => 96000,
        }
    }

    pub fn from_u32(rate: u32) -> Option<Self> {
        match rate {
            44100 => Some(Self::Hz44100),
            48000 => Some(Self::Hz48000),
            96000 => Some(Self::Hz96000),
            _ => None,
        }
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self::Hz48000
    }
}

/// Buffer sizes supported by the audio engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BufferSize {
    B64,
    B128,
    B256,
    B512,
}

impl BufferSize {
    pub fn as_usize(self) -> usize {
        match self {
            Self::B64 => 64,
            Self::B128 => 128,
            Self::B256 => 256,
            Self::B512 => 512,
        }
    }
}

impl Default for BufferSize {
    fn default() -> Self {
        Self::B256
    }
}

/// Audio engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: SampleRate,
    pub buffer_size: BufferSize,
    pub input_channels: u16,
    pub output_channels: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::Hz48000,
            buffer_size: BufferSize::B256,
            input_channels: 2,
            output_channels: 2,
        }
    }
}

/// Trait implemented by all DSP processors (effects, analyzers).
///
/// Called from the real-time audio thread.
/// Implementations MUST NOT allocate, lock, or make syscalls.
pub trait AudioProcessor: Send {
    /// Process audio in-place.
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);

    /// Set a parameter value. Must be real-time safe.
    fn set_param(&mut self, param: ParamId, value: f32);

    /// Reset internal state (e.g., clear delay lines).
    fn reset(&mut self);

    /// Return the processor's name for display.
    fn name(&self) -> &str;

    /// Whether this processor is currently bypassed.
    fn is_bypassed(&self) -> bool {
        false
    }
}

/// Parameter identifier for an audio processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParamId(pub u32);

/// Metadata about a single parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDescriptor {
    pub id: ParamId,
    pub name: String,
    pub unit: String,        // "dB", "ms", "Hz", "%", ""
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: Option<f32>,   // None = continuous
    pub kind: ParamKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParamKind {
    Float,
    Int,
    Bool,
    Enum(Vec<String>),
}

/// Self-describing audio processor. All effects implement this.
pub trait EffectDescriptor: AudioProcessor {
    /// A unique string identifying the effect type (e.g., "builtin:compressor").
    fn effect_type_id(&self) -> &str;

    /// Return descriptors for every parameter this effect exposes.
    fn param_descriptors(&self) -> Vec<ParamDescriptor>;

    /// Read the current value of a parameter.
    fn get_param(&self, param: ParamId) -> f32;
}

/// Context passed to graph nodes during processing.
#[derive(Debug, Clone)]
pub struct ProcessContext {
    pub sample_rate: u32,
    pub buffer_size: usize,
    pub transport_position: u64,
    pub is_playing: bool,
    pub bpm: Option<f64>,
}

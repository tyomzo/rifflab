mod config;
mod decode;
mod import;
mod library;
mod midi_input;
mod node_editor;
mod preset_bank;
mod preset_graph;
mod session;
mod tab_view;

/// Arturia MiniLab 3 default knob CC numbers (Arturia preset).
const MINILAB3_KNOB_CCS: [u8; 8] = [74, 71, 76, 77, 93, 18, 19, 16];
/// Arturia MiniLab 3 default fader CC numbers.
const MINILAB3_FADER_CCS: [u8; 4] = [82, 83, 85, 17];

/// Saved MIDI controller mapping (knob + fader CCs).
#[derive(serde::Serialize, serde::Deserialize)]
struct MidiMapping {
    knob_ccs: Vec<u8>,
    fader_ccs: Vec<u8>,
}

impl MidiMapping {
    /// Directory for MIDI mapping files.
    fn mappings_dir() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "rifflab")
            .map(|dirs| dirs.config_dir().join("midi_mappings"))
    }

    /// Sanitize device name for use as filename.
    fn device_filename(port_name: &str) -> String {
        // Use the part before the port number (e.g., "Minilab3:Minilab3 MIDI 20:0" → "Minilab3")
        let name = port_name.split(':').next().unwrap_or(port_name);
        name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
    }

    fn save(port_name: &str, knobs: &[u8], faders: &[u8]) -> Result<(), String> {
        let dir = Self::mappings_dir().ok_or("No config dir")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{e}"))?;
        let path = dir.join(format!("{}.json", Self::device_filename(port_name)));
        let mapping = MidiMapping { knob_ccs: knobs.to_vec(), fader_ccs: faders.to_vec() };
        let json = serde_json::to_string_pretty(&mapping).map_err(|e| format!("{e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("{e}"))?;
        Ok(())
    }

    fn load(port_name: &str) -> Option<MidiMapping> {
        let dir = Self::mappings_dir()?;
        let path = dir.join(format!("{}.json", Self::device_filename(port_name)));
        let json = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&json).ok()
    }
}

use anyhow::Result;
use clap::Parser;
use eframe::egui;
use rifflab_audio::backend::{self, DeviceInfo};
use rifflab_audio::engine::AudioEngine;
use rifflab_audio::graph::node::StemPlayer;
use rifflab_core::analysis::{NoteEvent, PitchFrame};
use rifflab_core::audio::{ParamDescriptor, ParamId, ParamKind};
use rifflab_core::metering::MeterData;
use rifflab_core::practice::AccuracyBucket;
use rifflab_core::song::StemType;
use rifflab_core::transport::{LoopRegion, TransportState};
use rifflab_cue::{Cue, CueAction, CueEngine, CueList, CuePosition, DispatchedAction};
use rifflab_fx::multichain::MultibandRouter;
use rifflab_fx::registry::EffectRegistry;
use rifflab_practice::compare::Comparator;
use rifflab_practice::scoring::SessionScorer;
use std::sync::{Arc, Mutex};

use config::AppConfig;
use library::Library;

// ─── CLI Arguments ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "rifflab", about = "Music practice workstation")]
struct Args {
    /// Audio file to load (WAV, FLAC, MP3)
    file: Option<String>,
    /// Audio backend preference
    #[arg(long, default_value = "auto")]
    backend: String,
    /// Buffer size in samples (overrides config file)
    #[arg(long)]
    buffer_size: Option<u32>,
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Number of source samples summarized into one overview peak entry.
const OVERVIEW_SAMPLES_PER_PEAK: usize = 128;

/// Default horizontal zoom: how many audio frames one screen pixel represents.
const DEFAULT_FRAMES_PER_PIXEL: f64 = 512.0;

/// Minimum / maximum zoom levels (frames per pixel).
const MIN_FRAMES_PER_PIXEL: f64 = 8.0;
const MAX_FRAMES_PER_PIXEL: f64 = 16384.0;

/// Sidebar width in logical pixels.
const SIDEBAR_WIDTH: f32 = 180.0;

/// Timeline ruler height in logical pixels.
const RULER_HEIGHT: f32 = 24.0;

/// Track lane height in logical pixels.
const TRACK_LANE_HEIGHT: f32 = 120.0;

/// Default bottom drawer height in logical pixels.
const DEFAULT_DRAWER_HEIGHT: f32 = 200.0;
/// Minimum bottom drawer height.
const MIN_DRAWER_HEIGHT: f32 = 80.0;
/// Maximum bottom drawer height.
const MAX_DRAWER_HEIGHT: f32 = 2000.0; // effectively unlimited — user controls via drag
/// Width of the piano keyboard strip on the left of the piano roll.
const PIANO_KEY_WIDTH: f32 = 48.0;
/// Height of each semitone row in the piano roll.
const SEMITONE_ROW_HEIGHT: f32 = 12.0;
/// Confidence threshold below which pitch frames are treated as silence.
const PITCH_CONFIDENCE_THRESHOLD: f32 = 0.5;

/// Track colors for stems.
const TRACK_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(0, 180, 120),   // green
    egui::Color32::from_rgb(60, 140, 220),   // blue
    egui::Color32::from_rgb(220, 160, 40),   // amber
    egui::Color32::from_rgb(200, 60, 100),   // rose
    egui::Color32::from_rgb(140, 80, 200),   // purple
    egui::Color32::from_rgb(80, 200, 200),   // cyan
];

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    env_logger::init();
    log::info!("Starting RiffLab");

    // 1. Parse CLI arguments with clap
    let args = Args::parse();

    // 2. Init library (creates ~/.local/share/rifflab/ tree)
    let _library = match Library::init() {
        Ok(lib) => {
            log::info!("Library root: {}", lib.root.display());
            Some(lib)
        }
        Err(e) => {
            log::warn!("Library init failed (non-fatal): {e}");
            None
        }
    };

    // 3. Load config (or create defaults), apply CLI overrides
    let mut app_config = AppConfig::load_or_create_default().unwrap_or_else(|e| {
        log::warn!("Config load failed, using defaults: {e}");
        AppConfig::default()
    });
    app_config.apply_cli_overrides(&args.backend, args.buffer_size.unwrap_or(app_config.audio.buffer_size));

    // 4. Create audio engine from config
    let audio_config = app_config.to_audio_config();
    let mut target_rate = audio_config.sample_rate.as_u32();
    let buffer_size = audio_config.buffer_size.as_usize();
    let (mut engine, meter_rx, pitch_rx) = AudioEngine::new(audio_config);
    engine.set_backend_prefs(
        &app_config.audio.backend,
        &app_config.audio.output_device,
        &app_config.audio.input_device,
    );
    let engine = Arc::new(Mutex::new(engine));

    // 5. File path from CLI (will be loaded after UI opens via load_file())
    let file_path = args.file.clone();

    // 6. Start engine and update config to match actual hardware rate
    {
        let mut eng = engine.lock().unwrap();
        match eng.start() {
            Ok(()) => {
                // Check if hardware negotiated a different sample rate
                if let Some(backend) = eng.backend_ref() {
                    if let Some(actual_rate) = backend.actual_sample_rate() {
                        if actual_rate != target_rate {
                            log::info!("Hardware rate {}Hz differs from config {}Hz, adapting", actual_rate, target_rate);
                            target_rate = actual_rate;
                        }
                    }
                }
                log::info!("Audio engine started ({}Hz)", target_rate);
            }
            Err(e) => log::error!("Failed to start engine: {e}"),
        }
    }

    // 7. Launch UI with all data connected
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 700.0])
            .with_title("RiffLab"),
        ..Default::default()
    };

    let app_engine = Arc::clone(&engine);
    eframe::run_native(
        "RiffLab",
        options,
        Box::new(move |cc| {
            // Use dark visuals
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(RiffLabApp::new(
                app_engine,
                meter_rx,
                pitch_rx,
                file_path,
                Vec::new(),
                Vec::new(),
                target_rate,
                buffer_size,
                app_config,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("UI error: {e}"))?;

    Ok(())
}

// ─── Waveform Overview ──────────────────────────────────────────────────────

/// Pre-computed waveform overview for efficient rendering at any zoom level.
struct WaveformOverview {
    /// (min, max) peak pairs at overview resolution.
    peaks: Vec<(f32, f32)>,
    /// The number of source frames per peak entry.
    samples_per_peak: usize,
    /// Total source frames.
    total_frames: u64,
    /// Color for this track.
    color: egui::Color32,
    /// Track name.
    name: String,
}

impl WaveformOverview {
    /// Build an overview from interleaved audio data.
    fn from_interleaved(
        data: &[f32],
        channels: u16,
        samples_per_peak: usize,
        color: egui::Color32,
        name: String,
    ) -> Self {
        let ch = channels.max(1) as usize;
        let total_frames = data.len() / ch;
        let num_peaks = total_frames.div_ceil(samples_per_peak);
        let mut peaks = Vec::with_capacity(num_peaks);

        for chunk_idx in 0..num_peaks {
            let start_frame = chunk_idx * samples_per_peak;
            let end_frame = (start_frame + samples_per_peak).min(total_frames);
            let mut min_val: f32 = 0.0;
            let mut max_val: f32 = 0.0;
            for frame in start_frame..end_frame {
                // Mono-downmix: average channels
                let mut sample = 0.0f32;
                for c in 0..ch {
                    sample += data[frame * ch + c];
                }
                sample /= ch as f32;
                min_val = min_val.min(sample);
                max_val = max_val.max(sample);
            }
            peaks.push((min_val, max_val));
        }

        Self {
            peaks,
            samples_per_peak,
            total_frames: total_frames as u64,
            color,
            name,
        }
    }

    /// Get the (min, max) peak values for a range of source frames.
    fn peaks_for_range(&self, start_frame: u64, end_frame: u64) -> (f32, f32) {
        let first_peak = (start_frame as usize) / self.samples_per_peak;
        let last_peak = (end_frame as usize).div_ceil(self.samples_per_peak);
        let first_peak = first_peak.min(self.peaks.len());
        let last_peak = last_peak.min(self.peaks.len());

        let mut min_val: f32 = 0.0;
        let mut max_val: f32 = 0.0;
        for i in first_peak..last_peak {
            let (lo, hi) = self.peaks[i];
            min_val = min_val.min(lo);
            max_val = max_val.max(hi);
        }
        (min_val, max_val)
    }
}

// ─── Loop Drag State ─────────────────────────────────────────────────────────

/// Tracks drag state for creating/editing loop regions on the timeline ruler.
#[derive(Default)]
struct LoopDragState {
    /// True if the user is currently dragging on the ruler to create a loop.
    dragging: bool,
    /// Frame position where the drag started.
    start_frame: u64,
    /// Frame position where the drag currently is.
    current_frame: u64,
}

// ─── Bottom Drawer Types ─────────────────────────────────────────────────────

/// Cached snapshot of multiband router state.
#[allow(dead_code)]
struct MultibandSnapshot {
    crossover_low_mid: f32,
    crossover_mid_high: f32,
    band_gains: [f32; 3],
    bands: [Vec<FxSnapCached>; 3],
}

/// Pre-computed spectrogram for one audio source, with display metadata.
#[allow(dead_code)]
struct SpectrogramDisplay {
    data: rifflab_analysis::spectrogram::SpectrogramResult,
    color: egui::Color32,
    name: String,
}

/// Which tab is active in the bottom drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Presets,
    Effects,
    Tab,
    PianoRoll,
    Accuracy,
}

/// Which view is shown in the central arrangement area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrangementView {
    Waveform,
    Spectrogram,
}

/// A played note recorded during practice, for piano roll display.
#[derive(Debug, Clone)]
/// Cached snapshot of a single effect's state for the UI.
#[allow(dead_code)]
struct FxSnapCached {
    name: String,
    type_id: String,
    bypassed: bool,
    params: Vec<(ParamDescriptor, f32)>,
}

struct PlayedNote {
    midi_note: u8,
    start_seconds: f64,
    end_seconds: f64,
    accuracy: AccuracyBucket,
}

// ─── Background File Loading ─────────────────────────────────────────────────

/// A single stem track ready to load.
struct StemTrack {
    stem_type: StemType,
    decoded: decode::DecodedAudio,
    overview: WaveformOverview,
    spectrogram: rifflab_analysis::spectrogram::SpectrogramResult,
}

/// Result of background file decoding + optional stem separation.
struct LoadedFile {
    file_name: String,
    stems: Vec<StemTrack>,
}

/// Messages sent from the loading thread to the UI.
enum LoadMsg {
    /// Progress status text.
    Status(String),
    /// Loading finished successfully.
    Done(LoadedFile),
    /// Loading failed.
    Error(String),
}

/// State of background file loading.
enum FileLoadState {
    Idle,
    Loading {
        file_name: String,
        receiver: std::sync::mpsc::Receiver<LoadMsg>,
    },
}

/// A single log entry.
struct LogEntry {
    text: String,
    is_error: bool,
    timestamp: std::time::Instant,
}

/// Scrollable message log for file loading and other operations.
struct MessageLog {
    entries: Vec<LogEntry>,
    /// Whether the log popup is open.
    open: bool,
}

impl MessageLog {
    fn new() -> Self {
        Self { entries: Vec::new(), open: false }
    }

    fn push(&mut self, text: String, is_error: bool) {
        self.entries.push(LogEntry {
            text,
            is_error,
            timestamp: std::time::Instant::now(),
        });
    }

    fn last(&self) -> Option<&LogEntry> {
        self.entries.last()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

// ─── Audio Settings Panel ────────────────────────────────────────────────────

/// PipeWire server settings (read via pw-metadata).
#[derive(Default, Clone)]
struct PipeWireSettings {
    available: bool,
    clock_rate: u32,
    allowed_rates: Vec<u32>,
    quantum: u32,
    min_quantum: u32,
    max_quantum: u32,
    force_quantum: u32,
}

impl PipeWireSettings {
    fn query() -> Self {
        let output = std::process::Command::new("pw-metadata")
            .args(["-n", "settings", "0"])
            .output();
        let output = match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return Self::default(),
        };

        let mut s = Self { available: true, ..Default::default() };
        for line in output.lines() {
            if let Some(rest) = line.strip_prefix("update: id:0 key:'") {
                let parts: Vec<&str> = rest.splitn(2, "' value:'").collect();
                if parts.len() == 2 {
                    let key = parts[0];
                    let val = parts[1].trim_end_matches("' type:''").trim_end_matches('\'');
                    match key {
                        "clock.rate" => s.clock_rate = val.parse().unwrap_or(0),
                        "clock.quantum" => s.quantum = val.parse().unwrap_or(0),
                        "clock.min-quantum" => s.min_quantum = val.parse().unwrap_or(0),
                        "clock.max-quantum" => s.max_quantum = val.parse().unwrap_or(0),
                        "clock.force-quantum" => s.force_quantum = val.parse().unwrap_or(0),
                        "clock.allowed-rates" => {
                            // Format: "[ 48000 ]" or "[ 44100 48000 96000 ]"
                            let inner = val.trim_start_matches("[ ").trim_end_matches(" ]");
                            s.allowed_rates = inner.split_whitespace()
                                .filter_map(|r| r.parse().ok())
                                .collect();
                        }
                        _ => {}
                    }
                }
            }
        }
        s
    }

    fn set_rate(&self, rate: u32) -> Result<(), String> {
        let status = std::process::Command::new("pw-metadata")
            .args(["-n", "settings", "0", "clock.rate", &rate.to_string()])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("pw-metadata returned non-zero".into());
        }
        // Also update allowed-rates to include this rate
        let rates_val = format!("[ {} ]", rate);
        let _ = std::process::Command::new("pw-metadata")
            .args(["-n", "settings", "0", "clock.allowed-rates", &rates_val])
            .status();
        Ok(())
    }

    fn set_quantum(&self, quantum: u32) -> Result<(), String> {
        let val = quantum.to_string();
        let status = std::process::Command::new("pw-metadata")
            .args(["-n", "settings", "0", "clock.force-quantum", &val])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("pw-metadata returned non-zero".into());
        }
        Ok(())
    }
}

/// Editing state for the audio settings window.
struct AudioSettings {
    open: bool,
    /// Cached device list (refreshed when panel opens).
    output_devices: Vec<DeviceInfo>,
    input_devices: Vec<DeviceInfo>,
    available_backends: Vec<String>,
    /// Editing copies of config values.
    backend: String,
    sample_rate: u32,
    buffer_size: u32,
    output_device: String,
    input_device: String,
    /// Status message after apply.
    status_msg: String,
    /// Whether settings have been modified since last apply.
    dirty: bool,
    /// PipeWire server settings (for JACK mode).
    pipewire: PipeWireSettings,
    /// Editing copies of PipeWire values.
    pw_rate: u32,
    pw_quantum: u32,
}

impl AudioSettings {
    fn new(config: &AppConfig) -> Self {
        let pw = PipeWireSettings::query();
        let pw_rate = pw.clock_rate;
        let pw_quantum = if pw.force_quantum > 0 { pw.force_quantum } else { pw.quantum };
        let mut s = Self {
            open: false,
            output_devices: Vec::new(),
            input_devices: Vec::new(),
            available_backends: Vec::new(),
            backend: config.audio.backend.clone(),
            sample_rate: config.audio.sample_rate,
            buffer_size: config.audio.buffer_size,
            output_device: config.audio.output_device.clone(),
            input_device: config.audio.input_device.clone(),
            status_msg: String::new(),
            dirty: false,
            pipewire: pw,
            pw_rate,
            pw_quantum,
        };
        s.refresh_devices();
        s
    }

    fn refresh_devices(&mut self) {
        let all_devices = backend::enumerate_devices();
        self.output_devices = all_devices.iter().filter(|d| d.is_output).cloned().collect();
        self.input_devices = all_devices.iter().filter(|d| d.is_input).cloned().collect();
        self.available_backends = backend::available_backends()
            .iter()
            .map(|b| match b {
                backend::BackendType::Alsa => "alsa".to_string(),
                backend::BackendType::Jack => "jack".to_string(),
            })
            .collect();
        // Always include "auto" as first option
        self.available_backends.insert(0, "auto".to_string());
    }

    fn refresh_pipewire(&mut self) {
        self.pipewire = PipeWireSettings::query();
        self.pw_rate = self.pipewire.clock_rate;
        self.pw_quantum = if self.pipewire.force_quantum > 0 {
            self.pipewire.force_quantum
        } else {
            self.pipewire.quantum
        };
    }

    fn load_from_config(&mut self, config: &AppConfig) {
        self.backend = config.audio.backend.clone();
        self.sample_rate = config.audio.sample_rate;
        self.buffer_size = config.audio.buffer_size;
        self.output_device = config.audio.output_device.clone();
        self.input_device = config.audio.input_device.clone();
        self.dirty = false;
        self.status_msg.clear();
        self.refresh_pipewire();
    }


}

// ─── App ─────────────────────────────────────────────────────────────────────

struct RiffLabApp {
    engine: Arc<Mutex<AudioEngine>>,
    meter_rx: rifflab_core::rtrb::Consumer<MeterData>,
    pitch_rx: rifflab_core::rtrb::Consumer<PitchFrame>,
    file_name: String,

    /// Latest meter readings.
    peak_l: f32,
    peak_r: f32,
    rms_l: f32,
    rms_r: f32,

    /// Latest detected pitch frame.
    current_pitch: PitchFrame,
    /// Smoothed tuner display state: holds the last confident note.
    tuner_note: u8,
    tuner_cents: f32,
    tuner_confidence: f32,
    /// How many consecutive frames the current tuner note has been held.
    tuner_hold_count: u32,
    /// Timestamp of last confident detection (for display timeout).
    tuner_last_active: std::time::Instant,
    /// Tuner settings popup.
    tuner_settings_open: bool,
    /// Tuner: minimum confidence to display (0.0–1.0).
    tuner_min_confidence: f32,
    /// Tuner: noise floor RMS threshold (0.0–0.2).
    tuner_noise_floor: f32,
    /// Tuner: how many consistent frames before switching note.
    tuner_hold_frames: u32,
    /// Tuner: display timeout in ms after last detection.
    tuner_timeout_ms: u32,

    /// Real-time comparator (active when reference notes are loaded).
    comparator: Option<Comparator>,
    /// Session scoring accumulator.
    scorer: SessionScorer,

    /// Pre-computed waveform overviews (one per stem).
    waveform_overviews: Vec<WaveformOverview>,

    /// Arrangement view zoom: frames per pixel.
    frames_per_pixel: f64,
    /// Arrangement view horizontal scroll offset in frames.
    scroll_offset_frames: f64,
    /// Whether the view should auto-follow the playhead.
    auto_follow: bool,

    /// Audio config info for status bar.
    sample_rate: u32,
    buffer_size: usize,

    /// Loop drag state for the timeline ruler.
    loop_drag: LoopDragState,

    // ── Piano roll / bottom drawer state ──
    /// Whether the bottom drawer is open.
    drawer_open: bool,
    /// Bottom drawer height in logical pixels.
    drawer_height: f32,
    /// Active tab in the bottom drawer.
    active_tab: BottomTab,
    /// Reference notes for piano roll display.
    reference_notes: Vec<NoteEvent>,
    /// Recorded played notes during practice (for piano roll display).
    played_notes: Vec<PlayedNote>,
    /// Piano roll vertical scroll offset (in MIDI note units from bottom).
    piano_roll_scroll_note: f32,
    /// Tracks the MIDI note currently being played (for note onset/offset detection).
    tracking_midi_note: u8,
    /// Whether the tracker was silent on the previous frame.
    tracking_was_silent: bool,
    /// File to load on first frame (from CLI arg).
    pending_file: Option<std::path::PathBuf>,
    /// Audio settings panel state.
    audio_settings: AudioSettings,
    /// Persisted app config (for saving).
    app_config: AppConfig,
    /// Background file loading state.
    file_load_state: FileLoadState,
    /// Scrollable message log for status/errors.
    message_log: MessageLog,
    /// Cached sidebar state to avoid locking every frame.
    /// (num_stems, solos, mutes, volumes, master_vol, input_vol, has_fx)
    sidebar_snapshot: Option<(usize, Vec<bool>, Vec<bool>, Vec<f32>, f32, f32, Vec<bool>)>,
    /// Effect registry for creating new effects.
    fx_registry: EffectRegistry,
    #[allow(dead_code)]
    fx_add_open: bool,
    #[allow(dead_code)]
    fx_snapshot: Vec<FxSnapCached>,
    #[allow(dead_code)]
    fx_mb_snapshot: Option<MultibandSnapshot>,
    #[allow(dead_code)]
    fx_multiband_mode: bool,
    /// When the effects snapshot was last refreshed.
    fx_snapshot_time: std::time::Instant,
    /// Force refresh on next frame (after add/remove/reorder).
    fx_snapshot_dirty: bool,
    #[allow(dead_code)]
    fx_preset_path: Option<std::path::PathBuf>,
    /// Current preset name (shown in UI).
    fx_preset_name: String,
    /// Current session directory (None = never saved).
    session_path: Option<std::path::PathBuf>,
    /// Original file path for session saving.
    original_file_path: Option<std::path::PathBuf>,
    /// Background save receiver.
    save_receiver: Option<std::sync::mpsc::Receiver<(bool, String)>>,
    /// Pre-computed spectrograms (one per stem).
    spectrograms: Vec<SpectrogramDisplay>,
    /// Cached spectrogram texture.
    spectrogram_texture: Option<egui::TextureHandle>,
    /// Cache key for the currently displayed texture.
    spectrogram_cache_key: (u64, u64, u32, u32, u64),
    /// Pending spectrogram render from background thread.
    spectrogram_pending: Option<std::sync::mpsc::Receiver<(egui::ColorImage, (u64, u64, u32, u32, u64))>>,
    /// Key of the render currently in flight (to avoid duplicate dispatches).
    spectrogram_pending_key: (u64, u64, u32, u32, u64),
    /// Which view mode is active in the arrangement area.
    arrangement_view: ArrangementView,
    /// Cue engine for timeline automation.
    cue_engine: CueEngine,
    /// Node graph for effects routing.
    fx_graph: node_editor::FxGraph,
    /// Node editor interaction state (not serialized).
    node_editor_state: node_editor::NodeEditorState,
    /// Hash of last compiled graph (to detect changes).
    fx_graph_compiled_hash: u64,
    /// MIDI input connection.
    midi_connection: Option<midi_input::MidiConnection>,
    midi_rx: Option<std::sync::mpsc::Receiver<midi_input::MidiEvent>>,
    midi_port_names: Vec<String>,
    midi_selected_port: usize,
    /// Preset bank (multiple presets loaded, one active).
    preset_bank: preset_bank::PresetBank,
    /// MIDI/Preset panel open.
    midi_panel_open: bool,
    /// Preset navigation graph.
    preset_nav: preset_graph::PresetGraph,
    /// MIDI learn target.
    midi_learn: preset_graph::MidiLearnTarget,
    /// Which preset is being edited (its pipeline shown in the effect editor).
    editing_preset_id: Option<u64>,
    /// Whether we are currently recording input.
    is_recording: bool,
    /// Per-track FxGraph (for editing in the node editor). Index = track index.
    stem_graphs: Vec<Option<node_editor::FxGraph>>,
    /// Which track's FX is being edited (None = live input, Some(idx) = track).
    editing_track_fx: Option<usize>,
    /// Throttle: when track FX params changed, defer recompile to avoid locking graph every frame.
    track_fx_dirty_since: Option<std::time::Instant>,
    /// Effect node MIDI learn: waiting for a MIDI event to bind to this node.
    midi_learn_node: Option<u64>,
    /// Tab document (loaded tab notation).
    tab_document: Option<rifflab_tab::model::TabDocument>,
    /// Tab view state (scroll, zoom, selection).
    tab_view_state: tab_view::TabViewState,
    /// MIDI CC numbers for physical knobs. Starts with MiniLab 3 defaults, can be re-learned.
    midi_knob_ccs: Vec<u8>,
    /// When true, next incoming CCs teach knob slots sequentially.
    midi_knob_learning: bool,
    /// Last raw CC value per knob CC (for endless encoder delta computation).
    midi_knob_last: std::collections::HashMap<u8, u8>,
    /// MIDI CC numbers for physical faders. Maps to track volumes + master.
    midi_fader_ccs: Vec<u8>,
    /// When true, next incoming CCs teach fader slots sequentially.
    midi_fader_learning: bool,
}

impl RiffLabApp {
    fn new(
        engine: Arc<Mutex<AudioEngine>>,
        meter_rx: rifflab_core::rtrb::Consumer<MeterData>,
        pitch_rx: rifflab_core::rtrb::Consumer<PitchFrame>,
        file_path: Option<String>,
        waveform_overviews: Vec<WaveformOverview>,
        reference_notes: Vec<NoteEvent>,
        sample_rate: u32,
        buffer_size: usize,
        app_config: AppConfig,
    ) -> Self {
        let file_name = file_path
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No file loaded")
            .to_string();

        // If reference notes were transcribed, set up the comparator
        let comparator = if !reference_notes.is_empty() {
            log::info!(
                "Loaded {} reference notes into comparator",
                reference_notes.len()
            );
            Some(Comparator::new(reference_notes.clone()))
        } else {
            None
        };

        Self {
            engine,
            meter_rx,
            pitch_rx,
            file_name,
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
            current_pitch: PitchFrame::default(),
            tuner_note: 0,
            tuner_cents: 0.0,
            tuner_confidence: 0.0,
            tuner_hold_count: 0,
            tuner_last_active: std::time::Instant::now(),
            tuner_settings_open: false,
            tuner_min_confidence: 0.4,
            tuner_noise_floor: 0.02,
            tuner_hold_frames: 2,
            tuner_timeout_ms: 800,
            comparator,
            scorer: SessionScorer::new(),
            waveform_overviews,
            frames_per_pixel: DEFAULT_FRAMES_PER_PIXEL,
            scroll_offset_frames: 0.0,
            auto_follow: true,
            sample_rate,
            buffer_size,
            loop_drag: LoopDragState::default(),
            drawer_open: true,
            drawer_height: DEFAULT_DRAWER_HEIGHT,
            active_tab: BottomTab::Presets,
            reference_notes,
            played_notes: Vec::new(),
            piano_roll_scroll_note: 48.0, // C3 at bottom
            tracking_midi_note: 0,
            tracking_was_silent: true,
            pending_file: file_path.map(std::path::PathBuf::from),
            audio_settings: AudioSettings::new(&app_config),
            app_config,
            file_load_state: FileLoadState::Idle,
            message_log: MessageLog::new(),
            sidebar_snapshot: None,
            fx_registry: EffectRegistry::new(),
            fx_add_open: false,
            fx_snapshot: Vec::new(),
            fx_mb_snapshot: None,
            fx_multiband_mode: false,
            fx_snapshot_time: std::time::Instant::now(),
            fx_snapshot_dirty: true,
            fx_preset_path: None,
            fx_preset_name: "Untitled".to_string(),
            session_path: None,
            original_file_path: None,
            save_receiver: None,
            spectrograms: Vec::new(),
            spectrogram_texture: None,
            spectrogram_cache_key: (0, 0, 0, 0, 0),
            spectrogram_pending: None,
            spectrogram_pending_key: (0, 0, 0, 0, 0),
            arrangement_view: ArrangementView::Waveform,
            cue_engine: CueEngine::new(sample_rate),
            fx_graph: node_editor::FxGraph::new_default(),
            node_editor_state: node_editor::NodeEditorState::default(),
            fx_graph_compiled_hash: 0,
            midi_connection: None,
            midi_rx: None,
            midi_port_names: {
                let ports = midi_input::list_midi_ports();
                log::info!("MIDI ports found: {:?}", ports);
                ports
            },
            midi_selected_port: 0,
            preset_bank: preset_bank::PresetBank::new(),
            midi_panel_open: false,
            preset_nav: preset_graph::PresetGraph::default(),
            midi_learn: preset_graph::MidiLearnTarget::None,
            editing_preset_id: None,
            is_recording: false,
            stem_graphs: Vec::new(),
            editing_track_fx: None,
            track_fx_dirty_since: None,
            midi_learn_node: None,
            tab_document: None,
            tab_view_state: tab_view::TabViewState::default(),
            midi_knob_ccs: MINILAB3_KNOB_CCS.to_vec(),
            midi_knob_learning: false,
            midi_knob_last: std::collections::HashMap::new(),
            midi_fader_ccs: MINILAB3_FADER_CCS.to_vec(),
            midi_fader_learning: false,
        }
    }

    /// Load a reference note sequence for practice comparison.
    #[allow(dead_code)]
    pub fn load_reference(&mut self, reference: Vec<NoteEvent>) {
        self.reference_notes = reference.clone();
        self.comparator = Some(Comparator::new(reference));
        self.scorer = SessionScorer::new();
        self.played_notes.clear();
    }

    /// Convert a frame position to an x coordinate relative to the arrangement rect.
    fn frame_to_x(&self, frame: u64) -> f64 {
        (frame as f64 - self.scroll_offset_frames) / self.frames_per_pixel
    }

    /// Convert an x coordinate (relative to arrangement rect) to a frame position.
    fn x_to_frame(&self, x: f64) -> u64 {
        let frame = x * self.frames_per_pixel + self.scroll_offset_frames;
        frame.max(0.0) as u64
    }

    /// Start loading an audio file in the background.
    fn load_file(&mut self, path: &std::path::Path) {
        log::info!("Loading file: {}", path.display());

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        let target_rate = {
            let eng = self.engine.lock().unwrap();
            eng.transport().sample_rate()
        };

        let path_buf = path.to_path_buf();
        let name_clone = file_name.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let _ = tx.send(LoadMsg::Status(format!("Decoding {}...", name_clone)));

            let decoded = match decode::decode_file(&path_buf) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(LoadMsg::Error(format!("Decode failed: {e}")));
                    return;
                }
            };

            let src_rate = decoded.sample_rate;
            let duration_secs = decoded.frames as f64 / decoded.sample_rate as f64;
            let _ = tx.send(LoadMsg::Status(format!(
                "Decoded: {:.1}s, {}ch, {}Hz",
                duration_secs, decoded.channels, src_rate,
            )));

            // Try stem separation with demucs
            let _ = tx.send(LoadMsg::Status("Separating stems with Demucs (GPU)...".into()));
            let stem_dir = std::env::temp_dir().join(format!("rifflab_stems_{}", std::process::id()));
            let _ = std::fs::create_dir_all(&stem_dir);

            // Find the run_demucs.py script relative to the binary
            let worker_script = {
                let exe = std::env::current_exe().unwrap_or_default();
                let workspace_root = exe.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
                    .unwrap_or_else(|| std::path::Path::new("."));
                workspace_root.join("workers").join("run_demucs.py")
            };

            let demucs_out_dir = stem_dir.join("stems");

            // Run demucs with streaming stdout to status bar
            let demucs_success = {
                use std::io::BufRead;
                let mut child = match std::process::Command::new("python3")
                    .arg(&worker_script)
                    .arg(&path_buf)
                    .arg(&demucs_out_dir)
                    .arg("htdemucs_ft")
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(LoadMsg::Status(format!("Demucs spawn failed: {e}")));
                        // Fall through to single-track loading
                        let _ = tx.send(LoadMsg::Status("Loading as single track...".into()));
                        let decoded_rs = if src_rate != target_rate {
                            decode::resample(decoded, target_rate)
                        } else {
                            decoded
                        };
                        let overview = WaveformOverview::from_interleaved(
                            &decoded_rs.data, decoded_rs.channels,
                            OVERVIEW_SAMPLES_PER_PEAK, TRACK_COLORS[0], name_clone.clone(),
                        );
                        let _ = tx.send(LoadMsg::Done(LoadedFile {
                            file_name: name_clone,
                            stems: vec![StemTrack {
                                stem_type: StemType::Other, decoded: decoded_rs, overview,
                                spectrogram: rifflab_analysis::spectrogram::SpectrogramResult {
                                    magnitudes: Vec::new(), num_bins: 0, num_columns: 0, hop_size: 512, fft_size: 2048,
                                },
                            }],
                        }));
                        return;
                    }
                };

                // Stream stdout lines as status messages
                if let Some(stdout) = child.stdout.take() {
                    let reader = std::io::BufReader::new(stdout);
                    let tx2 = tx.clone();
                    std::thread::spawn(move || {
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                if !line.trim().is_empty() {
                                    let _ = tx2.send(LoadMsg::Status(format!("[demucs] {}", line.trim())));
                                }
                            }
                        }
                    });
                }
                // Stream stderr lines as error messages
                if let Some(stderr) = child.stderr.take() {
                    let reader = std::io::BufReader::new(stderr);
                    let tx3 = tx.clone();
                    std::thread::spawn(move || {
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() && !trimmed.contains("UserWarning") {
                                    let _ = tx3.send(LoadMsg::Status(format!("[demucs] {}", trimmed)));
                                }
                            }
                        }
                    });
                }

                match child.wait() {
                    Ok(status) => status.success(),
                    Err(e) => {
                        let _ = tx.send(LoadMsg::Status(format!("Demucs wait failed: {e}")));
                        false
                    }
                }
            };

            let stem_names = ["vocals", "drums", "bass", "other"];
            let stem_types = [StemType::Vocals, StemType::Drums, StemType::Bass, StemType::Other];

            let mut stems: Vec<StemTrack> = Vec::new();

            if demucs_success && demucs_out_dir.exists() {
                let _ = tx.send(LoadMsg::Status("Loading separated stems...".into()));

                for (i, (stem_name, stem_type)) in stem_names.iter().zip(stem_types.iter()).enumerate() {
                    let stem_path = demucs_out_dir.join(format!("{}.wav", stem_name));
                    if stem_path.exists() {
                        match decode::decode_file(&stem_path) {
                            Ok(stem_decoded) => {
                                let stem_decoded = if stem_decoded.sample_rate != target_rate {
                                    decode::resample(stem_decoded, target_rate)
                                } else {
                                    stem_decoded
                                };

                                let color = TRACK_COLORS[i % TRACK_COLORS.len()];
                                let overview = WaveformOverview::from_interleaved(
                                    &stem_decoded.data,
                                    stem_decoded.channels,
                                    OVERVIEW_SAMPLES_PER_PEAK,
                                    color,
                                    stem_name.to_string(),
                                );
                                let mono = rifflab_analysis::spectrogram::downmix_to_mono(
                                    &stem_decoded.data, stem_decoded.channels,
                                );
                                let spec = rifflab_analysis::spectrogram::compute_spectrogram(
                                    &mono, stem_decoded.sample_rate,
                                );
                                stems.push(StemTrack {
                                    stem_type: *stem_type,
                                    decoded: stem_decoded,
                                    overview,
                                    spectrogram: spec,
                                });
                                let _ = tx.send(LoadMsg::Status(format!("Loaded stem: {}", stem_name)));
                            }
                            Err(e) => {
                                log::warn!("Failed to decode stem {}: {e}", stem_name);
                            }
                        }
                    }
                }

                // Clean up temp files
                let _ = std::fs::remove_dir_all(&stem_dir);
            }

            // If demucs failed or produced no stems, fall back to single track
            if stems.is_empty() {
                if !demucs_success {
                    let _ = tx.send(LoadMsg::Status("Demucs failed, loading as single track...".into()));
                } else {
                    let _ = tx.send(LoadMsg::Status("No stems found, loading as single track...".into()));
                }

                let decoded = if src_rate != target_rate {
                    decode::resample(decoded, target_rate)
                } else {
                    decoded
                };
                let overview = WaveformOverview::from_interleaved(
                    &decoded.data,
                    decoded.channels,
                    OVERVIEW_SAMPLES_PER_PEAK,
                    TRACK_COLORS[0],
                    name_clone.clone(),
                );
                let mono = rifflab_analysis::spectrogram::downmix_to_mono(
                    &decoded.data, decoded.channels,
                );
                let spec = rifflab_analysis::spectrogram::compute_spectrogram(
                    &mono, decoded.sample_rate,
                );
                stems.push(StemTrack {
                    stem_type: StemType::Other,
                    decoded,
                    overview,
                    spectrogram: spec,
                });
            }

            let _ = tx.send(LoadMsg::Done(LoadedFile {
                file_name: name_clone,
                stems,
            }));
        });

        self.file_name = format!("Loading {}...", file_name);
        self.message_log.clear();
        self.message_log.push(format!("Opening {}...", file_name), false);
        self.original_file_path = Some(path.to_path_buf());
        self.session_path = None; // new file = no saved session yet
        self.file_load_state = FileLoadState::Loading {
            file_name,
            receiver: rx,
        };
    }

    /// Check if background loading sent any messages, and apply them.
    fn poll_file_load(&mut self) {
        let state = std::mem::replace(&mut self.file_load_state, FileLoadState::Idle);
        match state {
            FileLoadState::Idle => {}
            FileLoadState::Loading { file_name, receiver } => {
                // Drain all available messages
                let mut keep_loading = true;
                loop {
                    match receiver.try_recv() {
                        Ok(LoadMsg::Status(msg)) => {
                            log::info!("[load] {msg}");
                            self.message_log.push(msg, false);
                        }
                        Ok(LoadMsg::Done(loaded)) => {
                            let num_stems = loaded.stems.len();
                            let load_msg = {
                                let mut eng = self.engine.lock().unwrap();
                                eng.transport_mut().stop();

                                let mut players = Vec::new();
                                let mut max_frames: u64 = 0;
                                for stem in &loaded.stems {
                                    let player = StemPlayer::new(
                                        stem.stem_type,
                                        stem.decoded.data.clone(),
                                        stem.decoded.channels,
                                    );
                                    max_frames = max_frames.max(player.total_frames());
                                    players.push(player);
                                }

                                eng.graph().lock().unwrap().load_stems(players);
                                eng.transport().set_length(max_frames);

                                let first = &loaded.stems[0];
                                format!(
                                    "Loaded: {} ({} stems, {:.1}s, {}Hz)",
                                    loaded.file_name,
                                    num_stems,
                                    max_frames as f64 / first.decoded.sample_rate as f64,
                                    first.decoded.sample_rate,
                                )
                            };

                            self.file_name = loaded.file_name;
                            let mut overviews = Vec::new();
                            let mut specs = Vec::new();
                            for (i, stem) in loaded.stems.into_iter().enumerate() {
                                let color = TRACK_COLORS[i % TRACK_COLORS.len()];
                                specs.push(SpectrogramDisplay {
                                    data: stem.spectrogram,
                                    color,
                                    name: stem.overview.name.clone(),
                                });
                                overviews.push(stem.overview);
                            }
                            self.waveform_overviews = overviews;
                            self.spectrograms = specs;
                            self.spectrogram_texture = None; // invalidate cache

                            self.comparator = None;
                            self.reference_notes.clear();
                            self.scorer = SessionScorer::new();
                            self.played_notes.clear();
                            self.scroll_offset_frames = 0.0;

                            log::info!("{load_msg}");
                            self.message_log.push(load_msg, false);
                            keep_loading = false;
                            break;
                        }
                        Ok(LoadMsg::Error(e)) => {
                            log::error!("File load error: {e}");
                            self.file_name = "No file loaded".to_string();
                            self.message_log.push(e, true);
                            keep_loading = false;
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            if keep_loading {
                                self.file_name = "No file loaded".to_string();
                                self.message_log.push("File loader thread crashed".into(), true);
                            }
                            keep_loading = false;
                            break;
                        }
                    }
                }
                if keep_loading {
                    self.file_load_state = FileLoadState::Loading { file_name, receiver };
                }
            }
        }
    }
}

impl eframe::App for RiffLabApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ─── Load pending file (from CLI arg, first frame only) ──
        if let Some(path) = self.pending_file.take() {
            self.load_file(&path);
        }

        // ─── Poll background file loading ────────────────────────
        self.poll_file_load();

        // ─── Poll background save ────────────────────────────────
        if let Some(ref rx) = self.save_receiver {
            match rx.try_recv() {
                Ok((is_error, msg)) => {
                    self.message_log.push(msg, is_error);
                    self.save_receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.save_receiver = None;
                }
                _ => {} // still saving
            }
        }

        // ─── Drain channels ──────────────────────────────────────
        while let Ok(meter) = self.meter_rx.pop() {
            // Smooth with exponential moving average
            self.peak_l = self.peak_l * 0.85 + meter.peak_l * 0.15;
            self.peak_r = self.peak_r * 0.85 + meter.peak_r * 0.15;
            self.rms_l = self.rms_l * 0.85 + meter.rms_l * 0.15;
            self.rms_r = self.rms_r * 0.85 + meter.rms_r * 0.15;
        }

        // Read transport state
        let (state, position, _total_length, is_running, loop_enabled, loop_start, loop_end) = {
            let eng = self.engine.lock().unwrap();
            let transport = eng.transport();
            (
                transport.state(),
                transport.position(),
                transport.length(),
                eng.is_running(),
                transport.loop_enabled(),
                transport.loop_start(),
                transport.loop_end(),
            )
        };
        let position_secs = position.seconds();
        let position_frame = position.frame;

        // ─── Tick cue engine ─────────────────────────────────────
        if state == TransportState::Playing {
            let actions = self.cue_engine.tick(position_frame);
            for action in actions {
                match action {
                    DispatchedAction::SetLoop(region) => {
                        let mut eng = self.engine.lock().unwrap();
                        eng.transport_mut().set_loop(Some(region));
                    }
                    DispatchedAction::ClearLoop => {
                        let mut eng = self.engine.lock().unwrap();
                        eng.transport_mut().set_loop(None);
                    }
                    DispatchedAction::SwitchPreset(name) => {
                        log::info!("Cue: switch preset to '{}'", name);
                        // TODO: load preset by name and apply to fx_chain
                    }
                }
            }
        }

        // ─── Poll MIDI input ──────────────────────────────────────
        // ─── Poll MIDI: learn bindings or activate presets ─────
        {
            let mut nav_activate: Option<u64> = None;
            let mut bank_activate: Option<usize> = None;
            let mut knob_changes: Vec<node_editor::NodeParamChange> = Vec::new();
            if let Some(ref rx) = self.midi_rx {
                while let Ok(event) = rx.try_recv() {
                    // MIDI Learn mode: capture binding
                    if self.midi_learn != preset_graph::MidiLearnTarget::None {
                        let binding = match &event {
                            midi_input::MidiEvent::ControlChange { channel, cc, value } if *value > 0 => {
                                Some(preset_graph::MidiBinding::ControlChange { channel: *channel, cc: *cc })
                            }
                            midi_input::MidiEvent::NoteOn { channel, note, .. } => {
                                Some(preset_graph::MidiBinding::NoteOn { channel: *channel, note: *note })
                            }
                            midi_input::MidiEvent::ProgramChange { channel, program } => {
                                Some(preset_graph::MidiBinding::ProgramChange { channel: *channel, program: *program })
                            }
                            _ => None,
                        };
                        if let Some(b) = binding {
                            match &self.midi_learn {
                                preset_graph::MidiLearnTarget::PresetNode(id) => {
                                    let id = *id;
                                    if let Some(node) = self.preset_nav.find_node_mut(id) {
                                        node.midi_binding = Some(b.clone());
                                    }
                                    self.message_log.push(format!("Bound {} to preset", b.label()), false);
                                }
                                preset_graph::MidiLearnTarget::GlobalNext => {
                                    self.message_log.push(format!("Next bound to {}", b.label()), false);
                                    self.preset_nav.midi_next = Some(b.clone());
                                }
                                preset_graph::MidiLearnTarget::GlobalPrev => {
                                    self.message_log.push(format!("Prev bound to {}", b.label()), false);
                                    self.preset_nav.midi_prev = Some(b.clone());
                                }
                                _ => {}
                            }
                            self.midi_learn = preset_graph::MidiLearnTarget::None;
                        }
                        continue;
                    }

                    // Knob learning: capture CC numbers sequentially
                    if self.midi_knob_learning {
                        if let midi_input::MidiEvent::ControlChange { cc, .. } = &event {
                            if !self.midi_knob_ccs.contains(cc) {
                                // Remove from fader list if it was there
                                self.midi_fader_ccs.retain(|&c| c != *cc);
                                self.midi_knob_ccs.push(*cc);
                                self.message_log.push(
                                    format!("Knob {} → CC#{} (turn next or click Done)", self.midi_knob_ccs.len(), cc),
                                    false,
                                );
                            }
                        }
                        continue;
                    }

                    // Fader learning: capture fader CC numbers sequentially
                    if self.midi_fader_learning {
                        if let midi_input::MidiEvent::ControlChange { cc, .. } = &event {
                            if !self.midi_fader_ccs.contains(cc) {
                                // Remove from knob list if it was there
                                self.midi_knob_ccs.retain(|&c| c != *cc);
                                self.midi_fader_ccs.push(*cc);
                                self.message_log.push(
                                    format!("Fader {} → CC#{} (move next or click Done)", self.midi_fader_ccs.len(), cc),
                                    false,
                                );
                            }
                        }
                        continue;
                    }

                    // Faders → track volumes + input + master (absolute, 0-127)
                    // Layout: [track1, track2, ..., input, master]
                    if let midi_input::MidiEvent::ControlChange { cc, value, .. } = &event {
                        if let Some(fader_idx) = self.midi_fader_ccs.iter().position(|&c| c == *cc) {
                            let graph_arc = self.engine.lock().unwrap().graph().clone();
                            if let Ok(mut g) = graph_arc.try_lock() {
                                let n = self.midi_fader_ccs.len();
                                if n >= 1 && fader_idx == n - 1 {
                                    // Last fader → master volume (0.0–1.0)
                                    g.master_volume = *value as f32 / 127.0;
                                } else if n >= 2 && fader_idx == n - 2 {
                                    // Second-to-last → input volume (0.0–2.0, matching UI slider)
                                    g.input_volume = *value as f32 / 127.0 * 2.0;
                                } else {
                                    // Rest → stem track volumes (0.0–1.0)
                                    if fader_idx < g.stem_volumes.len() {
                                        g.stem_volumes[fader_idx] = *value as f32 / 127.0;
                                    }
                                }
                            }
                            continue;
                        }
                    }

                    // Effect node MIDI learn: bind a MIDI key/CC to select a node
                    if let Some(learn_id) = self.midi_learn_node {
                        let binding = match &event {
                            midi_input::MidiEvent::ControlChange { channel, cc, value } if *value > 0 && !self.midi_knob_ccs.contains(cc) && !self.midi_fader_ccs.contains(cc) => {
                                Some(preset_graph::MidiBinding::ControlChange { channel: *channel, cc: *cc })
                            }
                            midi_input::MidiEvent::NoteOn { channel, note, .. } => {
                                Some(preset_graph::MidiBinding::NoteOn { channel: *channel, note: *note })
                            }
                            midi_input::MidiEvent::ProgramChange { channel, program } => {
                                Some(preset_graph::MidiBinding::ProgramChange { channel: *channel, program: *program })
                            }
                            _ => None,
                        };
                        if let Some(b) = binding {
                            if let Some(node) = self.fx_graph.find_node_mut(learn_id) {
                                self.message_log.push(format!("{} → {}", node.label, b.label()), false);
                                node.midi_binding = Some(b);
                            }
                            self.midi_learn_node = None;
                        }
                        continue;
                    }

                    // MIDI knobs → selected effect node params (endless encoder delta mode)
                    if let midi_input::MidiEvent::ControlChange { cc, value, .. } = &event {
                        if let Some(knob_idx) = self.midi_knob_ccs.iter().position(|&c| c == *cc) {
                            // Compute delta from last raw CC value (endless encoder)
                            let last = self.midi_knob_last.get(cc).copied();
                            self.midi_knob_last.insert(*cc, *value);
                            let delta = if let Some(prev) = last {
                                let d = *value as i16 - prev as i16;
                                // Handle wrap-around: if |delta| > 64, encoder wrapped
                                if d > 64 { d - 128 } else if d < -64 { d + 128 } else { d }
                            } else {
                                0 // First touch: just record position, don't jump
                            };
                            if delta != 0 {
                                if let Some(sel_node_id) = self.node_editor_state.selected_node {
                                    if let Some(node) = self.fx_graph.find_node(sel_node_id) {
                                        if let node_editor::NodeKind::Effect { type_id } = &node.kind {
                                            if let Some(effect) = self.fx_registry.create_effect(type_id) {
                                                let descs = effect.param_descriptors();
                                                if let Some(desc) = descs.get(knob_idx) {
                                                    // Scale: full knob sweep (0-127) = full param range
                                                    let range = desc.max - desc.min;
                                                    let step = range / 127.0 * delta as f32;
                                                    let current = self.node_editor_state.param_cache
                                                        .get(&(sel_node_id, desc.id.0))
                                                        .copied()
                                                        .unwrap_or(desc.default);
                                                    let val = (current + step).clamp(desc.min, desc.max);
                                                    self.node_editor_state.param_cache.insert((sel_node_id, desc.id.0), val);
                                                    knob_changes.push(node_editor::NodeParamChange {
                                                        node_id: sel_node_id,
                                                        param_id: rifflab_core::audio::ParamId(desc.id.0),
                                                        value: val,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                    }

                    // Check if MIDI event matches any effect node's binding → select that node
                    {
                        let mut matched_node: Option<(u64, String)> = None;
                        for node in &self.fx_graph.nodes {
                            if let Some(ref binding) = node.midi_binding {
                                if binding.matches_event(&event) {
                                    matched_node = Some((node.id, node.label.clone()));
                                    break;
                                }
                            }
                        }
                        if let Some((id, label)) = matched_node {
                            self.node_editor_state.selected_node = Some(id);
                            self.message_log.push(format!("Knobs → {}", label), false);
                            continue;
                        }
                    }

                    // Normal mode: check bindings
                    // 1. Direct preset binding
                    if let Some(id) = self.preset_nav.find_by_midi(&event) {
                        nav_activate = Some(id);
                    }
                    // 2. Next/Prev navigation
                    else if let Some(ref next_bind) = self.preset_nav.midi_next {
                        if next_bind.matches_event(&event) {
                            if let Some(active) = self.preset_nav.active_id {
                                if let Some(next_id) = self.preset_nav.next_from(active) {
                                    nav_activate = Some(next_id);
                                }
                            }
                        }
                    }
                    if nav_activate.is_none() {
                        if let Some(ref prev_bind) = self.preset_nav.midi_prev {
                            if prev_bind.matches_event(&event) {
                                if let Some(active) = self.preset_nav.active_id {
                                    if let Some(prev_id) = self.preset_nav.prev_from(active) {
                                        nav_activate = Some(prev_id);
                                    }
                                }
                            }
                        }
                    }
                    // 3. Legacy preset bank
                    if nav_activate.is_none() {
                        if let midi_input::MidiEvent::ControlChange { cc, value, .. } = &event {
                            if *value > 0 {
                                if let Some(idx) = self.preset_bank.find_by_cc(*cc) {
                                    bank_activate = Some(idx);
                                }
                            }
                        }
                    }
                }
            }
            if let Some(id) = nav_activate {
                self.activate_nav_preset(id);
            }
            if let Some(idx) = bank_activate {
                self.activate_preset(idx);
            }
            if !knob_changes.is_empty() {
                self.apply_node_param_changes(&knob_changes);
            }
        }

        // Drain pitch frames, run comparison, update score, track played notes
        while let Ok(pitch) = self.pitch_rx.pop() {
            self.current_pitch = pitch.clone();

            // Tuner smoothing with configurable parameters
            if pitch.frequency_hz > 0.0 && pitch.confidence > self.tuner_min_confidence {
                let new_note = pitch.midi_note;
                if new_note == self.tuner_note {
                    self.tuner_cents = self.tuner_cents * 0.5 + pitch.cents_deviation * 0.5;
                    self.tuner_confidence = self.tuner_confidence * 0.5 + pitch.confidence * 0.5;
                    self.tuner_hold_count = 0;
                } else {
                    self.tuner_hold_count += 1;
                    if self.tuner_hold_count >= self.tuner_hold_frames || self.tuner_note == 0 {
                        self.tuner_note = new_note;
                        self.tuner_cents = pitch.cents_deviation;
                        self.tuner_confidence = pitch.confidence;
                        self.tuner_hold_count = 0;
                    }
                }
                self.tuner_last_active = std::time::Instant::now();
            } else {
                if self.tuner_last_active.elapsed()
                    > std::time::Duration::from_millis(self.tuner_timeout_ms as u64)
                {
                    self.tuner_note = 0;
                    self.tuner_cents = 0.0;
                    self.tuner_confidence = 0.0;
                }
            }

            // Track played notes for piano roll display
            let is_silent = pitch.frequency_hz <= 0.0
                || pitch.confidence < PITCH_CONFIDENCE_THRESHOLD;
            let current_time = position_secs;

            if is_silent {
                // Close any active note
                self.tracking_was_silent = true;
                self.tracking_midi_note = 0;
            } else {
                let is_new_note =
                    self.tracking_was_silent || pitch.midi_note != self.tracking_midi_note;
                if is_new_note {
                    // Determine accuracy from comparison or raw cents
                    let accuracy = if pitch.cents_deviation.abs() <= 5.0 {
                        AccuracyBucket::Perfect
                    } else if pitch.cents_deviation.abs() <= 15.0 {
                        AccuracyBucket::Good
                    } else if pitch.cents_deviation.abs() <= 25.0 {
                        AccuracyBucket::Acceptable
                    } else {
                        AccuracyBucket::Off
                    };
                    self.played_notes.push(PlayedNote {
                        midi_note: pitch.midi_note,
                        start_seconds: current_time,
                        end_seconds: current_time,
                        accuracy,
                    });
                    self.tracking_midi_note = pitch.midi_note;
                    self.tracking_was_silent = false;
                } else if let Some(last) = self.played_notes.last_mut() {
                    // Extend the current note
                    last.end_seconds = current_time;
                }
            }

            if let Some(ref mut comparator) = self.comparator {
                let comparison = comparator.compare(&pitch, &position);
                // Update the accuracy of the current played note from comparison data
                if comparison.reference_note.is_some() {
                    if let Some(last) = self.played_notes.last_mut() {
                        if !comparison.note_correct {
                            last.accuracy = AccuracyBucket::Off;
                        } else {
                            last.accuracy = comparison.accuracy_bucket;
                        }
                    }
                }
                self.scorer.feed(comparison);
            }
        }

        // ─── Toolbar ─────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar")
            .exact_height(36.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    ui.heading("RiffLab");
                    ui.separator();

                    // Open file button (disabled while loading)
                    let is_loading = matches!(self.file_load_state, FileLoadState::Loading { .. });
                    let open_label = if is_loading { "\u{231B} Loading..." } else { "\u{1F4C2} Open" };
                    if ui.add_enabled(!is_loading, egui::Button::new(open_label)).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac", "mp3", "ogg", "aac", "m4a"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            self.load_file(&path);
                        }
                    }

                    // Save session
                    let has_stems = !self.waveform_overviews.is_empty();
                    let is_saving = self.save_receiver.is_some();
                    if has_stems {
                        if self.session_path.is_some() {
                            if ui.add_enabled(!is_saving, egui::Button::new("Save"))
                                .on_hover_text("Save session (overwrite)").clicked() {
                                self.save_current_session(false);
                            }
                        }
                        let save_as_label = if is_saving { "Saving..." } else { "Save As" };
                        if ui.add_enabled(!is_saving, egui::Button::new(save_as_label))
                            .on_hover_text("Save session to new folder").clicked() {
                            self.save_current_session(true);
                        }
                    }

                    // Load raw audio (no stem separation)
                    if ui.small_button("Load Raw").on_hover_text("Load audio without stem separation").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac", "mp3", "ogg", "aac", "m4a"])
                            .pick_file()
                        {
                            self.load_raw_audio(&path);
                        }
                    }

                    // Open session
                    if ui.small_button("Open Session").on_hover_text("Open a saved session").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.open_session(&dir);
                        }
                    }

                    ui.separator();

                    // Transport controls
                    let mut do_stop_rec = false;
                    let mut do_start_rec = false;
                    {
                        let mut eng = self.engine.lock().unwrap();

                        let play_label = if state == TransportState::Playing {
                            "\u{23F8} Pause"
                        } else {
                            "\u{23F5} Play"
                        };
                        if ui.button(play_label).clicked() {
                            if state == TransportState::Playing {
                                eng.transport_mut().pause();
                            } else {
                                eng.transport_mut().play();
                            }
                        }

                        if ui.button("\u{23F9} Stop").clicked() {
                            eng.transport_mut().stop();
                            if self.is_recording {
                                do_stop_rec = true;
                            }
                        }

                        // Record button
                        let rec_label = if self.is_recording {
                            egui::RichText::new("\u{23FA} Rec").color(egui::Color32::from_rgb(220, 50, 50))
                        } else {
                            egui::RichText::new("\u{23FA} Rec").color(egui::Color32::from_rgb(160, 160, 160))
                        };
                        if ui.button(rec_label).clicked() {
                            if self.is_recording {
                                do_stop_rec = true;
                            } else {
                                do_start_rec = true;
                            }
                        }
                    } // eng dropped

                    if do_stop_rec { self.stop_recording(); }
                    if do_start_rec { self.start_recording(); }

                    ui.separator();

                    // File name
                    ui.label(
                        egui::RichText::new(&self.file_name)
                            .color(egui::Color32::from_rgb(180, 200, 220)),
                    );

                    ui.separator();

                    // Position display (mm:ss.f)
                    let mins = (position_secs / 60.0) as u32;
                    let secs = position_secs % 60.0;
                    ui.monospace(
                        egui::RichText::new(format!("{:02}:{:05.2}", mins, secs))
                            .color(egui::Color32::from_rgb(120, 220, 180))
                            .size(15.0),
                    );

                    ui.separator();

                    // Visual tuner indicator (uses smoothed values)
                    {
                        let has_pitch = self.tuner_note > 0;
                        let note = if has_pitch {
                            let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
                            let octave = (self.tuner_note as i32 / 12) - 1;
                            let idx = (self.tuner_note % 12) as usize;
                            format!("{}{}", names[idx], octave)
                        } else {
                            "--".to_string()
                        };
                        let cents = self.tuner_cents;
                        let tuner_color = if has_pitch {
                            if cents.abs() <= 5.0 {
                                egui::Color32::from_rgb(80, 220, 80)
                            } else if cents.abs() <= 15.0 {
                                egui::Color32::from_rgb(220, 200, 60)
                            } else {
                                egui::Color32::from_rgb(220, 70, 70)
                            }
                        } else {
                            egui::Color32::from_rgb(80, 80, 80)
                        };

                        // Note name (fixed width, clickable to open tuner settings)
                        // Pad to 4 chars so "A4" and "C#4" take the same space.
                        let padded_note = format!("{:<4}", note);
                        let note_resp = ui.add_sized(
                            egui::vec2(44.0, 20.0),
                            egui::Label::new(
                                egui::RichText::new(padded_note)
                                    .size(16.0)
                                    .color(tuner_color)
                                    .family(egui::FontFamily::Monospace),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if note_resp.clicked() {
                            self.tuner_settings_open = !self.tuner_settings_open;
                        }
                        note_resp.on_hover_text("Click to open tuner settings");

                        // Cents deviation bar
                        let bar_width = 80.0f32;
                        let bar_height = 8.0f32;
                        let (bar_rect, _) = ui.allocate_exact_size(
                            egui::vec2(bar_width, bar_height),
                            egui::Sense::hover(),
                        );
                        let painter = ui.painter_at(bar_rect);
                        // Bar background
                        painter.rect_filled(
                            bar_rect,
                            2.0,
                            egui::Color32::from_rgb(30, 32, 36),
                        );
                        // Center tick
                        let center_x = bar_rect.center().x;
                        painter.line_segment(
                            [
                                egui::pos2(center_x, bar_rect.top()),
                                egui::pos2(center_x, bar_rect.bottom()),
                            ],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 85, 90)),
                        );
                        if has_pitch {
                            // Deviation indicator: map -50..+50 cents to bar
                            let norm = (cents / 50.0).clamp(-1.0, 1.0);
                            let ind_x = center_x + norm * (bar_width / 2.0);
                            let ind_w = 4.0f32;
                            let ind_rect = egui::Rect::from_center_size(
                                egui::pos2(ind_x, bar_rect.center().y),
                                egui::vec2(ind_w, bar_height),
                            );
                            painter.rect_filled(ind_rect, 1.0, tuner_color);
                        }

                        // Cents text (fixed width)
                        let cents_str = if has_pitch {
                            if cents >= 0.0 {
                                format!("+{:.0}c", cents)
                            } else {
                                format!("{:.0}c", cents)
                            }
                        } else {
                            "   ".to_string()
                        };
                        ui.add_sized(
                            egui::vec2(32.0, 14.0),
                            egui::Label::new(
                                egui::RichText::new(cents_str)
                                    .size(10.0)
                                    .color(tuner_color)
                                    .family(egui::FontFamily::Monospace),
                            ),
                        );
                    }

                    ui.separator();

                    // Score display
                    ui.label(format!(
                        "Score: {:.0} | {}/{}",
                        self.scorer.score(),
                        self.scorer.notes_correct(),
                        self.scorer.notes_total(),
                    ));

                    // Right-aligned controls
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // MIDI / Preset Bank button
                        if ui.small_button("\u{1F3B9} MIDI").clicked() {
                            self.midi_panel_open = !self.midi_panel_open;
                        }
                        // Settings gear button
                        if ui.small_button("\u{2699} Audio").clicked() {
                            self.audio_settings.refresh_devices();
                            self.audio_settings.load_from_config(&self.app_config);
                            self.audio_settings.open = true;
                        }
                        ui.separator();
                        ui.checkbox(&mut self.auto_follow, "Follow");
                        ui.separator();
                        let drawer_label = if self.drawer_open { "Hide Panel" } else { "Show Panel" };
                        if ui.small_button(drawer_label).clicked() {
                            self.drawer_open = !self.drawer_open;
                        }
                    });
                });
            });

        // ─── Status bar ──────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;

                    let status_color = egui::Color32::from_rgb(160, 170, 180);

                    // Sample rate
                    ui.label(
                        egui::RichText::new(format!("{}Hz", self.sample_rate))
                            .small()
                            .color(status_color),
                    );

                    // Buffer size + latency
                    let latency_ms =
                        self.buffer_size as f64 / self.sample_rate as f64 * 1000.0;
                    ui.label(
                        egui::RichText::new(format!(
                            "buf {} ({:.1}ms)",
                            self.buffer_size, latency_ms
                        ))
                        .small()
                        .color(status_color),
                    );

                    ui.separator();

                    if is_running {
                        // Peak dB (stereo max)
                        let peak_db = to_db(self.peak_l.max(self.peak_r));
                        let peak_color = if peak_db > -3.0 {
                            egui::Color32::from_rgb(255, 80, 80)
                        } else if peak_db > -12.0 {
                            egui::Color32::from_rgb(220, 200, 60)
                        } else {
                            egui::Color32::from_rgb(80, 200, 120)
                        };
                        ui.label(
                            egui::RichText::new(format!("Peak: {:.1} dB", peak_db))
                                .small()
                                .color(peak_color),
                        );

                        // RMS dB
                        let rms_db = to_db(self.rms_l.max(self.rms_r));
                        ui.label(
                            egui::RichText::new(format!("RMS: {:.1} dB", rms_db))
                                .small()
                                .color(status_color),
                        );

                        ui.separator();

                        // Transport state
                        let (state_label, state_color) = match state {
                            TransportState::Playing => {
                                ("Playing", egui::Color32::from_rgb(80, 220, 120))
                            }
                            TransportState::Paused => {
                                ("Paused", egui::Color32::from_rgb(220, 200, 60))
                            }
                            TransportState::Stopped => {
                                ("Stopped", egui::Color32::from_rgb(160, 160, 160))
                            }
                        };
                        ui.label(
                            egui::RichText::new(state_label)
                                .small()
                                .color(state_color),
                        );

                        // Loop indicator
                        if loop_enabled && loop_end > loop_start {
                            let ls = loop_start as f64 / self.sample_rate as f64;
                            let le = loop_end as f64 / self.sample_rate as f64;
                            ui.label(
                                egui::RichText::new(format!(
                                    "Loop: {:.1}s - {:.1}s",
                                    ls, le
                                ))
                                .small()
                                .color(egui::Color32::from_rgb(180, 120, 220)),
                            );
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("Engine not running")
                                .small()
                                .color(egui::Color32::from_rgb(220, 80, 80)),
                        );
                    }

                    // Right side: load status + zoom
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Zoom: {:.0} fr/px",
                                self.frames_per_pixel
                            ))
                            .small()
                            .color(status_color),
                        );

                        // Message log: show latest entry, click to expand full log
                        if let Some(last) = self.message_log.last() {
                            let is_active = matches!(self.file_load_state, FileLoadState::Loading { .. });
                            let age = last.timestamp.elapsed();
                            let show = last.is_error
                                || is_active
                                || age < std::time::Duration::from_secs(8);
                            if show {
                                ui.separator();
                                let color = if last.is_error {
                                    egui::Color32::from_rgb(220, 80, 80)
                                } else if is_active {
                                    egui::Color32::from_rgb(100, 180, 240)
                                } else {
                                    egui::Color32::from_rgb(80, 200, 120)
                                };
                                let count = self.message_log.entries.len();
                                let label_text = if count > 1 {
                                    format!("[{}/{}] {}", count, count, last.text)
                                } else {
                                    last.text.clone()
                                };
                                let resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&label_text).small().color(color)
                                    ).sense(egui::Sense::click()),
                                );
                                if resp.clicked() {
                                    self.message_log.open = !self.message_log.open;
                                }
                                resp.on_hover_text("Click to show full log");
                            }
                        }
                    });
                });
            });

        // ─── Bottom Drawer (Piano Roll / Accuracy / Effects) ─────
        if self.drawer_open {
            egui::TopBottomPanel::bottom("bottom_drawer")
                .resizable(true)
                .min_height(MIN_DRAWER_HEIGHT)
                .max_height(MAX_DRAWER_HEIGHT)
                .default_height(self.drawer_height)
                .show(ctx, |ui| {
                    // Tab bar
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let tabs = [
                            (BottomTab::Presets, "Presets"),
                            (BottomTab::Effects, "Effects"),
                            (BottomTab::Tab, "Tab"),
                            (BottomTab::PianoRoll, "Piano Roll"),
                            (BottomTab::Accuracy, "Accuracy"),
                        ];
                        for (tab, label) in &tabs {
                            let active = self.active_tab == *tab;
                            let btn = egui::Button::new(
                                egui::RichText::new(*label)
                                    .size(11.0)
                                    .color(if active {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_rgb(140, 145, 150)
                                    }),
                            )
                            .fill(if active {
                                egui::Color32::from_rgb(50, 55, 65)
                            } else {
                                egui::Color32::from_rgb(30, 32, 38)
                            })
                            .corner_radius(2)
                            .min_size(egui::vec2(80.0, 20.0));
                            if ui.add(btn).clicked() {
                                self.active_tab = *tab;
                            }
                        }

                        // Close button (right-aligned)
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .small_button(
                                        egui::RichText::new("X")
                                            .size(10.0)
                                            .color(egui::Color32::from_rgb(160, 160, 160)),
                                    )
                                    .clicked()
                                {
                                    self.drawer_open = false;
                                }
                            },
                        );
                    });

                    ui.separator();

                    // Tab content
                    match self.active_tab {
                        BottomTab::Presets => {
                            let action = preset_graph::draw_preset_graph(
                                ui, &mut self.preset_nav, &mut self.midi_learn,
                            );
                            match action {
                                preset_graph::PresetGraphAction::ActivatePreset(id) => {
                                    self.activate_nav_preset(id);
                                }
                                preset_graph::PresetGraphAction::EditPreset(id) => {
                                    self.editing_preset_id = Some(id);
                                    self.activate_nav_preset(id);
                                    self.active_tab = BottomTab::Effects;
                                }
                                preset_graph::PresetGraphAction::Save => {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("RiffLab Preset Graph", &["json"])
                                        .set_file_name("presets.json")
                                        .save_file()
                                    {
                                        if let Err(e) = preset_graph::save_preset_graph(&path, &self.preset_nav) {
                                            self.message_log.push(format!("Save failed: {e}"), true);
                                        } else {
                                            self.message_log.push(format!("Presets saved to {}", path.display()), false);
                                        }
                                    }
                                }
                                preset_graph::PresetGraphAction::Load => {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("RiffLab Preset/Graph", &["json"])
                                        .pick_file()
                                    {
                                        // Try as PresetGraph first, then as single FxGraph (create a preset node)
                                        match preset_graph::load_preset_graph(&path) {
                                            Ok(g) => {
                                                self.preset_nav = g;
                                                self.message_log.push(format!("Presets loaded from {}", path.display()), false);
                                            }
                                            Err(_) => {
                                                // Try as FxGraph — wrap in a new preset node
                                                match node_editor::load_graph(&path) {
                                                    Ok(graph) => {
                                                        let name = path.file_stem()
                                                            .map(|s| s.to_string_lossy().to_string())
                                                            .unwrap_or_else(|| "Loaded".into());
                                                        self.preset_nav.add_node_with_pipeline(name, graph);
                                                        self.message_log.push(format!("Pipeline loaded as preset from {}", path.display()), false);
                                                    }
                                                    Err(e) => self.message_log.push(format!("Load failed: {e}"), true),
                                                }
                                            }
                                        }
                                    }
                                }
                                preset_graph::PresetGraphAction::None => {}
                            }
                        }
                        BottomTab::Tab => {
                            if let Some(ref tab) = self.tab_document {
                                let tab_is_playing = state == TransportState::Playing;
                                let playback_secs = position_frame as f64 / self.sample_rate.max(1) as f64;
                                let tab_action = tab_view::draw_tab_view(
                                    ui, tab, playback_secs, tab_is_playing, &mut self.tab_view_state,
                                );
                                if let tab_view::TabViewAction::Seek(t) = tab_action {
                                    let frame = (t * self.sample_rate as f64) as u64;
                                    self.engine.lock().unwrap().transport_mut().seek(frame);
                                }
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(20.0);
                                    ui.label("No tab loaded");
                                    ui.add_space(10.0);
                                    if ui.button("Paste ASCII Tab").clicked() {
                                        // Will be handled by a text input dialog
                                        self.message_log.push("Paste tab text and press Enter".into(), false);
                                    }
                                    if ui.button("Load .rltab File").clicked() {
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("RiffLab Tab", &["rltab", "json"])
                                            .pick_file()
                                        {
                                            match rifflab_tab::io::load_tab(&path) {
                                                Ok(tab) => {
                                                    self.message_log.push(format!("Tab loaded: {}", tab.title), false);
                                                    self.tab_document = Some(tab);
                                                }
                                                Err(e) => self.message_log.push(format!("Load failed: {e}"), true),
                                            }
                                        }
                                    }
                                    if ui.button("Parse from Clipboard").clicked() {
                                        let clipboard_text = ui.input(|i| {
                                            i.events.iter().find_map(|e| {
                                                if let egui::Event::Paste(text) = e { Some(text.clone()) } else { None }
                                            })
                                        });
                                        if let Some(text) = clipboard_text {
                                            let notes = rifflab_tab::ascii::parse_ascii_tab(&text);
                                            if notes.is_empty() {
                                                self.message_log.push("No notes found in clipboard".into(), true);
                                            } else {
                                                let mut tab = rifflab_tab::model::TabDocument::new("Pasted Tab");
                                                tab.notes = notes;
                                                tab.generate_measures();
                                                self.message_log.push(format!("Parsed {} notes from clipboard", tab.notes.len()), false);
                                                self.tab_document = Some(tab);
                                            }
                                        } else {
                                            self.message_log.push("Ctrl+V to paste tab text first".into(), false);
                                        }
                                    }
                                });
                            }
                        }
                        BottomTab::PianoRoll => {
                            draw_piano_roll(
                                ui,
                                &self.reference_notes,
                                &self.played_notes,
                                self.scroll_offset_frames,
                                self.frames_per_pixel,
                                self.sample_rate,
                                position_frame,
                                self.piano_roll_scroll_note,
                            );
                        }
                        BottomTab::Accuracy => {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new("Accuracy view coming soon")
                                        .color(egui::Color32::from_rgb(100, 110, 120)),
                                );
                            });
                        }
                        BottomTab::Effects => {
                            // Show which context is being edited
                            if let Some(track_idx) = self.editing_track_fx {
                                ui.horizontal(|ui| {
                                    let track_name = self.waveform_overviews.get(track_idx)
                                        .map(|o| o.name.as_str()).unwrap_or("Track");
                                    ui.colored_label(egui::Color32::from_rgb(100, 200, 140),
                                        egui::RichText::new(format!("Editing: {} (Track {})", track_name, track_idx + 1)).size(11.0));
                                    if ui.small_button("Done").clicked() {
                                        self.save_track_fx_from_editor(track_idx);
                                        self.editing_track_fx = None;
                                        self.track_fx_dirty_since = None;
                                    }
                                    if ui.small_button("Back to Live").clicked() {
                                        self.save_track_fx_from_editor(track_idx);
                                        self.editing_track_fx = None;
                                        self.track_fx_dirty_since = None;
                                        // Restore live input graph
                                        self.fx_graph_compiled_hash = 0;
                                        self.compile_graph_if_changed();
                                    }
                                });
                                ui.separator();
                            }

                            // MIDI node selection toolbar
                            ui.horizontal(|ui| {
                                if let Some(sel_id) = self.node_editor_state.selected_node {
                                    if let Some(node) = self.fx_graph.find_node(sel_id) {
                                        ui.label(egui::RichText::new(
                                            format!("Knobs → {}", node.label)
                                        ).size(10.0).color(egui::Color32::from_rgb(100, 200, 255)));
                                        let binding_label = node.midi_binding.as_ref()
                                            .map(|b| b.label())
                                            .unwrap_or_else(|| "—".into());
                                        ui.label(egui::RichText::new(
                                            format!("[{}]", binding_label)
                                        ).size(10.0).color(egui::Color32::from_rgb(180, 180, 180)));
                                    }
                                    if self.midi_learn_node.is_some() {
                                        ui.colored_label(egui::Color32::from_rgb(255, 200, 80),
                                            egui::RichText::new("Press a key...").size(10.0));
                                        if ui.small_button("Cancel").clicked() {
                                            self.midi_learn_node = None;
                                        }
                                    } else if ui.small_button("Learn Select").on_hover_text("Bind a MIDI key to select this node").clicked() {
                                        self.midi_learn_node = Some(sel_id);
                                    }
                                } else {
                                    ui.label(egui::RichText::new("Click a node for knob control").size(10.0)
                                        .color(egui::Color32::from_rgb(140, 140, 140)));
                                }
                            });

                            // Compile graph FIRST so chain matches nodes
                            self.compile_graph_if_changed();
                            let effects_list = self.fx_registry.list_effects();
                            // Throttle snapshot reads to avoid lock contention with audio thread
                            let needs_snap = self.fx_snapshot_dirty
                                || self.fx_snapshot_time.elapsed() > std::time::Duration::from_millis(200);
                            if needs_snap {
                                self.fx_snapshot_time = std::time::Instant::now();
                                self.fx_snapshot_dirty = false;
                            }
                            let snapshots = if needs_snap {
                                self.build_node_param_snapshots()
                            } else {
                                Vec::new() // use param_cache for values between refreshes
                            };
                            let (changes, action) = node_editor::draw_node_editor(
                                ui,
                                &mut self.fx_graph,
                                &mut self.node_editor_state,
                                &effects_list,
                                &snapshots,
                                &self.fx_registry,
                            );
                            if !changes.is_empty() {
                                self.apply_node_param_changes(&changes);
                            }
                            match action {
                                node_editor::GraphAction::Save => {
                                    // Save param cache into graph nodes before serializing
                                    self.fx_graph.save_params_from_cache(&self.node_editor_state.param_cache);
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("RiffLab Graph", &["json"])
                                        .set_file_name("graph.json")
                                        .save_file()
                                    {
                                        if let Err(e) = node_editor::save_graph(&path, &self.fx_graph) {
                                            self.message_log.push(format!("Save graph failed: {e}"), true);
                                        } else {
                                            self.message_log.push(format!("Graph saved to {}", path.display()), false);
                                        }
                                    }
                                }
                                node_editor::GraphAction::Load => {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("RiffLab Graph", &["json"])
                                        .pick_file()
                                    {
                                        match node_editor::load_graph(&path) {
                                            Ok(g) => {
                                                self.fx_graph = g;
                                                self.fx_graph.load_params_to_cache(&mut self.node_editor_state.param_cache);
                                                self.fx_graph_compiled_hash = 0;
                                                self.compile_graph_if_changed();
                                                self.message_log.push(format!("Graph loaded from {}", path.display()), false);
                                            }
                                            Err(e) => {
                                                self.message_log.push(format!("Load graph failed: {e}"), true);
                                            }
                                        }
                                    }
                                }
                                node_editor::GraphAction::Changed => {
                                    self.fx_graph_compiled_hash = 0;
                                    self.node_editor_state.param_cache.clear();
                                    self.compile_graph_if_changed();
                                }
                                node_editor::GraphAction::None => {}
                            }
                            // Save current pipeline back to the editing preset node
                            if let Some(edit_id) = self.editing_preset_id {
                                if let Some(node) = self.preset_nav.find_node_mut(edit_id) {
                                    node.pipeline = self.fx_graph.clone();
                                    node.pipeline.save_params_from_cache(&self.node_editor_state.param_cache);
                                }
                            }
                            // Mark track FX dirty on param changes (throttled recompile)
                            if self.editing_track_fx.is_some() && !changes.is_empty() {
                                self.track_fx_dirty_since = Some(std::time::Instant::now());
                            }
                        }
                    }
                });
        }

        // ─── Throttled track FX recompile (200ms after last change) ────────
        if let Some(dirty_since) = self.track_fx_dirty_since {
            if dirty_since.elapsed() >= std::time::Duration::from_millis(200) {
                if let Some(track_idx) = self.editing_track_fx {
                    self.save_track_fx_from_editor(track_idx);
                }
                self.track_fx_dirty_since = None;
            }
        }

        // ─── Sidebar ─────────────────────────────────────────────
        // Snapshot graph state with try_lock — never block the audio thread.
        // If we can't get the lock this frame, use stale data from last frame.
        let graph_arc = self.engine.lock().unwrap().graph().clone();
        let (num_stems, mut solos, mut mutes, mut volumes, mut master_vol, mut input_vol, has_fx) = {
            if let Ok(graph) = graph_arc.try_lock() {
                let hfx: Vec<bool> = graph.stem_fx.iter()
                    .map(|f| f.as_ref().map_or(false, |c| !c.is_empty()))
                    .collect();
                let snap = (
                    graph.stem_players.len(),
                    graph.stem_solos.clone(),
                    graph.stem_mutes.clone(),
                    graph.stem_volumes.clone(),
                    graph.master_volume,
                    graph.input_volume,
                    hfx,
                );
                self.sidebar_snapshot = Some((snap.0, snap.1.clone(), snap.2.clone(), snap.3.clone(), snap.4, snap.5, snap.6.clone()));
                snap
            } else if let Some(ref snap) = self.sidebar_snapshot {
                snap.clone()
            } else {
                (0, Vec::new(), Vec::new(), Vec::new(), 1.0, 1.0, Vec::new())
            }
        };

        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;

                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Tracks")
                            .strong()
                            .size(13.0)
                            .color(egui::Color32::from_rgb(200, 210, 220)),
                    );
                    ui.separator();

                    for i in 0..num_stems {
                        let track_color = TRACK_COLORS[i % TRACK_COLORS.len()];
                        let track_name = if i < self.waveform_overviews.len() {
                            self.waveform_overviews[i].name.clone()
                        } else {
                            format!("Track {}", i + 1)
                        };

                        ui.add_space(4.0);

                        // Track header with color indicator
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(rect.center(), 4.0, track_color);

                            ui.label(
                                egui::RichText::new(&track_name)
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(220, 225, 230)),
                            );
                        });

                        // Solo / Mute buttons
                        ui.horizontal(|ui| {
                            let is_solo = solos.get(i).copied().unwrap_or(false);
                            let solo_color = if is_solo {
                                egui::Color32::from_rgb(220, 200, 40)
                            } else {
                                egui::Color32::from_rgb(80, 80, 80)
                            };
                            let solo_btn = egui::Button::new(
                                egui::RichText::new("S")
                                    .size(11.0)
                                    .color(if is_solo {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::from_rgb(160, 160, 160)
                                    }),
                            )
                            .fill(solo_color)
                            .min_size(egui::vec2(24.0, 18.0));
                            if ui.add(solo_btn).clicked() {
                                if let Some(s) = solos.get_mut(i) {
                                    *s = !*s;
                                }
                            }

                            let is_mute = mutes.get(i).copied().unwrap_or(false);
                            let mute_color = if is_mute {
                                egui::Color32::from_rgb(220, 60, 60)
                            } else {
                                egui::Color32::from_rgb(80, 80, 80)
                            };
                            let mute_btn = egui::Button::new(
                                egui::RichText::new("M")
                                    .size(11.0)
                                    .color(if is_mute {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_rgb(160, 160, 160)
                                    }),
                            )
                            .fill(mute_color)
                            .min_size(egui::vec2(24.0, 18.0));
                            if ui.add(mute_btn).clicked() {
                                if let Some(m) = mutes.get_mut(i) {
                                    *m = !*m;
                                }
                            }
                        });

                        // Volume slider
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Vol")
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(140, 140, 140)),
                            );
                            let mut vol = volumes.get(i).copied().unwrap_or(1.0);
                            let slider = egui::Slider::new(&mut vol, 0.0..=1.0)
                                .show_value(false)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
                            if ui.add(slider).changed() {
                                if let Some(v) = volumes.get_mut(i) {
                                    *v = vol;
                                }
                            }
                        });

                        // Per-track effects
                        let track_has_fx = has_fx.get(i).copied().unwrap_or(false);
                        ui.horizontal(|ui| {
                            if track_has_fx {
                                ui.colored_label(egui::Color32::from_rgb(80, 200, 120),
                                    egui::RichText::new("FX").size(9.0));
                                let is_editing = self.editing_track_fx == Some(i);
                                let edit_label = if is_editing { "Editing" } else { "Edit" };
                                if ui.small_button(edit_label).clicked() {
                                    if is_editing {
                                        // Stop editing: save graph back and recompile
                                        self.save_track_fx_from_editor(i);
                                        self.editing_track_fx = None;
                                        self.track_fx_dirty_since = None;
                                    } else {
                                        // Start editing: load graph into editor
                                        self.start_editing_track_fx(i);
                                    }
                                }
                                if ui.small_button("Clear").clicked() {
                                    if let Ok(mut g) = graph_arc.try_lock() {
                                        if let Some(fx) = g.stem_fx.get_mut(i) {
                                            *fx = None;
                                        }
                                    }
                                    if let Some(sg) = self.stem_graphs.get_mut(i) {
                                        *sg = None;
                                    }
                                    if self.editing_track_fx == Some(i) {
                                        self.editing_track_fx = None;
                                    }
                                }
                            } else {
                                if ui.small_button("Load FX").clicked() {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("RiffLab Graph", &["json"])
                                        .pick_file()
                                    {
                                        self.load_track_fx(i, &path);
                                    }
                                }
                            }
                        });

                        if i + 1 < num_stems {
                            ui.separator();
                        }
                    }

                    if num_stems == 0 {
                        ui.add_space(20.0);
                        ui.label(
                            egui::RichText::new("No tracks loaded")
                                .size(11.0)
                                .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                    }

                    // ── Live Input track ──
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        // Color dot (orange for input)
                        let input_color = egui::Color32::from_rgb(240, 140, 40);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, input_color);

                        ui.label(
                            egui::RichText::new("Live Input")
                                .size(12.0)
                                .color(egui::Color32::from_rgb(220, 225, 230)),
                        );
                    });

                    // Mute button for input
                    ui.horizontal(|ui| {
                        let is_muted = input_vol <= 0.0;
                        let mute_color = if is_muted {
                            egui::Color32::from_rgb(220, 60, 60)
                        } else {
                            egui::Color32::from_rgb(80, 80, 80)
                        };
                        let mute_btn = egui::Button::new(
                            egui::RichText::new("M")
                                .size(11.0)
                                .color(if is_muted {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_rgb(160, 160, 160)
                                }),
                        )
                        .fill(mute_color)
                        .min_size(egui::vec2(24.0, 18.0));
                        if ui.add(mute_btn).clicked() {
                            if is_muted {
                                input_vol = 1.0; // unmute
                            } else {
                                input_vol = 0.0; // mute
                            }
                        }
                    });

                    // Input volume slider
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Vol")
                                .size(10.0)
                                .color(egui::Color32::from_rgb(140, 140, 140)),
                        );
                        let slider = egui::Slider::new(&mut input_vol, 0.0..=2.0)
                            .show_value(false)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
                        ui.add(slider);
                    });

                    // ── Master ──
                    ui.add_space(12.0);
                    ui.separator();

                    ui.label(
                        egui::RichText::new("Master")
                            .strong()
                            .size(12.0)
                            .color(egui::Color32::from_rgb(200, 210, 220)),
                    );

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Vol")
                                .size(10.0)
                                .color(egui::Color32::from_rgb(140, 140, 140)),
                        );
                        let slider = egui::Slider::new(&mut master_vol, 0.0..=1.0)
                            .show_value(false)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
                        ui.add(slider);
                    });

                    ui.add_space(8.0);
                    draw_stereo_meter(ui, self.peak_l, self.peak_r, self.rms_l, self.rms_r);
                });
            });

        // Write back only if the user changed something
        let changed = self.sidebar_snapshot.as_ref().map_or(false, |snap| {
            snap.1 != solos || snap.2 != mutes || snap.3 != volumes
                || snap.4 != master_vol || snap.5 != input_vol
        });
        if changed {
            if let Ok(mut graph) = graph_arc.try_lock() {
                graph.stem_solos = solos;
                graph.stem_mutes = mutes;
                graph.stem_volumes = volumes;
                graph.master_volume = master_vol;
                graph.input_volume = input_vol;
            }
        }

        // ─── Arrangement View (Central Panel) ────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();

            // Handle zoom with Ctrl+Scroll
            if ui.rect_contains_pointer(ui.max_rect()) {
                let scroll_delta = ctx.input(|i| i.smooth_scroll_delta);
                let modifiers = ctx.input(|i| i.modifiers);

                if modifiers.ctrl && scroll_delta.y.abs() > 0.0 {
                    let zoom_factor = if scroll_delta.y > 0.0 { 0.85 } else { 1.18 };
                    // Zoom around mouse position
                    if let Some(mouse_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        let arrangement_left = ui.max_rect().left();
                        let mouse_x = (mouse_pos.x - arrangement_left) as f64;
                        let frame_at_mouse = self.x_to_frame(mouse_x);

                        self.frames_per_pixel =
                            (self.frames_per_pixel * zoom_factor).clamp(MIN_FRAMES_PER_PIXEL, MAX_FRAMES_PER_PIXEL);

                        // Adjust scroll so the frame under the mouse stays in place
                        self.scroll_offset_frames =
                            frame_at_mouse as f64 - mouse_x * self.frames_per_pixel;
                        self.scroll_offset_frames = self.scroll_offset_frames.max(0.0);
                    }
                } else if !modifiers.ctrl && scroll_delta.x.abs() > 0.0 {
                    // Horizontal scroll
                    self.scroll_offset_frames -= scroll_delta.x as f64 * self.frames_per_pixel;
                    self.scroll_offset_frames = self.scroll_offset_frames.max(0.0);
                    self.auto_follow = false;
                }
            }

            // Auto-follow playhead during playback
            if self.auto_follow && state == TransportState::Playing {
                let playhead_x = self.frame_to_x(position_frame);
                let view_width = available.x as f64;
                // Keep playhead in the middle third of the view
                if playhead_x > view_width * 0.75 || playhead_x < view_width * 0.1 {
                    self.scroll_offset_frames =
                        position_frame as f64 - view_width * 0.25 * self.frames_per_pixel;
                    self.scroll_offset_frames = self.scroll_offset_frames.max(0.0);
                }
            }

            // ─── Timeline ruler ─────────────────────────────────
            let ruler_rect = {
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(available.x, RULER_HEIGHT),
                    egui::Sense::click_and_drag(),
                );
                let rect = response.rect;

                // Ruler background
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(35, 38, 42));

                // Time markings
                draw_time_ruler(
                    &painter,
                    rect,
                    self.scroll_offset_frames,
                    self.frames_per_pixel,
                    self.sample_rate,
                );

                // Loop region overlay on ruler
                if loop_enabled && loop_end > loop_start {
                    let lx0 = rect.left() + self.frame_to_x(loop_start) as f32;
                    let lx1 = rect.left() + self.frame_to_x(loop_end) as f32;
                    let loop_rect = egui::Rect::from_min_max(
                        egui::pos2(lx0.max(rect.left()), rect.top()),
                        egui::pos2(lx1.min(rect.right()), rect.bottom()),
                    );
                    painter.rect_filled(
                        loop_rect,
                        0.0,
                        egui::Color32::from_rgba_premultiplied(140, 80, 200, 50),
                    );
                    // Loop bracket lines
                    painter.line_segment(
                        [
                            egui::pos2(lx0, rect.top()),
                            egui::pos2(lx0, rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(180, 120, 240)),
                    );
                    painter.line_segment(
                        [
                            egui::pos2(lx1, rect.top()),
                            egui::pos2(lx1, rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(180, 120, 240)),
                    );
                }

                // Cue markers on ruler (snapshot to avoid borrow conflicts with context menu)
                {
                    let cues_snap: Vec<_> = self.cue_engine.cue_list().cues.iter()
                        .filter_map(|cue| {
                            let frame = cue.position.to_frame(None, self.sample_rate)?;
                            Some((frame, cue.label.clone(), cue.color))
                        })
                        .collect();
                    for (cue_frame, label, color_rgb) in &cues_snap {
                        let cx = rect.left() + self.frame_to_x(*cue_frame) as f32;
                        if cx >= rect.left() && cx <= rect.right() {
                            let color = egui::Color32::from_rgb(color_rgb[0], color_rgb[1], color_rgb[2]);
                            painter.line_segment(
                                [egui::pos2(cx, rect.top()), egui::pos2(cx, rect.bottom())],
                                egui::Stroke::new(1.5, color),
                            );
                            if !label.is_empty() {
                                painter.text(
                                    egui::pos2(cx + 3.0, rect.top() + 2.0),
                                    egui::Align2::LEFT_TOP,
                                    label,
                                    egui::FontId::proportional(9.0),
                                    color,
                                );
                            }
                        }
                    }
                }

                // Playhead on ruler
                let playhead_x = rect.left() + self.frame_to_x(position_frame) as f32;
                if playhead_x >= rect.left() && playhead_x <= rect.right() {
                    // Draw a small triangle at the top
                    let tri_size = 5.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(playhead_x - tri_size, rect.top()),
                            egui::pos2(playhead_x + tri_size, rect.top()),
                            egui::pos2(playhead_x, rect.top() + tri_size * 1.5),
                        ],
                        egui::Color32::from_rgb(220, 50, 50),
                        egui::Stroke::NONE,
                    ));
                }

                // Handle ruler click to seek, drag to create loop
                if response.drag_started() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let x = (pos.x - rect.left()) as f64;
                        let frame = self.x_to_frame(x);
                        self.loop_drag.dragging = true;
                        self.loop_drag.start_frame = frame;
                        self.loop_drag.current_frame = frame;
                    }
                }

                if response.dragged() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let x = (pos.x - rect.left()) as f64;
                        let frame = self.x_to_frame(x);
                        self.loop_drag.current_frame = frame;
                    }
                }

                if response.drag_stopped() && self.loop_drag.dragging {
                    let s = self.loop_drag.start_frame;
                    let e = self.loop_drag.current_frame;
                    let (start, end) = if s < e { (s, e) } else { (e, s) };

                    // Only set loop if the region is meaningful (> 0.05 seconds)
                    let min_frames = (self.sample_rate as f64 * 0.05) as u64;
                    if end - start > min_frames {
                        let mut eng = self.engine.lock().unwrap();
                        eng.transport_mut()
                            .set_loop(Some(LoopRegion {
                                start_frame: start,
                                end_frame: end,
                            }));
                    }
                    self.loop_drag.dragging = false;
                }

                // Single click (not drag) = seek
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let x = (pos.x - rect.left()) as f64;
                        let frame = self.x_to_frame(x);
                        let mut eng = self.engine.lock().unwrap();
                        let len = eng.transport().length();
                        eng.transport_mut().seek(frame.min(len));
                    }
                }

                // Right-click context menu on ruler
                response.context_menu(|ui| {
                    let click_frame = response.interact_pointer_pos()
                        .map(|pos| self.x_to_frame((pos.x - rect.left()) as f64))
                        .unwrap_or(0);
                    let time_secs = click_frame as f64 / self.sample_rate as f64;

                    ui.label(egui::RichText::new(format!("@ {:.1}s", time_secs)).size(10.0));
                    ui.separator();

                    let next_idx = self.cue_engine.cue_list().len() + 1;
                    if ui.button("Add Marker").clicked() {
                        self.cue_engine.cue_list_mut().add(Cue {
                            position: CuePosition::Frame(click_frame),
                            action: CueAction::Marker,
                            label: format!("Cue {}", next_idx),
                            color: [220, 200, 50],
                        });
                        ui.close_menu();
                    }
                    if ui.button("Add Loop Cue").clicked() {
                        let end = click_frame + self.sample_rate as u64 * 4;
                        self.cue_engine.cue_list_mut().add(Cue {
                            position: CuePosition::Frame(click_frame),
                            action: CueAction::SetLoop(LoopRegion {
                                start_frame: click_frame,
                                end_frame: end,
                            }),
                            label: format!("Loop {}", next_idx),
                            color: [140, 80, 220],
                        });
                        ui.close_menu();
                    }
                    if ui.button("Add Preset Cue").clicked() {
                        self.cue_engine.cue_list_mut().add(Cue {
                            position: CuePosition::Frame(click_frame),
                            action: CueAction::SwitchPreset("Default".into()),
                            label: "Preset".to_string(),
                            color: [80, 200, 140],
                        });
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Clear Loop").clicked() {
                        let mut eng = self.engine.lock().unwrap();
                        eng.transport_mut().set_loop(None);
                        ui.close_menu();
                    }
                    if !self.cue_engine.cue_list().is_empty() {
                        if ui.button("Clear All Cues").clicked() {
                            *self.cue_engine.cue_list_mut() = CueList::new();
                            ui.close_menu();
                        }
                    }
                });

                // Drag preview overlay
                if self.loop_drag.dragging {
                    let s = self.loop_drag.start_frame;
                    let e = self.loop_drag.current_frame;
                    let (start, end) = if s < e { (s, e) } else { (e, s) };
                    let lx0 = rect.left() + self.frame_to_x(start) as f32;
                    let lx1 = rect.left() + self.frame_to_x(end) as f32;
                    let drag_rect = egui::Rect::from_min_max(
                        egui::pos2(lx0.max(rect.left()), rect.top()),
                        egui::pos2(lx1.min(rect.right()), rect.bottom()),
                    );
                    painter.rect_filled(
                        drag_rect,
                        0.0,
                        egui::Color32::from_rgba_premultiplied(180, 120, 240, 40),
                    );
                }

                rect
            };

            // ─── View toggle (Waveform / Spectrogram) ───────────
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let views = [
                    (ArrangementView::Waveform, "Waves"),
                    (ArrangementView::Spectrogram, "Spectrogram"),
                ];
                for (view, label) in &views {
                    let active = self.arrangement_view == *view;
                    let btn = egui::Button::new(
                        egui::RichText::new(*label).size(10.0)
                            .color(if active { egui::Color32::WHITE } else { egui::Color32::from_rgb(130, 135, 140) })
                    )
                    .fill(if active { egui::Color32::from_rgb(50, 55, 65) } else { egui::Color32::TRANSPARENT })
                    .corner_radius(2)
                    .min_size(egui::vec2(70.0, 16.0));
                    if ui.add(btn).clicked() {
                        self.arrangement_view = *view;
                        self.spectrogram_texture = None; // invalidate
                    }
                }
            });

            // ─── Track lanes ─────────────────────────────────────
            let track_area_height =
                available.y - RULER_HEIGHT - 26.0; // ruler + toggle bar margin
            let num_tracks = self.waveform_overviews.len().max(1);
            let lane_height =
                (track_area_height / num_tracks as f32).clamp(60.0, TRACK_LANE_HEIGHT);

            if self.arrangement_view == ArrangementView::Spectrogram && !self.spectrograms.is_empty() {
                // ─── Spectrogram view ─────────────────────────────
                let spec_height = track_area_height.max(60.0);
                self.draw_spectrogram(ui, ctx, available.x, spec_height);
            } else if self.waveform_overviews.is_empty() {
                // No tracks — show placeholder
                let (_, painter) = ui.allocate_painter(
                    egui::vec2(available.x, lane_height),
                    egui::Sense::hover(),
                );
                let rect = painter.clip_rect();
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(22, 24, 28));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No audio loaded. Usage: rifflab <audio-file>",
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_rgb(100, 110, 120),
                );
            } else {
                for (track_idx, overview) in self.waveform_overviews.iter().enumerate() {
                    let (response, painter) = ui.allocate_painter(
                        egui::vec2(available.x, lane_height),
                        egui::Sense::click(),
                    );
                    let rect = response.rect;

                    // Track background
                    let bg_color = if track_idx % 2 == 0 {
                        egui::Color32::from_rgb(22, 24, 28)
                    } else {
                        egui::Color32::from_rgb(26, 28, 32)
                    };
                    painter.rect_filled(rect, 0.0, bg_color);

                    // Center line
                    let center_y = rect.center().y;
                    painter.line_segment(
                        [
                            egui::pos2(rect.left(), center_y),
                            egui::pos2(rect.right(), center_y),
                        ],
                        egui::Stroke::new(
                            0.5,
                            egui::Color32::from_rgba_premultiplied(255, 255, 255, 20),
                        ),
                    );

                    // Loop region overlay on waveform
                    if loop_enabled && loop_end > loop_start {
                        let lx0 = rect.left() + self.frame_to_x(loop_start) as f32;
                        let lx1 = rect.left() + self.frame_to_x(loop_end) as f32;
                        let loop_rect = egui::Rect::from_min_max(
                            egui::pos2(lx0.max(rect.left()), rect.top()),
                            egui::pos2(lx1.min(rect.right()), rect.bottom()),
                        );
                        painter.rect_filled(
                            loop_rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(140, 80, 200, 20),
                        );
                    }

                    // Draw waveform
                    let half_height = (rect.height() * 0.45) as f64;
                    let width = rect.width() as usize;

                    // Build the filled waveform shape as two polylines (top envelope + bottom envelope)
                    let mut top_points: Vec<egui::Pos2> = Vec::with_capacity(width);
                    let mut bottom_points: Vec<egui::Pos2> = Vec::with_capacity(width);

                    for px in 0..width {
                        let frame_start =
                            self.scroll_offset_frames + px as f64 * self.frames_per_pixel;
                        let frame_end = frame_start + self.frames_per_pixel;

                        if frame_start < 0.0 || frame_start >= overview.total_frames as f64 {
                            top_points.push(egui::pos2(rect.left() + px as f32, center_y));
                            bottom_points.push(egui::pos2(rect.left() + px as f32, center_y));
                            continue;
                        }

                        let (min_val, max_val) = overview.peaks_for_range(
                            frame_start.max(0.0) as u64,
                            (frame_end as u64).min(overview.total_frames),
                        );

                        let x = rect.left() + px as f32;
                        let y_top = center_y - (max_val as f64 * half_height) as f32;
                        let y_bot = center_y - (min_val as f64 * half_height) as f32;

                        top_points.push(egui::pos2(x, y_top));
                        bottom_points.push(egui::pos2(x, y_bot));
                    }

                    // Build a filled polygon from top_points + reversed bottom_points
                    if !top_points.is_empty() {
                        // Draw filled area
                        let waveform_fill = egui::Color32::from_rgba_premultiplied(
                            overview.color.r(),
                            overview.color.g(),
                            overview.color.b(),
                            60,
                        );
                        let waveform_stroke_color = egui::Color32::from_rgba_premultiplied(
                            overview.color.r(),
                            overview.color.g(),
                            overview.color.b(),
                            180,
                        );

                        // Draw filled columns using line segments for efficiency
                        for i in 0..top_points.len() {
                            let x = top_points[i].x;
                            let y_top = top_points[i].y;
                            let y_bot = bottom_points[i].y;
                            if (y_bot - y_top).abs() > 0.5 {
                                painter.line_segment(
                                    [egui::pos2(x, y_top), egui::pos2(x, y_bot)],
                                    egui::Stroke::new(1.0, waveform_fill),
                                );
                            }
                        }

                        // Draw top and bottom envelope lines
                        if top_points.len() >= 2 {
                            painter.add(egui::Shape::line(
                                top_points,
                                egui::Stroke::new(0.8, waveform_stroke_color),
                            ));
                            painter.add(egui::Shape::line(
                                bottom_points,
                                egui::Stroke::new(0.8, waveform_stroke_color),
                            ));
                        }
                    }

                    // Playhead line on waveform
                    let playhead_x = rect.left() + self.frame_to_x(position_frame) as f32;
                    if playhead_x >= rect.left() && playhead_x <= rect.right() {
                        painter.line_segment(
                            [
                                egui::pos2(playhead_x, rect.top()),
                                egui::pos2(playhead_x, rect.bottom()),
                            ],
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(220, 50, 50)),
                        );
                    }

                    // Click on waveform to seek
                    if response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let x = (pos.x - rect.left()) as f64;
                            let frame = self.x_to_frame(x);
                            let mut eng = self.engine.lock().unwrap();
                            let len = eng.transport().length();
                            eng.transport_mut().seek(frame.min(len));
                        }
                    }

                    // Track label in top-left corner
                    painter.text(
                        egui::pos2(rect.left() + 6.0, rect.top() + 4.0),
                        egui::Align2::LEFT_TOP,
                        &overview.name,
                        egui::FontId::proportional(11.0),
                        egui::Color32::from_rgba_premultiplied(
                            overview.color.r(),
                            overview.color.g(),
                            overview.color.b(),
                            160,
                        ),
                    );

                    // Thin separator line between tracks
                    if track_idx + 1 < self.waveform_overviews.len() {
                        painter.line_segment(
                            [
                                egui::pos2(rect.left(), rect.bottom()),
                                egui::pos2(rect.right(), rect.bottom()),
                            ],
                            egui::Stroke::new(
                                0.5,
                                egui::Color32::from_rgb(50, 55, 60),
                            ),
                        );
                    }
                }
            }

            // ─── Horizontal scrollbar ─────────────────────────────
            // Compute scrollbar proportions
            let max_total_frames = self
                .waveform_overviews
                .iter()
                .map(|o| o.total_frames)
                .max()
                .unwrap_or(0) as f64;

            if max_total_frames > 0.0 {
                let view_frames = available.x as f64 * self.frames_per_pixel;
                let total_scrollable = max_total_frames + view_frames * 0.1;
                let frac_start = (self.scroll_offset_frames / total_scrollable) as f32;
                let frac_width = (view_frames / total_scrollable) as f32;

                let scrollbar_height = 10.0;
                let (sb_response, sb_painter) = ui.allocate_painter(
                    egui::vec2(available.x, scrollbar_height),
                    egui::Sense::click_and_drag(),
                );
                let sb_rect = sb_response.rect;

                // Background
                sb_painter.rect_filled(sb_rect, 2.0, egui::Color32::from_rgb(30, 32, 36));

                // Thumb
                let thumb_left = sb_rect.left() + frac_start * sb_rect.width();
                let thumb_width = (frac_width * sb_rect.width()).max(20.0);
                let thumb_rect = egui::Rect::from_min_size(
                    egui::pos2(thumb_left, sb_rect.top()),
                    egui::vec2(thumb_width, scrollbar_height),
                );
                let thumb_color = if sb_response.hovered() || sb_response.dragged() {
                    egui::Color32::from_rgb(90, 95, 105)
                } else {
                    egui::Color32::from_rgb(60, 65, 72)
                };
                sb_painter.rect_filled(thumb_rect, 2.0, thumb_color);

                // Handle scrollbar drag
                if sb_response.dragged() {
                    let delta_x = sb_response.drag_delta().x;
                    let delta_frac = delta_x / sb_rect.width();
                    self.scroll_offset_frames += delta_frac as f64 * total_scrollable;
                    self.scroll_offset_frames = self.scroll_offset_frames
                        .max(0.0)
                        .min(max_total_frames);
                    self.auto_follow = false;
                }

                // Click on scrollbar to jump
                if sb_response.clicked() {
                    if let Some(pos) = sb_response.interact_pointer_pos() {
                        let frac = ((pos.x - sb_rect.left()) / sb_rect.width())
                            .clamp(0.0, 1.0) as f64;
                        self.scroll_offset_frames =
                            frac * total_scrollable - view_frames * 0.5;
                        self.scroll_offset_frames = self.scroll_offset_frames
                            .max(0.0)
                            .min(max_total_frames);
                        self.auto_follow = false;
                    }
                }
            }

            // Extend the playhead line from the ruler across the whole arrangement
            let _ = ruler_rect; // used above for playhead triangle
        });

        // ─── Audio Settings Window ───────────────────────────────
        self.draw_audio_settings(ctx);
        self.draw_tuner_settings(ctx);
        self.draw_midi_panel(ctx);
        self.draw_message_log(ctx);

        // Request continuous repaint for smooth animation
        ctx.request_repaint();
    }
}

impl RiffLabApp {
    fn draw_audio_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.audio_settings.open;
        egui::Window::new("Audio Settings")
            .open(&mut open)
            .resizable(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let section_color = egui::Color32::from_rgb(180, 200, 220);
                let hint_color = egui::Color32::from_rgb(130, 135, 145);

                let is_jack = self.audio_settings.backend == "jack";

                // ── Backend ──
                ui.label(egui::RichText::new("Backend").strong().color(section_color));
                egui::ComboBox::from_id_salt("backend_combo")
                    .selected_text(&self.audio_settings.backend)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for b in &self.audio_settings.available_backends {
                            let label = match b.as_str() {
                                "auto" => "Auto (try cpal, then JACK)",
                                "alsa" => "cpal (ALSA / PipeWire)",
                                "jack" => "JACK",
                                other => other,
                            };
                            if ui.selectable_value(&mut self.audio_settings.backend, b.clone(), label).changed() {
                                self.audio_settings.dirty = true;
                            }
                        }
                    });

                ui.add_space(4.0);
                ui.separator();

                if is_jack {
                    let pw = &self.audio_settings.pipewire;
                    if !pw.available {
                        ui.label(
                            egui::RichText::new(
                                "JACK mode: pw-metadata not found.\n\
                                 Install PipeWire tools to configure server settings,\n\
                                 or use qpwgraph to route ports."
                            )
                            .size(11.0)
                            .color(hint_color),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(
                                "JACK mode: these settings change the PipeWire server.\n\
                                 Device routing is done via qpwgraph or equivalent."
                            )
                            .size(11.0)
                            .color(hint_color),
                        );

                        ui.add_space(4.0);

                        // ── PipeWire Sample Rate ──
                        ui.label(egui::RichText::new("Server Sample Rate").strong().color(section_color));
                        ui.horizontal(|ui| {
                            let rates = [44100u32, 48000, 96000];
                            for &rate in &rates {
                                let label = format!("{} Hz", rate);
                                if ui.selectable_label(
                                    self.audio_settings.pw_rate == rate,
                                    &label,
                                ).clicked() {
                                    self.audio_settings.pw_rate = rate;
                                    self.audio_settings.dirty = true;
                                }
                            }
                        });

                        ui.add_space(4.0);

                        // ── PipeWire Buffer Size (quantum) ──
                        ui.label(egui::RichText::new("Server Buffer Size (quantum)").strong().color(section_color));
                        ui.horizontal(|ui| {
                            let sizes = [64u32, 128, 256, 512, 1024, 2048];
                            for &size in &sizes {
                                let in_range = size >= pw.min_quantum && size <= pw.max_quantum;
                                let latency = size as f64 / self.audio_settings.pw_rate.max(1) as f64 * 1000.0;
                                let label = format!("{} ({:.1}ms)", size, latency);
                                let mut btn = ui.add_enabled(
                                    in_range,
                                    egui::SelectableLabel::new(
                                        self.audio_settings.pw_quantum == size,
                                        &label,
                                    ),
                                );
                                if !in_range {
                                    btn = btn.on_disabled_hover_text(format!(
                                        "Server range: {}–{}",
                                        pw.min_quantum, pw.max_quantum,
                                    ));
                                }
                                if btn.clicked() {
                                    self.audio_settings.pw_quantum = size;
                                    self.audio_settings.dirty = true;
                                }
                            }
                        });
                    }
                } else {
                    // cpal: full device and parameter selection

                    // ── Output Device ──
                    ui.label(egui::RichText::new("Output Device").strong().color(section_color));
                    {
                        let current = if self.audio_settings.output_device.is_empty() {
                            "(system default)".to_string()
                        } else {
                            self.audio_settings.output_device.clone()
                        };
                        egui::ComboBox::from_id_salt("output_device_combo")
                            .selected_text(&current)
                            .width(300.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_value(
                                    &mut self.audio_settings.output_device,
                                    String::new(),
                                    "(system default)",
                                ).changed() {
                                    self.audio_settings.dirty = true;
                                }
                                for dev in &self.audio_settings.output_devices {
                                    let rates_str = dev.sample_rates.iter()
                                        .map(|r| format!("{}k", r / 1000))
                                        .collect::<Vec<_>>()
                                        .join("/");
                                    let label = format!(
                                        "{} ({}ch, {}{})",
                                        dev.name,
                                        dev.channels,
                                        rates_str,
                                        if dev.is_default { " *" } else { "" },
                                    );
                                    if ui.selectable_value(
                                        &mut self.audio_settings.output_device,
                                        dev.name.clone(),
                                        label,
                                    ).changed() {
                                        self.audio_settings.dirty = true;
                                    }
                                }
                            });
                    }

                    ui.add_space(4.0);

                    // ── Input Device ──
                    ui.label(egui::RichText::new("Input Device").strong().color(section_color));
                    {
                        let current = if self.audio_settings.input_device.is_empty() {
                            "(system default)".to_string()
                        } else if self.audio_settings.input_device == "(none)" {
                            "(none)".to_string()
                        } else {
                            self.audio_settings.input_device.clone()
                        };
                        egui::ComboBox::from_id_salt("input_device_combo")
                            .selected_text(&current)
                            .width(300.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_value(
                                    &mut self.audio_settings.input_device,
                                    String::new(),
                                    "(system default)",
                                ).changed() {
                                    self.audio_settings.dirty = true;
                                }
                                if ui.selectable_value(
                                    &mut self.audio_settings.input_device,
                                    "(none)".to_string(),
                                    "(none — no input)",
                                ).changed() {
                                    self.audio_settings.dirty = true;
                                }
                                for dev in &self.audio_settings.input_devices {
                                    let rates_str = dev.sample_rates.iter()
                                        .map(|r| format!("{}k", r / 1000))
                                        .collect::<Vec<_>>()
                                        .join("/");
                                    let label = format!(
                                        "{} ({}ch, {}{})",
                                        dev.name,
                                        dev.channels,
                                        rates_str,
                                        if dev.is_default { " *" } else { "" },
                                    );
                                    if ui.selectable_value(
                                        &mut self.audio_settings.input_device,
                                        dev.name.clone(),
                                        label,
                                    ).changed() {
                                        self.audio_settings.dirty = true;
                                    }
                                }
                            });
                    }

                    ui.add_space(4.0);
                    ui.separator();

                    // ── Sample Rate ──
                    ui.label(egui::RichText::new("Sample Rate").strong().color(section_color));
                    // On PipeWire, we control the rate via pw-metadata so all standard rates
                    // are available regardless of what cpal reports (it only shows the active rate).
                    ui.horizontal(|ui| {
                        let rates = [44100u32, 48000, 96000];
                        for &rate in &rates {
                            if ui.selectable_label(
                                self.audio_settings.sample_rate == rate,
                                format!("{} Hz", rate),
                            ).clicked() {
                                self.audio_settings.sample_rate = rate;
                                self.audio_settings.dirty = true;
                            }
                        }
                    });

                    ui.add_space(4.0);

                    // ── Buffer Size ──
                    ui.label(egui::RichText::new("Buffer Size").strong().color(section_color));
                    ui.horizontal(|ui| {
                        let sizes = [64u32, 128, 256, 512, 1024];
                        for &size in &sizes {
                            let latency = size as f64 / self.audio_settings.sample_rate as f64 * 1000.0;
                            let label = format!("{} ({:.1}ms)", size, latency);
                            if ui.selectable_label(
                                self.audio_settings.buffer_size == size,
                                &label,
                            ).clicked() {
                                self.audio_settings.buffer_size = size;
                                self.audio_settings.dirty = true;
                            }
                        }
                    });
                }

                ui.add_space(8.0);
                ui.separator();

                // ── Status / Apply / Refresh ──
                if !self.audio_settings.status_msg.is_empty() {
                    let color = if self.audio_settings.status_msg.starts_with("Error") {
                        egui::Color32::from_rgb(220, 80, 80)
                    } else {
                        egui::Color32::from_rgb(80, 200, 120)
                    };
                    ui.label(
                        egui::RichText::new(&self.audio_settings.status_msg)
                            .color(color)
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                }

                ui.horizontal(|ui| {
                    if is_jack {
                        if self.audio_settings.pipewire.available {
                            if ui.button("Refresh Server").clicked() {
                                self.audio_settings.refresh_pipewire();
                            }
                        }
                    } else {
                        if ui.button("Refresh Devices").clicked() {
                            self.audio_settings.refresh_devices();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let apply_text = if self.audio_settings.dirty {
                            egui::RichText::new("Apply & Restart Audio").strong()
                        } else {
                            egui::RichText::new("Apply & Restart Audio")
                        };
                        if ui.button(apply_text).clicked() {
                            self.apply_audio_settings();
                        }
                    });
                });
            });
        self.audio_settings.open = open;
    }

    fn apply_audio_settings(&mut self) {
        let is_jack = self.audio_settings.backend == "jack";

        // For JACK: apply PipeWire server settings first
        if is_jack && self.audio_settings.pipewire.available {
            let pw = &self.audio_settings.pipewire;

            if self.audio_settings.pw_rate != pw.clock_rate {
                match pw.set_rate(self.audio_settings.pw_rate) {
                    Ok(()) => log::info!("PipeWire rate set to {}", self.audio_settings.pw_rate),
                    Err(e) => {
                        self.audio_settings.status_msg = format!("Error setting server rate: {e}");
                        log::error!("{}", self.audio_settings.status_msg);
                        return;
                    }
                }
            }

            let current_quantum = if pw.force_quantum > 0 { pw.force_quantum } else { pw.quantum };
            if self.audio_settings.pw_quantum != current_quantum {
                match pw.set_quantum(self.audio_settings.pw_quantum) {
                    Ok(()) => log::info!("PipeWire quantum set to {}", self.audio_settings.pw_quantum),
                    Err(e) => {
                        self.audio_settings.status_msg = format!("Error setting server quantum: {e}");
                        log::error!("{}", self.audio_settings.status_msg);
                        return;
                    }
                }
            }

            // Re-read server state after changes
            self.audio_settings.refresh_pipewire();

            // Update our config to reflect the server values
            self.audio_settings.sample_rate = self.audio_settings.pipewire.clock_rate;
            self.audio_settings.buffer_size = if self.audio_settings.pipewire.force_quantum > 0 {
                self.audio_settings.pipewire.force_quantum
            } else {
                self.audio_settings.pipewire.quantum
            };
        }

        // Update config
        self.app_config.audio.backend = self.audio_settings.backend.clone();
        self.app_config.audio.sample_rate = self.audio_settings.sample_rate;
        self.app_config.audio.buffer_size = self.audio_settings.buffer_size;
        if !is_jack {
            self.app_config.audio.output_device = self.audio_settings.output_device.clone();
            self.app_config.audio.input_device = self.audio_settings.input_device.clone();
        }

        // Save config to disk
        if let Some(path) = config::default_config_path() {
            if let Err(e) = self.app_config.save(&path) {
                log::error!("Failed to save config: {e}");
                self.audio_settings.status_msg = format!("Error saving config: {e}");
                return;
            }
            log::info!("Config saved to {}", path.display());
        }

        // Build new AudioConfig
        let audio_config = self.app_config.to_audio_config();
        self.sample_rate = audio_config.sample_rate.as_u32();
        self.buffer_size = audio_config.buffer_size.as_usize();

        // Restart engine
        let mut eng = self.engine.lock().unwrap();
        eng.set_backend_prefs(
            &self.app_config.audio.backend,
            &self.app_config.audio.output_device,
            &self.app_config.audio.input_device,
        );
        eng.set_config(audio_config);

        match eng.restart() {
            Ok((meter_rx, pitch_rx)) => {
                self.meter_rx = meter_rx;
                self.pitch_rx = pitch_rx;
                // Check actual hardware rate — may differ from requested
                if let Some(backend) = eng.backend_ref() {
                    if let Some(actual_rate) = backend.actual_sample_rate() {
                        if actual_rate != self.sample_rate {
                            log::warn!("Requested {}Hz, hardware using {}Hz", self.sample_rate, actual_rate);
                            self.sample_rate = actual_rate;
                            self.audio_settings.sample_rate = actual_rate;
                            self.app_config.audio.sample_rate = actual_rate;
                        }
                    }
                }
                self.audio_settings.status_msg = format!(
                    "Audio restarted: {}Hz, {} samples",
                    self.sample_rate, self.buffer_size,
                );
                self.audio_settings.dirty = false;
                log::info!("{}", self.audio_settings.status_msg);
            }
            Err(e) => {
                self.audio_settings.status_msg = format!("Error: {e}");
                log::error!("Engine restart failed: {e}");
            }
        }
    }

    fn draw_spectrogram(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, width_px: f32, height_px: f32) {
        let width = width_px as usize;
        let height = height_px as usize;
        if width < 10 || height < 10 { return; }

        // Check for completed background render
        if let Some(ref rx) = self.spectrogram_pending {
            if let Ok((image, key)) = rx.try_recv() {
                self.spectrogram_texture = Some(ctx.load_texture("spectrogram", image, egui::TextureOptions::LINEAR));
                self.spectrogram_cache_key = key;
                self.spectrogram_pending = None;
            }
        }

        // Track volumes for opacity
        let track_volumes: Vec<f32> = self.sidebar_snapshot
            .as_ref()
            .map(|(_, _, _, vols, _, _, _)| vols.clone())
            .unwrap_or_default();

        let vol_hash: u64 = track_volumes.iter()
            .enumerate()
            .fold(0u64, |h: u64, (i, v): (usize, &f32)| h.wrapping_add((v.to_bits() as u64).wrapping_mul(i as u64 + 1)));

        let cache_key = (
            self.scroll_offset_frames.to_bits(),
            self.frames_per_pixel.to_bits(),
            width as u32,
            height as u32,
            vol_hash,
        );

        // Dispatch background render if cache is stale and no render in flight
        if self.spectrogram_cache_key != cache_key
            && self.spectrogram_pending_key != cache_key
            && self.spectrogram_pending.is_none()
        {
            // Collect render params (clone the data refs we need)
            struct SpecRenderInfo {
                magnitudes: Vec<u8>,
                num_bins: usize,
                num_columns: usize,
                hop_size: usize,
                fft_size: usize,
                color: [u8; 3],
                volume: f32,
            }

            let specs: Vec<SpecRenderInfo> = self.spectrograms.iter().enumerate().map(|(i, s)| {
                SpecRenderInfo {
                    magnitudes: s.data.magnitudes.clone(),
                    num_bins: s.data.num_bins,
                    num_columns: s.data.num_columns,
                    hop_size: s.data.hop_size,
                    fft_size: s.data.fft_size,
                    color: [s.color.r(), s.color.g(), s.color.b()],
                    volume: track_volumes.get(i).copied().unwrap_or(1.0),
                }
            }).collect();

            let scroll = self.scroll_offset_frames;
            let fpp = self.frames_per_pixel;
            let sr = self.sample_rate;
            let key = cache_key;

            let (tx, rx) = std::sync::mpsc::channel();
            self.spectrogram_pending = Some(rx);
            self.spectrogram_pending_key = key;

            std::thread::spawn(move || {
                let mut pixels = vec![0u8; width * height * 4];

                let freq_min = 20.0f64;
                let freq_max = (sr as f64) / 2.0;
                let log_min = freq_min.ln();
                let log_max = freq_max.ln();

                for spec in &specs {
                    if spec.num_columns == 0 || spec.volume <= 0.0 { continue; }
                    let opacity = 0.65 * spec.volume.min(1.0);
                    let base_r = spec.color[0] as f32;
                    let base_g = spec.color[1] as f32;
                    let base_b = spec.color[2] as f32;

                    for px in 0..width {
                        let frame = scroll + px as f64 * fpp;
                        let col = (frame / spec.hop_size as f64) as usize;
                        if col >= spec.num_columns { continue; }

                        for py in 0..height {
                            let frac = (height - 1 - py) as f64 / height as f64;
                            let freq = (log_min + frac * (log_max - log_min)).exp();
                            let bin = (freq * spec.fft_size as f64 / sr as f64) as usize;
                            if bin >= spec.num_bins { continue; }

                            let mag_u8 = spec.magnitudes[col * spec.num_bins + bin];
                            if mag_u8 == 0 { continue; }

                            let alpha = (mag_u8 as f32 / 255.0) * opacity;
                            let idx = (py * width + px) * 4;
                            let inv = 1.0 - alpha;
                            pixels[idx]     = (pixels[idx]     as f32 * inv + base_r * alpha) as u8;
                            pixels[idx + 1] = (pixels[idx + 1] as f32 * inv + base_g * alpha) as u8;
                            pixels[idx + 2] = (pixels[idx + 2] as f32 * inv + base_b * alpha) as u8;
                            pixels[idx + 3] = (pixels[idx + 3] as f32 * inv + 255.0 * alpha).min(255.0) as u8;
                        }
                    }
                }

                let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
                let _ = tx.send((image, key));
            });
        }

        // Draw the last available texture (may be stale for 1-2 frames during re-render)
        if let Some(tex) = &self.spectrogram_texture {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width_px, height_px), egui::Sense::hover());
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
        } else {
            // No texture yet — allocate space with dark background
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width_px, height_px), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(18, 20, 24));
        }
    }

    /// Build parameter snapshots for all effect nodes in the graph.
    fn build_node_param_snapshots(&self) -> Vec<node_editor::NodeParamSnapshot> {
        // Get the compiled route to know which nodes map to which chain effects
        let route = self.fx_graph.compile();

        // Extract the ordered node IDs of effect nodes from the route
        let effect_node_ids: Vec<u64> = match &route {
            node_editor::CompiledRoute::SingleChain(_type_ids) => {
                // Find effect nodes in cable-traversal order by matching type_ids
                let mut ids = Vec::new();
                let input_node = self.fx_graph.nodes.iter().find(|n| matches!(n.kind, node_editor::NodeKind::Input));
                if let Some(input) = input_node {
                    let start = node_editor::PortId { node_id: input.id, index: 0, is_output: true };
                    let traced = self.fx_graph.trace_chain(start);
                    for nid in &traced {
                        if let Some(node) = self.fx_graph.find_node(*nid) {
                            if matches!(node.kind, node_editor::NodeKind::Effect { .. }) {
                                ids.push(node.id);
                            }
                        }
                    }
                }
                ids
            }
            _ => Vec::new(), // TODO: multiband
        };

        let ga = self.engine.lock().unwrap().graph().clone();
        let Ok(graph) = ga.try_lock() else { return Vec::new() };

        let mut snapshots = Vec::new();
        for (chain_idx, effect) in graph.fx_chain.effects().iter().enumerate() {
            if let Some(&node_id) = effect_node_ids.get(chain_idx) {
                let descs = effect.param_descriptors();
                let params: Vec<_> = descs.into_iter()
                    .map(|d| { let v = effect.get_param(d.id); (d, v) })
                    .collect();
                snapshots.push(node_editor::NodeParamSnapshot {
                    node_id,
                    params,
                    bypassed: effect.is_bypassed(),
                });
            }
        }

        snapshots
    }

    /// Apply parameter changes from the node editor to the audio engine.
    fn apply_node_param_changes(&mut self, changes: &[node_editor::NodeParamChange]) {
        // Get effect node IDs in cable-traversal order (matches compiled chain order)
        let effect_nodes: Vec<u64> = {
            let input = self.fx_graph.nodes.iter().find(|n| matches!(n.kind, node_editor::NodeKind::Input));
            if let Some(inp) = input {
                let start = node_editor::PortId { node_id: inp.id, index: 0, is_output: true };
                self.fx_graph.trace_chain(start).into_iter()
                    .filter(|id| self.fx_graph.find_node(*id).map_or(false, |n| matches!(n.kind, node_editor::NodeKind::Effect { .. })))
                    .collect()
            } else { Vec::new() }
        };

        // Use the same single-statement pattern that works in the sidebar
        {
            let ga = self.engine.lock().unwrap().graph().clone();
            let Ok(mut graph) = ga.try_lock() else { return };
            for change in changes {
                if let Some(chain_idx) = effect_nodes.iter().position(|id| *id == change.node_id) {
                    if let Some(effect) = graph.fx_chain.effects_mut().get_mut(chain_idx) {
                        effect.set_param(change.param_id, change.value);
                    }
                }
            }
        }
    }

    /// Compile the visual node graph into audio engine configuration when it changes.
    fn compile_graph_if_changed(&mut self) {
        // Simple hash: number of nodes + cables + node IDs
        let hash = {
            let mut h: u64 = self.fx_graph.nodes.len() as u64 * 1000003;
            for n in &self.fx_graph.nodes {
                h = h.wrapping_add(n.id.wrapping_mul(7919));
            }
            h = h.wrapping_add(self.fx_graph.cables.len() as u64 * 104729);
            for c in &self.fx_graph.cables {
                h = h.wrapping_add(c.from.node_id.wrapping_mul(31) + c.to.node_id.wrapping_mul(37));
            }
            h
        };

        if hash == self.fx_graph_compiled_hash {
            return;
        }
        self.fx_graph_compiled_hash = hash;

        let route = self.fx_graph.compile();
        // Get graph Arc once, outside the match
        let graph_arc: Arc<Mutex<rifflab_audio::graph::AudioGraph>> = {
            let eng = self.engine.lock().unwrap();
            Arc::clone(eng.graph())
        };

        match route {
            node_editor::CompiledRoute::Empty => {
                if let Ok(mut g) = graph_arc.try_lock() {
                    g.fx_chain = rifflab_fx::chain::EffectChain::new();
                    g.fx_multiband_active = false;
                    g.fx_input_connected = false;
                }
            }
            node_editor::CompiledRoute::SingleChain(type_ids) => {
                // Get effect node IDs in traversal order to match params
                let effect_node_ids: Vec<u64> = {
                    let input = self.fx_graph.nodes.iter().find(|n| matches!(n.kind, node_editor::NodeKind::Input));
                    if let Some(inp) = input {
                        let start = node_editor::PortId { node_id: inp.id, index: 0, is_output: true };
                        self.fx_graph.trace_chain(start).into_iter()
                            .filter(|id| self.fx_graph.find_node(*id).map_or(false, |n| matches!(n.kind, node_editor::NodeKind::Effect { .. })))
                            .collect()
                    } else { Vec::new() }
                };

                let mut chain = rifflab_fx::chain::EffectChain::new();
                for (i, type_id) in type_ids.iter().enumerate() {
                    if let Some(mut effect) = self.fx_registry.create_effect(type_id) {
                        // Apply cached param values
                        if let Some(&node_id) = effect_node_ids.get(i) {
                            for (&(nid, pid), &val) in &self.node_editor_state.param_cache {
                                if nid == node_id {
                                    effect.set_param(ParamId(pid), val);
                                }
                            }
                        }
                        chain.add(effect);
                    }
                }
                if let Ok(mut g) = graph_arc.try_lock() {
                    g.fx_chain = chain;
                    g.fx_multiband_active = false;
                    g.fx_input_connected = true;
                }
            }
            node_editor::CompiledRoute::Multiband {
                low_mid_hz, mid_high_hz, gains,
                low_chain, mid_chain, high_chain,
            } => {
                let build = |tids: &[String]| {
                    let mut c = rifflab_fx::chain::EffectChain::new();
                    for tid in tids {
                        if let Some(e) = self.fx_registry.create_effect(tid) { c.add(e); }
                    }
                    c
                };
                let chains = [build(&low_chain), build(&mid_chain), build(&high_chain)];

                if let Ok(mut g) = graph_arc.try_lock() {
                    let mut mb = g.fx_multiband.take().unwrap_or_else(MultibandRouter::new);
                    mb.set_crossover_low_mid(low_mid_hz);
                    mb.set_crossover_mid_high(mid_high_hz);
                    mb.band_gains = gains;
                    let [cl, cm, ch] = chains;
                    mb.chains = [cl, cm, ch];
                    g.fx_multiband = Some(mb);
                    g.fx_multiband_active = true;
                    g.fx_input_connected = true;
                }
            }
        }
        self.fx_snapshot_dirty = true;
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    fn draw_multiband_rack(
        &mut self,
        ui: &mut egui::Ui,
        _available_height: f32,
        mb_add: &mut Option<(usize, String)>,
        mb_remove: &mut Option<(usize, usize)>,
        mb_param: &mut Vec<(usize, usize, ParamId, f32)>,
        mb_bypass: &mut Vec<(usize, usize)>,
        mb_reorder: &mut Option<(usize, usize, usize)>,
        mb_xover_change: &mut Option<(f32, f32)>,
        mb_gain_change: &mut Option<[f32; 3]>,
    ) {
        let snap = match &self.fx_mb_snapshot {
            Some(s) => s,
            None => {
                ui.colored_label(egui::Color32::from_rgb(100, 110, 120), "Multiband not initialized");
                return;
            }
        };

        let band_names = ["Low", "Mid", "High"];
        let band_colors = [
            egui::Color32::from_rgb(60, 180, 120),
            egui::Color32::from_rgb(200, 180, 60),
            egui::Color32::from_rgb(180, 80, 120),
        ];

        // Crossover frequency sliders
        let mut xover_low = snap.crossover_low_mid;
        let mut xover_high = snap.crossover_mid_high;
        let mut gains = snap.band_gains;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Crossover:").size(10.0).color(egui::Color32::from_rgb(160, 165, 170)));
            ui.add(egui::Slider::new(&mut xover_low, 20.0..=2000.0).text("Low/Mid").suffix(" Hz").logarithmic(true));
            ui.add(egui::Slider::new(&mut xover_high, 100.0..=20000.0).text("Mid/High").suffix(" Hz").logarithmic(true));
        });
        if (xover_low - snap.crossover_low_mid).abs() > 0.1 || (xover_high - snap.crossover_mid_high).abs() > 0.1 {
            *mb_xover_change = Some((xover_low, xover_high));
        }

        ui.separator();

        // Band sections
        let freq_ranges = [
            format!("{:.0} Hz – {:.0} Hz", 20.0, xover_low),
            format!("{:.0} Hz – {:.0} Hz", xover_low, xover_high),
            format!("{:.0} Hz – {:.0} kHz", xover_high, self.sample_rate as f32 / 2000.0),
        ];

        egui::ScrollArea::vertical().show(ui, |ui| {
            for band_idx in 0..3 {
                let band_snap = &snap.bands[band_idx];
                let color = band_colors[band_idx];

                // Band header
                ui.horizontal(|ui| {
                    ui.colored_label(color,
                        egui::RichText::new(format!("{} Band ({})", band_names[band_idx], freq_ranges[band_idx]))
                            .strong().size(11.0));

                    // Gain slider
                    ui.label(egui::RichText::new("Gain").size(9.0).color(egui::Color32::from_rgb(140, 145, 150)));
                    if ui.add(egui::Slider::new(&mut gains[band_idx], -24.0..=12.0)
                        .suffix(" dB").show_value(true)).changed() {
                        *mb_gain_change = Some(gains);
                    }

                    // Add effect button for this band
                    let add_resp = ui.small_button("+ Add");
                    let popup_id = ui.make_persistent_id(format!("mb_add_{}", band_idx));
                    if add_resp.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(popup_id));
                    }
                    egui::popup_below_widget(ui, popup_id, &add_resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                        ui.set_min_width(180.0);
                        for (type_id, name, _) in &self.fx_registry.list_effects() {
                            if type_id == "builtin:tuner" || type_id == "builtin:multiband" { continue; }
                            if ui.button(name).clicked() {
                                *mb_add = Some((band_idx, type_id.clone()));
                                ui.memory_mut(|m| m.toggle_popup(popup_id));
                            }
                        }
                    });
                });

                // Effect cards for this band (horizontal)
                if band_snap.is_empty() {
                    ui.label(egui::RichText::new("  (empty)").size(10.0).color(egui::Color32::from_rgb(90, 95, 100)));
                } else {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let card_width = 160.0f32;
                        let card_bg = egui::Color32::from_rgb(30, 33, 38);

                        for (fx_idx, fx) in band_snap.iter().enumerate() {
                            egui::Frame::new()
                                .fill(card_bg)
                                .corner_radius(3.0)
                                .inner_margin(4.0)
                                .show(ui, |ui| {
                                    ui.set_width(card_width);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            // Move arrows
                                            let arrow_color = egui::Color32::from_rgb(90, 95, 110);
                                            let num_fx = band_snap.len();
                                            if fx_idx > 0 {
                                                if ui.add(egui::Button::new(egui::RichText::new("\u{25C0}").size(8.0).color(arrow_color)).min_size(egui::vec2(16.0, 14.0))).clicked() {
                                                    *mb_reorder = Some((band_idx, fx_idx, fx_idx - 1));
                                                }
                                            }
                                            if fx_idx + 1 < num_fx {
                                                if ui.add(egui::Button::new(egui::RichText::new("\u{25B6}").size(8.0).color(arrow_color)).min_size(egui::vec2(16.0, 14.0))).clicked() {
                                                    *mb_reorder = Some((band_idx, fx_idx, fx_idx + 1));
                                                }
                                            }

                                            // Bypass
                                            let (bp_label, bp_color) = if fx.bypassed {
                                                ("OFF", egui::Color32::from_rgb(110, 110, 110))
                                            } else {
                                                ("ON", egui::Color32::from_rgb(70, 190, 110))
                                            };
                                            if ui.add(egui::Button::new(egui::RichText::new(bp_label).size(8.0).color(bp_color)).min_size(egui::vec2(24.0, 14.0))).clicked() {
                                                mb_bypass.push((band_idx, fx_idx));
                                            }

                                            // Remove
                                            if ui.small_button(egui::RichText::new("\u{2716}").size(8.0).color(egui::Color32::from_rgb(150, 60, 60))).clicked() {
                                                *mb_remove = Some((band_idx, fx_idx));
                                            }
                                        });

                                        ui.label(egui::RichText::new(&fx.name).size(10.0).color(color));

                                        if !fx.bypassed {
                                            for (desc, val) in &fx.params {
                                                let mut v = *val;
                                                let changed = match &desc.kind {
                                                    ParamKind::Float => {
                                                        let unit = desc.unit.clone();
                                                        let name = desc.name.clone();
                                                        ui.add(egui::Slider::new(&mut v, desc.min..=desc.max)
                                                            .text(&name)
                                                            .custom_formatter(move |val, _| {
                                                                if unit.is_empty() { format!("{:.2}", val) }
                                                                else { format!("{:.1}{}", val, unit) }
                                                            })
                                                        ).changed()
                                                    }
                                                    ParamKind::Enum(labels) => {
                                                        let cur = v.round() as usize;
                                                        let cur_label = labels.get(cur).cloned().unwrap_or_default();
                                                        let mut changed = false;
                                                        egui::ComboBox::from_id_salt(format!("mb{}_{}{}", band_idx, fx_idx, desc.id.0))
                                                            .selected_text(&cur_label)
                                                            .width(90.0)
                                                            .show_ui(ui, |ui| {
                                                                for (i, l) in labels.iter().enumerate() {
                                                                    if ui.selectable_value(&mut v, i as f32, l).changed() {
                                                                        changed = true;
                                                                    }
                                                                }
                                                            });
                                                        changed
                                                    }
                                                    ParamKind::Int => {
                                                        let mut iv = v.round() as i32;
                                                        let c = ui.add(egui::Slider::new(&mut iv, desc.min as i32..=desc.max as i32).text(&desc.name)).changed();
                                                        if c { v = iv as f32; }
                                                        c
                                                    }
                                                    ParamKind::Bool => {
                                                        let mut b = v > 0.5;
                                                        let c = ui.checkbox(&mut b, &desc.name).changed();
                                                        if c { v = if b { 1.0 } else { 0.0 }; }
                                                        c
                                                    }
                                                };
                                                if changed {
                                                    mb_param.push((band_idx, fx_idx, desc.id, v));
                                                }
                                            }
                                        }
                                    });
                                });

                            if fx_idx + 1 < band_snap.len() {
                                ui.label(egui::RichText::new("\u{25B6}").size(10.0).color(egui::Color32::from_rgb(60, 65, 75)));
                            }
                        }
                    });
                }

                if band_idx < 2 {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(2.0);
                }
            }
        });
    }

    #[allow(dead_code)]
    fn draw_effects_rack(&mut self, ui: &mut egui::Ui) {
        let graph_arc = self.engine.lock().unwrap().graph().clone();

        // Snapshot effect state only when needed (dirty flag or periodic refresh).
        // This avoids locking the graph every frame which causes audio clicks.
        let needs_refresh = self.fx_snapshot_dirty
            || self.fx_snapshot_time.elapsed() > std::time::Duration::from_millis(200);
        if needs_refresh {
            if let Ok(graph) = graph_arc.try_lock() {
                let snap_chain = |chain: &rifflab_fx::chain::EffectChain| -> Vec<FxSnapCached> {
                    chain.effects().iter().map(|e| {
                        let descs = e.param_descriptors();
                        let params = descs.into_iter()
                            .filter(|d| d.name != "Bypass")
                            .map(|d| { let v = e.get_param(d.id); (d, v) })
                            .collect();
                        FxSnapCached {
                            name: e.name().to_string(),
                            type_id: e.effect_type_id().to_string(),
                            bypassed: e.is_bypassed(),
                            params,
                        }
                    }).collect()
                };

                self.fx_snapshot = snap_chain(&graph.fx_chain);
                self.fx_multiband_mode = graph.fx_multiband_active;

                if let Some(ref mb) = graph.fx_multiband {
                    self.fx_mb_snapshot = Some(MultibandSnapshot {
                        crossover_low_mid: mb.crossover_low_mid,
                        crossover_mid_high: mb.crossover_mid_high,
                        band_gains: mb.band_gains,
                        bands: [
                            snap_chain(&mb.chains[0]),
                            snap_chain(&mb.chains[1]),
                            snap_chain(&mb.chains[2]),
                        ],
                    });
                }

                self.fx_snapshot_time = std::time::Instant::now();
                self.fx_snapshot_dirty = false;
            }
        }
        let fx_snap = &self.fx_snapshot;

        // Collect mutations to apply after rendering
        let mut add_effect: Option<String> = None;
        let mut remove_idx: Option<usize> = None;
        let mut param_changes: Vec<(usize, ParamId, f32)> = Vec::new();
        let mut bypass_toggles: Vec<usize> = Vec::new();
        let mut reorder: Option<(usize, usize)> = None; // (from, to)

        // Multiband-specific mutations
        let mut mb_add: Option<(usize, String)> = None; // (band_idx, type_id)
        let mut mb_remove: Option<(usize, usize)> = None; // (band_idx, effect_idx)
        let mut mb_param: Vec<(usize, usize, ParamId, f32)> = Vec::new(); // (band, fx, param, val)
        let mut mb_bypass: Vec<(usize, usize)> = Vec::new(); // (band, fx)
        let mut mb_reorder: Option<(usize, usize, usize)> = None; // (band, from, to)
        let mut mb_xover_change: Option<(f32, f32)> = None;
        let mut mb_gain_change: Option<[f32; 3]> = None;
        let mut toggle_mode = false;

        // Preset actions (collected for after header render)
        let mut do_save = false;
        let mut do_save_as = false;
        let mut do_open = false;

        // Header
        ui.horizontal(|ui| {
            // Mode toggle
            let single_active = !self.fx_multiband_mode;
            let label_color = egui::Color32::from_rgb(200, 210, 220);
            let dim_color = egui::Color32::from_rgb(120, 125, 135);
            if ui.add(egui::Button::new(
                egui::RichText::new("Single").size(10.0).color(if single_active { egui::Color32::WHITE } else { dim_color })
            ).fill(if single_active { egui::Color32::from_rgb(50, 55, 65) } else { egui::Color32::TRANSPARENT })
             .corner_radius(2).min_size(egui::vec2(50.0, 16.0))).clicked() && !single_active {
                toggle_mode = true;
            }
            if ui.add(egui::Button::new(
                egui::RichText::new("Multiband").size(10.0).color(if !single_active { egui::Color32::WHITE } else { dim_color })
            ).fill(if !single_active { egui::Color32::from_rgb(50, 55, 65) } else { egui::Color32::TRANSPARENT })
             .corner_radius(2).min_size(egui::vec2(70.0, 16.0))).clicked() && single_active {
                toggle_mode = true;
            }

            ui.separator();

            ui.label(
                egui::RichText::new(&self.fx_preset_name)
                    .size(11.0)
                    .color(label_color),
            );

            ui.separator();

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let add_resp = ui.button("+ Add");
                let popup_id = ui.make_persistent_id("fx_add_popup");
                if add_resp.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                }
                egui::popup_below_widget(ui, popup_id, &add_resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(180.0);
                    for (type_id, name, _) in &self.fx_registry.list_effects() {
                        if type_id == "builtin:tuner" { continue; }
                        if ui.button(name).clicked() {
                            add_effect = Some(type_id.clone());
                            ui.memory_mut(|m| m.toggle_popup(popup_id));
                        }
                    }
                });

                // Preset buttons (right-to-left continues)
                if ui.small_button("Open").clicked() {
                    do_open = true;
                }
                if self.fx_preset_path.is_some() {
                    if ui.small_button("Save").clicked() {
                        do_save = true;
                    }
                }
                if ui.small_button("Save As").clicked() {
                    do_save_as = true;
                }
            });
        });

        ui.separator();

        let available_height = ui.available_height();

        if self.fx_multiband_mode {
            // ─── Multiband view ──────────────────────────────────
            self.draw_multiband_rack(ui, available_height,
                &mut mb_add, &mut mb_remove, &mut mb_param, &mut mb_bypass,
                &mut mb_reorder, &mut mb_xover_change, &mut mb_gain_change);
        } else {
        // ─── Single chain view ───────────────────────────────
        // Render effects as horizontal cards that wrap on overflow.
        // Use horizontal scroll so cards flow left-to-right.
        // (available_height already computed above)
        egui::ScrollArea::horizontal()
            .min_scrolled_height(available_height)
            .show(ui, |ui| {
            if fx_snap.is_empty() {
                ui.set_min_height(available_height);
                ui.centered_and_justified(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(100, 110, 120),
                        "No effects loaded. Click '+ Add Effect' to start.",
                    );
                });
                return;
            }

            let card_width = 170.0f32;
            let card_bg = egui::Color32::from_rgb(32, 35, 40);
            let card_bg_bypassed = egui::Color32::from_rgb(28, 30, 34);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                for (idx, fx) in fx_snap.iter().enumerate() {
                    let bg = if fx.bypassed { card_bg_bypassed } else { card_bg };
                    let num_fx = fx_snap.len();

                    egui::Frame::new()
                        .fill(bg)
                        .corner_radius(4.0)
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            // Force vertical layout inside the card
                            ui.set_width(card_width);
                            ui.vertical(|ui| {
                                // Header: move arrows + bypass + name + remove
                                ui.horizontal(|ui| {
                                    // Move left/right buttons
                                    let arrow_color = egui::Color32::from_rgb(100, 110, 130);
                                    if idx > 0 {
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("\u{25C0}").size(9.0).color(arrow_color)
                                        ).min_size(egui::vec2(18.0, 16.0))).on_hover_text("Move left").clicked() {
                                            reorder = Some((idx, idx - 1));
                                        }
                                    } else {
                                        ui.add_space(22.0);
                                    }
                                    if idx + 1 < num_fx {
                                        if ui.add(egui::Button::new(
                                            egui::RichText::new("\u{25B6}").size(9.0).color(arrow_color)
                                        ).min_size(egui::vec2(18.0, 16.0))).on_hover_text("Move right").clicked() {
                                            reorder = Some((idx, idx + 1));
                                        }
                                    } else {
                                        ui.add_space(22.0);
                                    }

                                    // Bypass toggle
                                    let (label, color) = if fx.bypassed {
                                        ("OFF", egui::Color32::from_rgb(120, 120, 120))
                                    } else {
                                        ("ON", egui::Color32::from_rgb(80, 200, 120))
                                    };
                                    if ui.add(
                                        egui::Button::new(egui::RichText::new(label).size(9.0).color(color))
                                            .min_size(egui::vec2(28.0, 16.0))
                                    ).clicked() {
                                        bypass_toggles.push(idx);
                                    }

                                    // Remove button
                                    if ui.small_button(
                                        egui::RichText::new("\u{2716}").size(9.0).color(egui::Color32::from_rgb(160, 70, 70))
                                    ).clicked() {
                                        remove_idx = Some(idx);
                                    }
                                });

                                // Effect name
                                let name_color = if fx.bypassed {
                                    egui::Color32::from_rgb(100, 100, 100)
                                } else {
                                    egui::Color32::from_rgb(220, 225, 230)
                                };
                                ui.label(egui::RichText::new(&fx.name).strong().size(11.0).color(name_color));

                                // Parameters stacked vertically
                                if !fx.bypassed {
                                    ui.add_space(2.0);
                                    for (desc, val) in &fx.params {
                                        let mut v = *val;

                                        let changed = match &desc.kind {
                                            ParamKind::Bool => {
                                                let mut b = v > 0.5;
                                                let c = ui.checkbox(&mut b, &desc.name).changed();
                                                if c { v = if b { 1.0 } else { 0.0 }; }
                                                c
                                            }
                                            ParamKind::Enum(labels) => {
                                                ui.label(egui::RichText::new(&desc.name).size(9.0).color(egui::Color32::from_rgb(140, 145, 150)));
                                                let cur = v.round() as usize;
                                                let cur_label = labels.get(cur).cloned().unwrap_or_default();
                                                let mut changed = false;
                                                egui::ComboBox::from_id_salt(format!("fx{}_{}", idx, desc.id.0))
                                                    .selected_text(&cur_label)
                                                    .width(card_width - 12.0)
                                                    .show_ui(ui, |ui| {
                                                        for (i, l) in labels.iter().enumerate() {
                                                            if ui.selectable_value(&mut v, i as f32, l).changed() {
                                                                changed = true;
                                                            }
                                                        }
                                                    });
                                                changed
                                            }
                                            ParamKind::Int => {
                                                let mut iv = v.round() as i32;
                                                let c = ui.add(
                                                    egui::Slider::new(&mut iv, desc.min as i32..=desc.max as i32)
                                                        .text(&desc.name)
                                                ).changed();
                                                if c { v = iv as f32; }
                                                c
                                            }
                                            ParamKind::Float => {
                                                let unit = desc.unit.clone();
                                                let name = desc.name.clone();
                                                ui.add(
                                                    egui::Slider::new(&mut v, desc.min..=desc.max)
                                                        .text(&name)
                                                        .custom_formatter(move |val, _| {
                                                            if unit.is_empty() { format!("{:.2}", val) }
                                                            else { format!("{:.1}{}", val, unit) }
                                                        })
                                                ).changed()
                                            }
                                        };

                                        if changed {
                                            param_changes.push((idx, desc.id, v));
                                        }
                                    }
                                }
                            });
                        });

                    // Arrow between cards showing signal flow
                    if idx + 1 < fx_snap.len() {
                        ui.label(egui::RichText::new("\u{25B6}").size(14.0).color(egui::Color32::from_rgb(80, 85, 95)));
                    }
                }
            });
        });

        } // end single-chain view else branch

        // Apply mutations (brief lock)
        let needs_lock = add_effect.is_some() || remove_idx.is_some()
            || !param_changes.is_empty() || !bypass_toggles.is_empty()
            || toggle_mode
            || mb_add.is_some() || mb_remove.is_some()
            || !mb_param.is_empty() || !mb_bypass.is_empty()
            || mb_reorder.is_some() || mb_xover_change.is_some()
            || mb_gain_change.is_some()
            || reorder.is_some();
        if needs_lock {
            if let Ok(mut graph) = graph_arc.try_lock() {
                // Add
                if let Some(type_id) = &add_effect {
                    if let Some(effect) = self.fx_registry.create_effect(type_id) {
                        graph.fx_chain.add(effect);
                    }
                }
                // Remove (do before param changes to avoid index shift issues)
                if let Some(idx) = remove_idx {
                    if idx < graph.fx_chain.len() {
                        graph.fx_chain.remove(idx);
                    }
                }
                // Bypass toggles
                for idx in &bypass_toggles {
                    if let Some(effect) = graph.fx_chain.effects_mut().get_mut(*idx) {
                        for desc in effect.param_descriptors() {
                            if desc.name == "Bypass" {
                                let cur = effect.get_param(desc.id);
                                effect.set_param(desc.id, if cur > 0.5 { 0.0 } else { 1.0 });
                                break;
                            }
                        }
                    }
                }
                // Param changes
                for (idx, param_id, val) in &param_changes {
                    if let Some(effect) = graph.fx_chain.effects_mut().get_mut(*idx) {
                        effect.set_param(*param_id, *val);
                    }
                }
                // Reorder
                if let Some((from, to)) = reorder {
                    graph.fx_chain.move_effect(from, to);
                }

                // Mode toggle
                if toggle_mode {
                    graph.fx_multiband_active = !graph.fx_multiband_active;
                    if graph.fx_multiband_active && graph.fx_multiband.is_none() {
                        graph.fx_multiband = Some(MultibandRouter::new());
                    }
                    self.fx_multiband_mode = graph.fx_multiband_active;
                }

                // Multiband mutations
                if let Some(_mb) = graph.fx_multiband.as_mut() {
                    // handled below
                }
                if let Some((band, type_id)) = &mb_add {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        if let Some(effect) = self.fx_registry.create_effect(type_id) {
                            mb.chains[*band].add(effect);
                        }
                    }
                }
                if let Some((band, fx_idx)) = mb_remove {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        if fx_idx < mb.chains[band].len() {
                            mb.chains[band].remove(fx_idx);
                        }
                    }
                }
                for (band, fx_idx, param_id, val) in &mb_param {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        if let Some(effect) = mb.chains[*band].effects_mut().get_mut(*fx_idx) {
                            effect.set_param(*param_id, *val);
                        }
                    }
                }
                for (band, fx_idx) in &mb_bypass {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        if let Some(effect) = mb.chains[*band].effects_mut().get_mut(*fx_idx) {
                            for desc in effect.param_descriptors() {
                                if desc.name == "Bypass" {
                                    let cur = effect.get_param(desc.id);
                                    effect.set_param(desc.id, if cur > 0.5 { 0.0 } else { 1.0 });
                                    break;
                                }
                            }
                        }
                    }
                }
                if let Some((band, from, to)) = mb_reorder {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        mb.chains[band].move_effect(from, to);
                    }
                }
                if let Some((low, high)) = mb_xover_change {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        mb.set_crossover_low_mid(low);
                        mb.set_crossover_mid_high(high);
                    }
                }
                if let Some(gains) = mb_gain_change {
                    if let Some(ref mut mb) = graph.fx_multiband {
                        mb.band_gains = gains;
                    }
                }

                // Mark snapshot dirty so it refreshes next frame
                self.fx_snapshot_dirty = true;
            }
        }

        // Handle preset save/load (after mutations, outside graph lock)
        if do_save {
            if let Some(ref path) = self.fx_preset_path.clone() {
                self.save_fx_preset(path, &graph_arc);
            }
        }
        if do_save_as {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("RiffLab Preset", &["toml"])
                .set_file_name(&format!("{}.toml", self.fx_preset_name))
                .save_file()
            {
                self.fx_preset_path = Some(path.clone());
                self.fx_preset_name = path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Untitled")
                    .to_string();
                self.save_fx_preset(&path, &graph_arc);
            }
        }
        if do_open {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("RiffLab Preset", &["toml"])
                .pick_file()
            {
                self.load_fx_preset(&path, &graph_arc);
            }
        }
    }

    #[allow(dead_code)]
    fn save_fx_preset(&self, path: &std::path::Path, graph_arc: &Arc<Mutex<rifflab_audio::graph::AudioGraph>>) {
        if let Ok(graph) = graph_arc.try_lock() {
            let preset = graph.fx_chain.to_preset(&self.fx_preset_name);
            match rifflab_fx::preset::save_preset(&preset, path) {
                Ok(()) => log::info!("Preset saved to {}", path.display()),
                Err(e) => log::error!("Failed to save preset: {e}"),
            }
        }
    }

    #[allow(dead_code)]
    fn load_fx_preset(&mut self, path: &std::path::Path, graph_arc: &Arc<Mutex<rifflab_audio::graph::AudioGraph>>) {
        match rifflab_fx::preset::load_preset(path) {
            Ok(preset) => {
                if let Ok(mut graph) = graph_arc.try_lock() {
                    graph.fx_chain.load_from_preset(&preset, &self.fx_registry);
                    self.fx_preset_name = preset.name.clone();
                    self.fx_preset_path = Some(path.to_path_buf());
                    self.fx_snapshot_dirty = true;
                    log::info!("Preset loaded: {} ({} effects)", preset.name, preset.effects.len());
                }
            }
            Err(e) => log::error!("Failed to load preset: {e}"),
        }
    }

    fn save_current_session(&mut self, save_as: bool) {
        let dir = if save_as || self.session_path.is_none() {
            rfd::FileDialog::new()
                .set_title("Save Session — choose folder")
                .pick_folder()
        } else {
            self.session_path.clone()
        };

        let dir = match dir {
            Some(d) => d,
            None => return,
        };

        let graph_arc = self.engine.lock().unwrap().graph().clone();
        let graph = match graph_arc.try_lock() {
            Ok(g) => g,
            Err(_) => {
                self.message_log.push("Cannot save: audio engine busy".into(), true);
                return;
            }
        };

        // Collect stem data (clone so we can release the lock before I/O)
        let stems_data: Vec<(StemType, Vec<f32>, u16, u64)> = graph.stem_players.iter()
            .map(|p| (p.stem_type, p.data().as_ref().clone(), p.channels(), p.total_frames()))
            .collect();

        let effects_preset = if !graph.fx_chain.is_empty() {
            Some(graph.fx_chain.to_preset(&self.fx_preset_name))
        } else {
            None
        };

        drop(graph); // release lock before I/O

        let cue_list = if !self.cue_engine.cue_list().is_empty() {
            Some(self.cue_engine.cue_list().clone())
        } else {
            None
        };

        let original = self.original_file_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let file_name = self.file_name.clone();
        let sample_rate = self.sample_rate;
        self.fx_graph.save_params_from_cache(&self.node_editor_state.param_cache);
        let fx_graph = self.fx_graph.clone();
        self.session_path = Some(dir.clone());
        self.message_log.push("Saving session...".into(), false);

        // Spawn background thread for I/O
        let tx = {
            let (tx, rx) = std::sync::mpsc::channel::<(bool, String)>();
            let tx_clone = tx.clone();
            std::thread::spawn(move || {
                let stems_refs: Vec<(StemType, &[f32], u16, u64)> = stems_data.iter()
                    .map(|(t, d, c, f)| (*t, d.as_slice(), *c, *f))
                    .collect();
                match session::save_session(
                    &dir, &file_name, &original, sample_rate,
                    &stems_refs, effects_preset.as_ref(), cue_list.as_ref(),
                    Some(&fx_graph),
                ) {
                    Ok(()) => { let _ = tx_clone.send((false, format!("Session saved to {}", dir.display()))); }
                    Err(e) => { let _ = tx_clone.send((true, format!("Save failed: {e}"))); }
                }
            });
            rx
        };
        // Poll save result on next frames
        // Simple approach: check once per frame until we get a message
        // Store the receiver temporarily
        self.save_receiver = Some(tx);
    }

    fn open_session(&mut self, dir: &std::path::Path) {
        match session::load_session(dir) {
            Ok(loaded) => {
                let target_rate = self.sample_rate;
                let mut stems = Vec::new();
                for (stem_type, decoded) in loaded.stems {
                    let decoded = if decoded.sample_rate != target_rate {
                        decode::resample(decoded, target_rate)
                    } else {
                        decoded
                    };
                    let color_idx = stems.len();
                    let overview = WaveformOverview::from_interleaved(
                        &decoded.data,
                        decoded.channels,
                        OVERVIEW_SAMPLES_PER_PEAK,
                        TRACK_COLORS[color_idx % TRACK_COLORS.len()],
                        format!("{:?}", stem_type),
                    );
                    stems.push((stem_type, decoded, overview));
                }

                // Load into engine
                {
                    let mut eng = self.engine.lock().unwrap();
                    eng.transport_mut().stop();

                    let mut players = Vec::new();
                    let mut max_frames: u64 = 0;
                    let mut overviews = Vec::new();
                    for (stem_type, decoded, overview) in stems {
                        let player = StemPlayer::new(stem_type, decoded.data, decoded.channels);
                        max_frames = max_frames.max(player.total_frames());
                        players.push(player);
                        overviews.push(overview);
                    }
                    eng.graph().lock().unwrap().load_stems(players);
                    eng.transport().set_length(max_frames);
                    self.waveform_overviews = overviews;
                }

                // Load effects
                {
                    let fx_ga = self.engine.lock().unwrap().graph().clone();
                    if let Some(preset) = &loaded.effects_preset {
                        if let Ok(mut g) = fx_ga.try_lock() {
                            g.fx_chain.load_from_preset(preset, &self.fx_registry);
                            self.fx_preset_name = preset.name.clone();
                            self.fx_snapshot_dirty = true;
                        }
                    }
                }

                // Load cues
                if let Some(cues) = loaded.cue_list {
                    self.cue_engine.set_cue_list(cues);
                }

                // Load effects graph and compile to audio engine
                if let Some(graph) = loaded.fx_graph {
                    self.fx_graph = graph;
                    self.fx_graph.load_params_to_cache(&mut self.node_editor_state.param_cache);
                    self.fx_graph_compiled_hash = 0;
                    self.compile_graph_if_changed();
                }

                self.file_name = loaded.manifest.name;
                self.session_path = Some(dir.to_path_buf());
                self.original_file_path = Some(std::path::PathBuf::from(&loaded.manifest.original_file));

                // Reset practice state
                self.comparator = None;
                self.reference_notes.clear();
                self.scorer = SessionScorer::new();
                self.played_notes.clear();
                self.scroll_offset_frames = 0.0;

                self.message_log.push(format!(
                    "Session opened: {} ({} stems)",
                    self.file_name,
                    self.waveform_overviews.len(),
                ), false);
            }
            Err(e) => {
                self.message_log.push(format!("Open session failed: {e}"), true);
            }
        }
    }

    /// Start editing a track's effects in the node editor.
    fn start_editing_track_fx(&mut self, track_idx: usize) {
        if let Some(Some(graph)) = self.stem_graphs.get(track_idx) {
            self.fx_graph = graph.clone();
            self.fx_graph.load_params_to_cache(&mut self.node_editor_state.param_cache);
            self.fx_graph_compiled_hash = 0;
            self.editing_track_fx = Some(track_idx);
            self.active_tab = BottomTab::Effects;
            self.message_log.push(format!("Editing FX for track {}", track_idx + 1), false);
        }
    }

    /// Save the current node editor state back to the track's FX.
    fn save_track_fx_from_editor(&mut self, track_idx: usize) {
        // Save the current graph back
        self.fx_graph.save_params_from_cache(&self.node_editor_state.param_cache);

        while self.stem_graphs.len() <= track_idx {
            self.stem_graphs.push(None);
        }
        self.stem_graphs[track_idx] = Some(self.fx_graph.clone());

        // Recompile to EffectChain
        let route = self.fx_graph.compile();
        let type_ids = match route {
            node_editor::CompiledRoute::SingleChain(ids) => ids,
            _ => Vec::new(),
        };

        let mut chain = rifflab_fx::chain::EffectChain::new();
        let effect_nodes: Vec<u64> = {
            let input = self.fx_graph.nodes.iter().find(|n| matches!(n.kind, node_editor::NodeKind::Input));
            if let Some(inp) = input {
                let start = node_editor::PortId { node_id: inp.id, index: 0, is_output: true };
                self.fx_graph.trace_chain(start).into_iter()
                    .filter(|id| self.fx_graph.find_node(*id).map_or(false, |n| matches!(n.kind, node_editor::NodeKind::Effect { .. })))
                    .collect()
            } else { Vec::new() }
        };

        for (i, type_id) in type_ids.iter().enumerate() {
            if let Some(mut effect) = self.fx_registry.create_effect(type_id) {
                if let Some(&node_id) = effect_nodes.get(i) {
                    for (&(nid, pid), &val) in &self.node_editor_state.param_cache {
                        if nid == node_id {
                            effect.set_param(ParamId(pid), val);
                        }
                    }
                }
                chain.add(effect);
            }
        }

        let graph_arc = self.engine.lock().unwrap().graph().clone();
        if let Ok(mut g) = graph_arc.try_lock() {
            while g.stem_fx.len() <= track_idx {
                g.stem_fx.push(None);
            }
            g.stem_fx[track_idx] = Some(chain);
        }

        self.message_log.push(format!("Track {} FX updated", track_idx + 1), false);
    }

    /// Load an effect graph preset onto a specific stem track.
    fn load_track_fx(&mut self, track_idx: usize, path: &std::path::Path) {
        match node_editor::load_graph(path) {
            Ok(graph) => {
                // Compile the graph to get a chain of effect type IDs
                let route = graph.compile();
                let type_ids = match route {
                    node_editor::CompiledRoute::SingleChain(ids) => ids,
                    _ => Vec::new(),
                };

                let mut chain = rifflab_fx::chain::EffectChain::new();
                for type_id in &type_ids {
                    if let Some(mut effect) = self.fx_registry.create_effect(type_id) {
                        // Apply saved params from graph nodes
                        let effect_nodes: Vec<&node_editor::FxNode> = graph.nodes.iter()
                            .filter(|n| matches!(n.kind, node_editor::NodeKind::Effect { .. }))
                            .collect();
                        // Match by position
                        let chain_idx = chain.len();
                        if let Some(node) = effect_nodes.get(chain_idx) {
                            for &(pid, val) in &node.params {
                                effect.set_param(ParamId(pid), val);
                            }
                        }
                        chain.add(effect);
                    }
                }

                let graph_arc = self.engine.lock().unwrap().graph().clone();
                if let Ok(mut g) = graph_arc.try_lock() {
                    // Ensure stem_fx is long enough
                    while g.stem_fx.len() <= track_idx {
                        g.stem_fx.push(None);
                    }
                    g.stem_fx[track_idx] = Some(chain);
                }

                // Store the graph for editing
                while self.stem_graphs.len() <= track_idx {
                    self.stem_graphs.push(None);
                }
                self.stem_graphs[track_idx] = Some(graph);

                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                self.message_log.push(format!("Track {}: loaded FX from '{}'", track_idx + 1, name), false);
            }
            Err(e) => {
                self.message_log.push(format!("Load FX failed: {e}"), true);
            }
        }
    }

    /// Load an audio file as a single track without stem separation.
    fn load_raw_audio(&mut self, path: &std::path::Path) {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Unknown").to_string();
        let target_rate = self.sample_rate;
        let path_buf = path.to_path_buf();

        self.message_log.push(format!("Loading raw: {}...", file_name), false);

        // Decode on background thread
        let (tx, rx) = std::sync::mpsc::channel();
        let name = file_name.clone();
        std::thread::spawn(move || {
            match decode::decode_file(&path_buf) {
                Ok(decoded) => {
                    let decoded = decode::resample(decoded, target_rate);
                    let _ = tx.send(Ok((name, decoded)));
                }
                Err(e) => { let _ = tx.send(Err(format!("{e}"))); }
            }
        });

        // Poll in next frames via the existing save_receiver pattern
        // For simplicity, block briefly (raw files are fast to decode)
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok((name, decoded))) => {
                let track_idx = self.waveform_overviews.len();
                let color = TRACK_COLORS[track_idx % TRACK_COLORS.len()];

                let overview = WaveformOverview::from_interleaved(
                    &decoded.data, decoded.channels, OVERVIEW_SAMPLES_PER_PEAK, color, name.clone(),
                );
                let mono = rifflab_analysis::spectrogram::downmix_to_mono(&decoded.data, decoded.channels);
                let spec = rifflab_analysis::spectrogram::compute_spectrogram(&mono, decoded.sample_rate);

                self.waveform_overviews.push(overview);
                self.spectrograms.push(SpectrogramDisplay { data: spec, color, name: name.clone() });
                self.spectrogram_texture = None;

                let graph_arc = self.engine.lock().unwrap().graph().clone();
                if let Ok(mut graph) = graph_arc.try_lock() {
                    let player = StemPlayer::new(StemType::Other, decoded.data, decoded.channels);
                    let total_frames = player.total_frames();
                    graph.stem_players.push(player);
                    graph.stem_volumes.push(1.0);
                    graph.stem_mutes.push(false);
                    graph.stem_solos.push(false);
                    self.engine.lock().unwrap().transport().set_length(total_frames);
                }
                self.file_name = name;
                self.sidebar_snapshot = None;
                self.message_log.push(format!("Loaded: {} ({:.1}s)", self.file_name, decoded.frames as f64 / target_rate as f64), false);
            }
            Ok(Err(e)) => self.message_log.push(format!("Load failed: {e}"), true),
            Err(_) => self.message_log.push("Load timed out".into(), true),
        }
    }

    /// Start recording raw input audio.
    fn start_recording(&mut self) {
        let graph_arc = self.engine.lock().unwrap().graph().clone();
        if let Ok(mut graph) = graph_arc.try_lock() {
            graph.recorded_data.clear();
            graph.recorded_channels = 2;
            graph.recording = true;
        }
        self.is_recording = true;
        self.message_log.push("Recording started".into(), false);
        log::info!("Recording started");
    }

    /// Stop recording and add the recorded audio as a new stem track.
    fn stop_recording(&mut self) {
        self.is_recording = false;

        let graph_arc = self.engine.lock().unwrap().graph().clone();
        let (data, channels, sample_rate) = {
            let Ok(mut graph) = graph_arc.try_lock() else { return };
            graph.recording = false;
            let data = std::mem::take(&mut graph.recorded_data);
            let ch = graph.recorded_channels;
            (data, ch, self.sample_rate)
        };

        if data.is_empty() {
            self.message_log.push("Recording empty, discarded".into(), false);
            return;
        }

        let frames = data.len() as u64 / channels as u64;
        let duration = frames as f64 / sample_rate as f64;
        self.message_log.push(format!("Recorded {:.1}s ({} frames)", duration, frames), false);
        log::info!("Recording stopped: {:.1}s, {}ch, {}Hz", duration, channels, sample_rate);

        // Add as a stem track
        let track_idx = self.waveform_overviews.len();
        let color = TRACK_COLORS[track_idx % TRACK_COLORS.len()];

        let overview = WaveformOverview::from_interleaved(
            &data, channels, OVERVIEW_SAMPLES_PER_PEAK, color, "Recording".to_string(),
        );

        // Compute spectrogram
        let mono = rifflab_analysis::spectrogram::downmix_to_mono(&data, channels);
        let spec = rifflab_analysis::spectrogram::compute_spectrogram(&mono, sample_rate);

        self.waveform_overviews.push(overview);
        self.spectrograms.push(SpectrogramDisplay { data: spec, color, name: "Recording".into() });
        self.spectrogram_texture = None; // invalidate cache

        // Add as stem player
        {
            let Ok(mut graph) = graph_arc.try_lock() else { return };
            let player = StemPlayer::new(StemType::Other, data, channels);
            let total_frames = player.total_frames();
            graph.stem_players.push(player);
            graph.stem_volumes.push(1.0);
            graph.stem_mutes.push(false);
            graph.stem_solos.push(false);
            // Update transport length if this recording is longer
            let eng = self.engine.lock().unwrap();
            let current_len = eng.transport().position().frame;
            if total_frames > current_len {
                eng.transport().set_length(total_frames);
            }
        }

        self.sidebar_snapshot = None; // force refresh
    }

    /// Activate a preset from the navigation graph.
    fn activate_nav_preset(&mut self, id: u64) {
        let pipeline = self.preset_nav.find_node(id).map(|n| (n.pipeline.clone(), n.name.clone()));
        if let Some((graph, name)) = pipeline {
            self.fx_graph = graph;
            self.fx_graph.load_params_to_cache(&mut self.node_editor_state.param_cache);
            self.fx_graph_compiled_hash = 0;
            self.compile_graph_if_changed();
            self.preset_nav.active_id = Some(id);
            self.editing_preset_id = Some(id);
            self.message_log.push(format!("Preset: {}", name), false);
        }
    }

    /// Activate a preset from the bank by index.
    fn activate_preset(&mut self, index: usize) {
        let slot_data = self.preset_bank.presets.get(index)
            .map(|s| (s.graph.clone(), s.name.clone()));
        if let Some((graph, name)) = slot_data {
            self.fx_graph = graph;
            self.fx_graph.load_params_to_cache(&mut self.node_editor_state.param_cache);
            self.fx_graph_compiled_hash = 0;
            self.compile_graph_if_changed();
            self.preset_bank.active_index = Some(index);
            self.message_log.push(format!("Preset: {}", name), false);
            log::info!("Activated preset: {} (slot {})", name, index);
        }
    }

    fn draw_midi_panel(&mut self, ctx: &egui::Context) {
        let mut open = self.midi_panel_open;
        egui::Window::new("MIDI & Preset Bank")
            .open(&mut open)
            .resizable(true)
            .default_width(400.0)
            .show(ctx, |ui| {
                let label_color = egui::Color32::from_rgb(180, 200, 220);

                // MIDI port selection
                ui.label(egui::RichText::new("MIDI Input").strong().color(label_color));
                ui.horizontal(|ui| {
                    let connected = self.midi_connection.is_some();
                    let port_label = if self.midi_port_names.is_empty() {
                        "No MIDI ports found".to_string()
                    } else {
                        self.midi_port_names.get(self.midi_selected_port)
                            .cloned()
                            .unwrap_or_else(|| "Select...".to_string())
                    };

                    egui::ComboBox::from_id_salt("midi_port")
                        .selected_text(&port_label)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in self.midi_port_names.iter().enumerate() {
                                ui.selectable_value(&mut self.midi_selected_port, i, name);
                            }
                        });

                    if ui.small_button("Refresh").clicked() {
                        self.midi_port_names = midi_input::list_midi_ports();
                    }

                    if connected {
                        let name = self.midi_connection.as_ref().map(|c| c.port_name.as_str()).unwrap_or("?");
                        ui.colored_label(egui::Color32::from_rgb(80, 200, 120),
                            format!("Connected: {}", name));
                        if ui.small_button("Disconnect").clicked() {
                            self.midi_connection = None;
                            self.midi_rx = None;
                        }
                    } else {
                        if ui.add_enabled(!self.midi_port_names.is_empty(),
                            egui::Button::new("Connect")).clicked()
                        {
                            match midi_input::connect(self.midi_selected_port) {
                                Ok((conn, rx)) => {
                                    // Auto-load saved mapping for this device
                                    if let Some(mapping) = MidiMapping::load(&conn.port_name) {
                                        self.midi_knob_ccs = mapping.knob_ccs;
                                        self.midi_fader_ccs = mapping.fader_ccs;
                                        self.message_log.push(format!("MIDI connected: {} (mapping loaded)", conn.port_name), false);
                                    } else {
                                        self.message_log.push(format!("MIDI connected: {}", conn.port_name), false);
                                    }
                                    self.midi_knob_last.clear();
                                    self.midi_connection = Some(conn);
                                    self.midi_rx = Some(rx);
                                }
                                Err(e) => {
                                    self.message_log.push(format!("MIDI error: {e}"), true);
                                }
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                // MIDI Controller Mapping
                ui.label(egui::RichText::new("Controller Mapping").strong().color(label_color));
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if self.midi_knob_learning {
                        ui.colored_label(egui::Color32::from_rgb(255, 200, 80),
                            format!("Turn knobs in order... ({} learned)", self.midi_knob_ccs.len()));
                        if ui.small_button("Done").clicked() {
                            self.midi_knob_learning = false;
                            self.message_log.push(format!("Learned {} knobs", self.midi_knob_ccs.len()), false);
                        }
                    } else if self.midi_fader_learning {
                        ui.colored_label(egui::Color32::from_rgb(255, 200, 80),
                            format!("Move faders in order... ({} learned, last=master)", self.midi_fader_ccs.len()));
                        if ui.small_button("Done").clicked() {
                            self.midi_fader_learning = false;
                            self.message_log.push(format!("Learned {} faders", self.midi_fader_ccs.len()), false);
                        }
                    } else {
                        if ui.button("Learn Knobs").on_hover_text("Turn MIDI knobs 1-8 in order → controls effect params").clicked() {
                            self.midi_knob_ccs.clear();
                            self.midi_knob_learning = true;
                            self.message_log.push("Turn MIDI knobs in order...".into(), false);
                        }
                        if ui.button("Learn Faders").on_hover_text("Move faders in order → tracks + input + master volume").clicked() {
                            self.midi_fader_ccs.clear();
                            self.midi_fader_learning = true;
                            self.message_log.push("Move faders in order (last=master, 2nd last=input)...".into(), false);
                        }
                    }
                });

                // Show current mappings
                if !self.midi_knob_ccs.is_empty() || !self.midi_fader_ccs.is_empty() {
                    ui.horizontal(|ui| {
                        if !self.midi_knob_ccs.is_empty() {
                            let ccs: Vec<String> = self.midi_knob_ccs.iter().map(|c| format!("{}", c)).collect();
                            ui.label(egui::RichText::new(format!("Knobs: CC {}", ccs.join(", ")))
                                .size(10.0).color(egui::Color32::from_rgb(140, 160, 140)));
                        }
                    });
                    if !self.midi_fader_ccs.is_empty() {
                        ui.horizontal(|ui| {
                            let n = self.midi_fader_ccs.len();
                            for (i, &cc) in self.midi_fader_ccs.iter().enumerate() {
                                let role = if n >= 2 && i == n - 1 {
                                    "Master".to_string()
                                } else if n >= 3 && i == n - 2 {
                                    "Input".to_string()
                                } else {
                                    format!("Track {}", i + 1)
                                };
                                ui.label(egui::RichText::new(format!("CC{}: {}", cc, role))
                                    .size(10.0).color(egui::Color32::from_rgb(140, 160, 140)));
                            }
                        });
                    }
                    // Save mapping button
                    if self.midi_connection.is_some() {
                        ui.horizontal(|ui| {
                            if ui.small_button("Save Mapping").clicked() {
                                if let Some(ref conn) = self.midi_connection {
                                    match MidiMapping::save(&conn.port_name, &self.midi_knob_ccs, &self.midi_fader_ccs) {
                                        Ok(()) => self.message_log.push(
                                            format!("Mapping saved for {}", MidiMapping::device_filename(&conn.port_name)), false),
                                        Err(e) => self.message_log.push(format!("Save failed: {e}"), true),
                                    }
                                }
                            }
                            if let Some(ref conn) = self.midi_connection {
                                ui.label(egui::RichText::new(
                                    format!("({})", MidiMapping::device_filename(&conn.port_name))
                                ).size(9.0).color(egui::Color32::from_rgb(120, 120, 120)));
                            }
                        });
                    }
                }

                ui.add_space(8.0);
                ui.separator();

                // Preset bank
                ui.label(egui::RichText::new("Preset Bank").strong().color(label_color));
                ui.add_space(4.0);

                let mut remove_idx: Option<usize> = None;
                let mut activate_idx: Option<usize> = None;

                for i in 0..self.preset_bank.presets.len() {
                    let is_active = self.preset_bank.active_index == Some(i);
                    let slot = &self.preset_bank.presets[i];

                    ui.horizontal(|ui| {
                        // Active indicator
                        let dot_color = if is_active {
                            egui::Color32::from_rgb(80, 220, 120)
                        } else {
                            egui::Color32::from_rgb(60, 65, 75)
                        };
                        let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot_rect.center(), 4.0, dot_color);

                        // Click name to activate
                        let name_color = if is_active {
                            egui::Color32::from_rgb(220, 230, 240)
                        } else {
                            egui::Color32::from_rgb(170, 175, 185)
                        };
                        if ui.add(
                            egui::Label::new(egui::RichText::new(&slot.name).size(12.0).color(name_color))
                                .sense(egui::Sense::click())
                        ).clicked() {
                            activate_idx = Some(i);
                        }

                        // CC number (editable)
                        ui.label(egui::RichText::new(format!("CC#{}", slot.midi_cc)).size(10.0)
                            .color(egui::Color32::from_rgb(130, 135, 145)));

                        // Remove button
                        if ui.small_button(
                            egui::RichText::new("\u{2716}").size(9.0).color(egui::Color32::from_rgb(150, 60, 60))
                        ).clicked() {
                            remove_idx = Some(i);
                        }
                    });
                }

                if self.preset_bank.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(100, 110, 120),
                        "No presets loaded. Click '+ Add' to load graph presets.",
                    );
                }

                ui.add_space(4.0);
                if ui.button("+ Add Preset").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("RiffLab Graph", &["json"])
                        .pick_file()
                    {
                        match self.preset_bank.add_from_file(&path) {
                            Ok(idx) => {
                                self.message_log.push(
                                    format!("Added preset: {}", self.preset_bank.presets[idx].name), false);
                            }
                            Err(e) => {
                                self.message_log.push(format!("Failed to add preset: {e}"), true);
                            }
                        }
                    }
                }

                // Apply deferred actions
                if let Some(idx) = remove_idx {
                    self.preset_bank.remove(idx);
                }
                if let Some(idx) = activate_idx {
                    self.activate_preset(idx);
                }
            });
        self.midi_panel_open = open;
    }

    fn draw_message_log(&mut self, ctx: &egui::Context) {
        let mut open = self.message_log.open;
        egui::Window::new("Message Log")
            .open(&mut open)
            .resizable(true)
            .default_width(500.0)
            .default_height(200.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("Clear").clicked() {
                        self.message_log.clear();
                    }
                    ui.label(
                        egui::RichText::new(format!("{} messages", self.message_log.entries.len()))
                            .size(10.0)
                            .color(egui::Color32::from_rgb(130, 135, 140)),
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for entry in &self.message_log.entries {
                            let color = if entry.is_error {
                                egui::Color32::from_rgb(220, 80, 80)
                            } else {
                                egui::Color32::from_rgb(180, 190, 200)
                            };
                            ui.label(egui::RichText::new(&entry.text).size(11.0).color(color));
                        }
                    });
            });
        self.message_log.open = open;
    }

    fn draw_tuner_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.tuner_settings_open;
        egui::Window::new("Tuner Settings")
            .open(&mut open)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let label_color = egui::Color32::from_rgb(180, 200, 220);

                ui.label(egui::RichText::new("Confidence Threshold").color(label_color));
                ui.add(egui::Slider::new(&mut self.tuner_min_confidence, 0.1..=0.9)
                    .step_by(0.05)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                ui.label(egui::RichText::new(
                    "How sure the detector must be. Lower = more responsive, higher = more stable."
                ).size(10.0).color(egui::Color32::from_rgb(120, 125, 130)));

                ui.add_space(4.0);

                ui.label(egui::RichText::new("Noise Floor (RMS)").color(label_color));
                if ui.add(egui::Slider::new(&mut self.tuner_noise_floor, 0.001..=0.1)
                    .logarithmic(true)
                    .custom_formatter(|v, _| {
                        let db = if v > 0.0 { 20.0 * (v as f64).log10() } else { -100.0 };
                        format!("{:.1} dB", db)
                    })).changed()
                {
                    let eng = self.engine.lock().unwrap();
                    eng.set_analysis_noise_floor(self.tuner_noise_floor);
                }
                ui.label(egui::RichText::new(
                    "Signal below this level is ignored. Raise if picking up noise."
                ).size(10.0).color(egui::Color32::from_rgb(120, 125, 130)));

                ui.add_space(4.0);

                ui.label(egui::RichText::new("Note Hold (frames)").color(label_color));
                ui.add(egui::Slider::new(&mut self.tuner_hold_frames, 1..=10));
                ui.label(egui::RichText::new(
                    "How many consistent detections before switching note. Higher = less jumpy."
                ).size(10.0).color(egui::Color32::from_rgb(120, 125, 130)));

                ui.add_space(4.0);

                ui.label(egui::RichText::new("Display Timeout (ms)").color(label_color));
                ui.add(egui::Slider::new(&mut self.tuner_timeout_ms, 200..=3000).step_by(100.0));
                ui.label(egui::RichText::new(
                    "How long the note stays visible after the signal fades."
                ).size(10.0).color(egui::Color32::from_rgb(120, 125, 130)));

                ui.add_space(8.0);

                // Live debug info
                ui.separator();
                let raw = &self.current_pitch;
                ui.label(egui::RichText::new(format!(
                    "Raw: {:.1}Hz  conf={:.2}  midi={}  cents={:.1}",
                    raw.frequency_hz, raw.confidence, raw.midi_note, raw.cents_deviation,
                )).size(10.0).color(egui::Color32::from_rgb(100, 110, 120)));
            });
        self.tuner_settings_open = open;
    }
}

// ─── Piano Roll ──────────────────────────────────────────────────────────────

/// Returns the note name for a MIDI note number (e.g. 60 -> "C4").
fn midi_note_name(midi: u8) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (midi as i32 / 12) - 1;
    let idx = (midi % 12) as usize;
    format!("{}{}", names[idx], octave)
}

/// Returns true if the MIDI note is a black key.
fn is_black_key(midi: u8) -> bool {
    matches!(midi % 12, 1 | 3 | 6 | 8 | 10)
}

/// Get the color for an accuracy bucket (played notes).
fn accuracy_color(bucket: AccuracyBucket) -> egui::Color32 {
    match bucket {
        AccuracyBucket::Perfect => egui::Color32::from_rgb(60, 200, 80),
        AccuracyBucket::Good => egui::Color32::from_rgb(220, 200, 50),
        AccuracyBucket::Acceptable => egui::Color32::from_rgb(220, 140, 40),
        AccuracyBucket::Off => egui::Color32::from_rgb(220, 60, 60),
    }
}

/// Draw the piano roll view.
#[allow(clippy::too_many_arguments)]
fn draw_piano_roll(
    ui: &mut egui::Ui,
    reference_notes: &[NoteEvent],
    played_notes: &[PlayedNote],
    scroll_offset_frames: f64,
    frames_per_pixel: f64,
    sample_rate: u32,
    position_frame: u64,
    scroll_note: f32,
) {
    let available = ui.available_size();
    if available.x < 10.0 || available.y < 10.0 {
        return;
    }

    let sr = sample_rate as f64;

    // Allocate painter for the full area
    let (response, painter) =
        ui.allocate_painter(available, egui::Sense::click_and_drag());
    let full_rect = response.rect;

    // Piano keyboard area (left strip)
    let key_rect = egui::Rect::from_min_size(
        full_rect.min,
        egui::vec2(PIANO_KEY_WIDTH, full_rect.height()),
    );

    // Note grid area (right of keyboard)
    let grid_rect = egui::Rect::from_min_max(
        egui::pos2(full_rect.left() + PIANO_KEY_WIDTH, full_rect.top()),
        full_rect.max,
    );

    // Background
    painter.rect_filled(full_rect, 0.0, egui::Color32::from_rgb(18, 20, 24));

    // Compute visible note range
    let visible_rows = (grid_rect.height() / SEMITONE_ROW_HEIGHT).ceil() as i32;
    let bottom_note = scroll_note.floor() as i32;
    let top_note = (bottom_note + visible_rows).min(127);

    // Helper: convert MIDI note to Y position (higher notes = higher on screen = lower Y)
    let note_to_y = |midi: i32| -> f32 {
        let rows_from_bottom = midi as f32 - scroll_note;
        grid_rect.bottom() - rows_from_bottom * SEMITONE_ROW_HEIGHT
    };

    // Helper: convert time (seconds) to x position in the grid
    let time_to_x = |seconds: f64| -> f32 {
        let frame = seconds * sr;
        let x = (frame - scroll_offset_frames) / frames_per_pixel;
        grid_rect.left() + x as f32
    };

    // Draw grid rows (semitone lanes)
    for midi in bottom_note.max(0)..=top_note.min(127) {
        let y_top = note_to_y(midi + 1);
        let y_bot = note_to_y(midi);
        if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
            continue;
        }
        let y_top = y_top.max(grid_rect.top());
        let y_bot = y_bot.min(grid_rect.bottom());

        let row_rect = egui::Rect::from_min_max(
            egui::pos2(grid_rect.left(), y_top),
            egui::pos2(grid_rect.right(), y_bot),
        );

        // Alternating row colors (darker for black keys)
        let bg = if is_black_key(midi as u8) {
            egui::Color32::from_rgb(14, 16, 20)
        } else {
            egui::Color32::from_rgb(22, 24, 28)
        };
        painter.rect_filled(row_rect, 0.0, bg);

        // Grid line at each semitone boundary
        painter.line_segment(
            [
                egui::pos2(grid_rect.left(), y_bot),
                egui::pos2(grid_rect.right(), y_bot),
            ],
            egui::Stroke::new(
                if midi % 12 == 0 { 0.8 } else { 0.3 },
                egui::Color32::from_rgb(40, 44, 50),
            ),
        );
    }

    // Draw reference notes (semi-transparent colored rectangles)
    let ref_color = egui::Color32::from_rgba_premultiplied(60, 120, 200, 60);
    let ref_border = egui::Color32::from_rgba_premultiplied(80, 150, 230, 120);
    for note in reference_notes {
        let x0 = time_to_x(note.onset_seconds);
        let x1 = time_to_x(note.end_seconds());
        let y_top = note_to_y(note.midi_note as i32 + 1);
        let y_bot = note_to_y(note.midi_note as i32);

        // Clip to visible grid
        if x1 < grid_rect.left() || x0 > grid_rect.right() {
            continue;
        }
        if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
            continue;
        }

        let note_rect = egui::Rect::from_min_max(
            egui::pos2(x0.max(grid_rect.left()), y_top.max(grid_rect.top())),
            egui::pos2(x1.min(grid_rect.right()), y_bot.min(grid_rect.bottom())),
        );
        painter.rect_filled(note_rect, 2.0, ref_color);
        painter.rect_stroke(note_rect, 2.0, egui::Stroke::new(1.0, ref_border), egui::StrokeKind::Inside);
    }

    // Check which reference notes were missed (no overlapping played note)
    let missed_color = egui::Color32::from_rgba_premultiplied(120, 120, 120, 100);
    for note in reference_notes {
        let has_match = played_notes.iter().any(|p| {
            p.midi_note == note.midi_note
                && p.start_seconds < note.end_seconds()
                && p.end_seconds > note.onset_seconds
        });
        if !has_match {
            let x0 = time_to_x(note.onset_seconds);
            let x1 = time_to_x(note.end_seconds());
            let y_top = note_to_y(note.midi_note as i32 + 1);
            let y_bot = note_to_y(note.midi_note as i32);

            if x1 < grid_rect.left() || x0 > grid_rect.right() {
                continue;
            }
            if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
                continue;
            }

            let note_rect = egui::Rect::from_min_max(
                egui::pos2(x0.max(grid_rect.left()), y_top.max(grid_rect.top())),
                egui::pos2(x1.min(grid_rect.right()), y_bot.min(grid_rect.bottom())),
            );
            painter.rect_stroke(
                note_rect,
                2.0,
                egui::Stroke::new(1.5, missed_color),
                egui::StrokeKind::Inside,
            );
        }
    }

    // Draw played notes (solid rectangles, color-coded by accuracy)
    for pn in played_notes {
        let x0 = time_to_x(pn.start_seconds);
        let x1 = time_to_x(pn.end_seconds);
        let y_top = note_to_y(pn.midi_note as i32 + 1);
        let y_bot = note_to_y(pn.midi_note as i32);

        // Clip
        if x1 < grid_rect.left() || x0 > grid_rect.right() {
            continue;
        }
        if y_top > grid_rect.bottom() || y_bot < grid_rect.top() {
            continue;
        }

        // Ensure minimum width of 2px so short notes are visible
        let draw_x1 = x1.max(x0 + 2.0);

        let note_rect = egui::Rect::from_min_max(
            egui::pos2(x0.max(grid_rect.left()), y_top.max(grid_rect.top()) + 1.0),
            egui::pos2(
                draw_x1.min(grid_rect.right()),
                y_bot.min(grid_rect.bottom()) - 1.0,
            ),
        );
        let color = accuracy_color(pn.accuracy);
        painter.rect_filled(note_rect, 1.0, color);
    }

    // Draw piano keyboard strip
    painter.rect_filled(key_rect, 0.0, egui::Color32::from_rgb(25, 28, 32));
    for midi in bottom_note.max(0)..=top_note.min(127) {
        let y_top = note_to_y(midi + 1);
        let y_bot = note_to_y(midi);
        if y_top > key_rect.bottom() || y_bot < key_rect.top() {
            continue;
        }
        let y_top = y_top.max(key_rect.top());
        let y_bot = y_bot.min(key_rect.bottom());

        let is_black = is_black_key(midi as u8);
        let key_bg = if is_black {
            egui::Color32::from_rgb(30, 32, 38)
        } else {
            egui::Color32::from_rgb(55, 58, 65)
        };
        let kr = egui::Rect::from_min_max(
            egui::pos2(key_rect.left(), y_top),
            egui::pos2(key_rect.right() - 1.0, y_bot),
        );
        painter.rect_filled(kr, 0.0, key_bg);

        // Note label for C notes and every octave
        if midi % 12 == 0 || (y_bot - y_top) > 10.0 {
            let name = midi_note_name(midi as u8);
            let text_color = if midi % 12 == 0 {
                egui::Color32::from_rgb(180, 185, 190)
            } else {
                egui::Color32::from_rgb(110, 115, 120)
            };
            let font = egui::FontId::monospace(9.0);
            painter.text(
                egui::pos2(key_rect.left() + 3.0, (y_top + y_bot) / 2.0),
                egui::Align2::LEFT_CENTER,
                name,
                font,
                text_color,
            );
        }

        // Separator line
        painter.line_segment(
            [
                egui::pos2(key_rect.left(), y_bot),
                egui::pos2(key_rect.right(), y_bot),
            ],
            egui::Stroke::new(0.3, egui::Color32::from_rgb(40, 44, 50)),
        );
    }

    // Right border of keyboard
    painter.line_segment(
        [
            egui::pos2(key_rect.right(), key_rect.top()),
            egui::pos2(key_rect.right(), key_rect.bottom()),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 55, 62)),
    );

    // Playhead line
    let playhead_x = time_to_x(position_frame as f64 / sr);
    if playhead_x >= grid_rect.left() && playhead_x <= grid_rect.right() {
        painter.line_segment(
            [
                egui::pos2(playhead_x, grid_rect.top()),
                egui::pos2(playhead_x, grid_rect.bottom()),
            ],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(220, 50, 50)),
        );
    }
}

// ─── Drawing helpers ─────────────────────────────────────────────────────────

/// Draw time markings on the ruler.
fn draw_time_ruler(
    painter: &egui::Painter,
    rect: egui::Rect,
    scroll_offset_frames: f64,
    frames_per_pixel: f64,
    sample_rate: u32,
) {
    let sr = sample_rate as f64;
    let view_duration_secs = rect.width() as f64 * frames_per_pixel / sr;

    // Choose a nice tick interval based on zoom level
    let target_pixels_per_tick = 80.0;
    let secs_per_tick_target = target_pixels_per_tick * frames_per_pixel / sr;

    // Snap to nice intervals: 0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60...
    let nice_intervals = [
        0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
    ];
    let secs_per_tick = nice_intervals
        .iter()
        .find(|&&i| i >= secs_per_tick_target)
        .copied()
        .unwrap_or(300.0);

    let start_secs = scroll_offset_frames / sr;
    let end_secs = start_secs + view_duration_secs;

    // Start at the first tick >= start_secs
    let first_tick = (start_secs / secs_per_tick).ceil() * secs_per_tick;

    let text_color = egui::Color32::from_rgb(160, 170, 180);
    let tick_color = egui::Color32::from_rgb(80, 85, 90);
    let minor_tick_color = egui::Color32::from_rgb(50, 55, 60);

    let mut t = first_tick;
    while t <= end_secs + secs_per_tick {
        let frame = t * sr;
        let x = rect.left() + ((frame - scroll_offset_frames) / frames_per_pixel) as f32;

        if x < rect.left() - 10.0 || x > rect.right() + 10.0 {
            t += secs_per_tick;
            continue;
        }

        // Major tick
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - 8.0),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0, tick_color),
        );

        // Time label
        let label = if t >= 60.0 {
            let mins = (t / 60.0) as u32;
            let secs = t % 60.0;
            if secs_per_tick >= 1.0 {
                format!("{}:{:02.0}", mins, secs)
            } else {
                format!("{}:{:04.1}", mins, secs)
            }
        } else if secs_per_tick >= 1.0 {
            format!("{:.0}s", t)
        } else if secs_per_tick >= 0.1 {
            format!("{:.1}s", t)
        } else {
            format!("{:.2}s", t)
        };

        painter.text(
            egui::pos2(x + 3.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::monospace(10.0),
            text_color,
        );

        // Minor ticks (subdivisions)
        let subdivs = 4;
        let minor_step = secs_per_tick / subdivs as f64;
        for s in 1..subdivs {
            let mt = t + s as f64 * minor_step;
            let mx_frame = mt * sr;
            let mx =
                rect.left() + ((mx_frame - scroll_offset_frames) / frames_per_pixel) as f32;
            if mx >= rect.left() && mx <= rect.right() {
                painter.line_segment(
                    [
                        egui::pos2(mx, rect.bottom() - 4.0),
                        egui::pos2(mx, rect.bottom()),
                    ],
                    egui::Stroke::new(0.5, minor_tick_color),
                );
            }
        }

        t += secs_per_tick;
    }

    // Bottom border of ruler
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 55, 60)),
    );
}

/// Draw a stereo peak/RMS meter in the sidebar.
fn draw_stereo_meter(ui: &mut egui::Ui, peak_l: f32, peak_r: f32, rms_l: f32, rms_r: f32) {
    let available_width = ui.available_width();
    let meter_height = 100.0;
    let bar_width = (available_width - 12.0) / 2.0;

    let (_, painter) =
        ui.allocate_painter(egui::vec2(available_width, meter_height), egui::Sense::hover());
    let rect = painter.clip_rect();

    // Left meter
    let l_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 2.0, rect.top()),
        egui::vec2(bar_width, meter_height),
    );
    draw_single_meter(&painter, l_rect, peak_l, rms_l);

    // Right meter
    let r_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + bar_width + 10.0, rect.top()),
        egui::vec2(bar_width, meter_height),
    );
    draw_single_meter(&painter, r_rect, peak_r, rms_r);

    // Labels
    painter.text(
        egui::pos2(l_rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "L",
        egui::FontId::proportional(9.0),
        egui::Color32::from_rgb(120, 120, 120),
    );
    painter.text(
        egui::pos2(r_rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "R",
        egui::FontId::proportional(9.0),
        egui::Color32::from_rgb(120, 120, 120),
    );
}

fn draw_single_meter(painter: &egui::Painter, rect: egui::Rect, peak: f32, rms: f32) {
    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(20, 22, 26));

    // RMS bar (bottom-up)
    let rms_height = rms.min(1.0) * rect.height();
    let rms_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - rms_height),
        egui::vec2(rect.width(), rms_height),
    );

    // Gradient-like coloring: green -> yellow -> red
    let rms_db = to_db(rms);
    let rms_color = if rms_db > -6.0 {
        egui::Color32::from_rgb(220, 60, 60)
    } else if rms_db > -18.0 {
        egui::Color32::from_rgb(60, 180, 80)
    } else {
        egui::Color32::from_rgb(40, 140, 60)
    };
    painter.rect_filled(rms_rect, 2.0, rms_color);

    // Peak indicator line
    let peak_y = rect.bottom() - peak.min(1.0) * rect.height();
    let peak_db = to_db(peak);
    let peak_color = if peak_db > -3.0 {
        egui::Color32::from_rgb(255, 60, 60)
    } else {
        egui::Color32::from_rgb(120, 255, 140)
    };
    painter.line_segment(
        [
            egui::pos2(rect.left(), peak_y),
            egui::pos2(rect.right(), peak_y),
        ],
        egui::Stroke::new(2.0, peak_color),
    );

    // dB scale markers
    for &db in &[-6.0f32, -12.0, -24.0, -36.0] {
        let linear = 10.0f32.powf(db / 20.0);
        let y = rect.bottom() - linear * rect.height();
        if y > rect.top() && y < rect.bottom() {
            painter.line_segment(
                [
                    egui::pos2(rect.left(), y),
                    egui::pos2(rect.left() + 3.0, y),
                ],
                egui::Stroke::new(0.5, egui::Color32::from_rgb(80, 80, 80)),
            );
        }
    }
}

fn to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -f32::INFINITY
    } else {
        20.0 * linear.log10()
    }
}

mod config;
mod decode;
mod import;
mod library;

use anyhow::Result;
use clap::Parser;
use eframe::egui;
use rifflab_audio::backend::{self, DeviceInfo};
use rifflab_audio::engine::AudioEngine;
use rifflab_audio::graph::node::StemPlayer;
use rifflab_core::analysis::{NoteEvent, PitchFrame};
use rifflab_core::metering::MeterData;
use rifflab_core::practice::AccuracyBucket;
use rifflab_core::song::StemType;
use rifflab_core::transport::{LoopRegion, TransportState};
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
const MAX_DRAWER_HEIGHT: f32 = 500.0;
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
    let library = match Library::init() {
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

/// Which tab is active in the bottom drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    PianoRoll,
    Accuracy,
    Effects,
}

/// A played note recorded during practice, for piano roll display.
#[derive(Debug, Clone)]
struct PlayedNote {
    midi_note: u8,
    start_seconds: f64,
    end_seconds: f64,
    accuracy: AccuracyBucket,
}

// ─── Background File Loading ─────────────────────────────────────────────────

/// Result of background file decoding + resampling.
struct LoadedFile {
    file_name: String,
    decoded: decode::DecodedAudio,
    overview: WaveformOverview,
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

/// Persistent load status shown in the status bar.
struct LoadStatus {
    /// Current status message.
    text: String,
    /// Whether the last operation was an error.
    is_error: bool,
    /// When the status was last updated (for auto-clear of success messages).
    timestamp: std::time::Instant,
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

    /// Get sample rates supported by the currently selected output device.
    /// Returns empty vec if "system default" is selected (meaning all common rates shown).
    fn selected_output_sample_rates(&self) -> Vec<u32> {
        if self.output_device.is_empty() {
            return Vec::new(); // default device — show all options
        }
        self.output_devices
            .iter()
            .find(|d| d.name == self.output_device)
            .map(|d| d.sample_rates.clone())
            .unwrap_or_default()
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
    /// Status message for file loading (shown in status bar).
    load_status: LoadStatus,
    /// Cached sidebar state to avoid locking every frame.
    sidebar_snapshot: Option<(usize, Vec<bool>, Vec<bool>, Vec<f32>, f32)>,
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
            active_tab: BottomTab::PianoRoll,
            reference_notes,
            played_notes: Vec::new(),
            piano_roll_scroll_note: 48.0, // C3 at bottom
            tracking_midi_note: 0,
            tracking_was_silent: true,
            pending_file: file_path.map(std::path::PathBuf::from),
            audio_settings: AudioSettings::new(&app_config),
            app_config,
            file_load_state: FileLoadState::Idle,
            load_status: LoadStatus {
                text: String::new(),
                is_error: false,
                timestamp: std::time::Instant::now(),
            },
            sidebar_snapshot: None,
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

            let decoded = if src_rate != target_rate {
                let _ = tx.send(LoadMsg::Status(format!(
                    "Resampling {}Hz -> {}Hz (sinc)...",
                    src_rate, target_rate,
                )));
                decode::resample(decoded, target_rate)
            } else {
                decoded
            };

            // Debug: dump first 5s of resampled audio to /tmp for verification
            {
                let dump_frames = (decoded.sample_rate as usize * 5).min(decoded.frames as usize);
                let dump_samples = dump_frames * decoded.channels as usize;
                let peak: f32 = decoded.data[..dump_samples].iter()
                    .map(|s| s.abs()).fold(0.0f32, f32::max);
                log::info!(
                    "[decode] Resampled: {}ch, {}Hz, {} frames, peak={:.4}, data[0..8]={:?}",
                    decoded.channels, decoded.sample_rate, decoded.frames, peak,
                    &decoded.data[..8.min(decoded.data.len())],
                );
            }

            let _ = tx.send(LoadMsg::Status("Building waveform overview...".into()));

            let overview = WaveformOverview::from_interleaved(
                &decoded.data,
                decoded.channels,
                OVERVIEW_SAMPLES_PER_PEAK,
                TRACK_COLORS[0],
                name_clone.clone(),
            );

            let _ = tx.send(LoadMsg::Done(LoadedFile {
                file_name: name_clone,
                decoded,
                overview,
            }));
        });

        self.file_name = format!("Loading {}...", file_name);
        self.load_status = LoadStatus {
            text: format!("Opening {}...", file_name),
            is_error: false,
            timestamp: std::time::Instant::now(),
        };
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
                            self.load_status = LoadStatus {
                                text: msg,
                                is_error: false,
                                timestamp: std::time::Instant::now(),
                            };
                        }
                        Ok(LoadMsg::Done(loaded)) => {
                            // Apply to engine
                            let load_msg = {
                                let mut eng = self.engine.lock().unwrap();
                                eng.transport_mut().stop();
                                let channels = loaded.decoded.channels;
                                let frames = loaded.decoded.frames;
                                let rate = loaded.decoded.sample_rate;
                                let player = StemPlayer::new(
                                    StemType::Other,
                                    loaded.decoded.data,
                                    channels,
                                );
                                let total_frames = player.total_frames();
                                eng.graph().lock().unwrap().load_stems(vec![player]);
                                eng.transport().set_length(total_frames);
                                format!(
                                    "Loaded: {} ({:.1}s, {}ch, {}Hz, {} frames)",
                                    loaded.file_name,
                                    frames as f64 / rate as f64,
                                    channels,
                                    rate,
                                    frames,
                                )
                            };

                            self.file_name = loaded.file_name;
                            self.waveform_overviews = vec![loaded.overview];

                            // Reset practice state
                            self.comparator = None;
                            self.reference_notes.clear();
                            self.scorer = SessionScorer::new();
                            self.played_notes.clear();
                            self.scroll_offset_frames = 0.0;

                            log::info!("{load_msg}");
                            self.load_status = LoadStatus {
                                text: load_msg,
                                is_error: false,
                                timestamp: std::time::Instant::now(),
                            };
                            keep_loading = false;
                            break;
                        }
                        Ok(LoadMsg::Error(e)) => {
                            log::error!("File load error: {e}");
                            self.file_name = "No file loaded".to_string();
                            self.load_status = LoadStatus {
                                text: e,
                                is_error: true,
                                timestamp: std::time::Instant::now(),
                            };
                            keep_loading = false;
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            break; // no more messages yet
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            if keep_loading {
                                // Thread died without sending Done/Error
                                self.file_name = "No file loaded".to_string();
                                self.load_status = LoadStatus {
                                    text: "File loader thread crashed".into(),
                                    is_error: true,
                                    timestamp: std::time::Instant::now(),
                                };
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

                    ui.separator();

                    // Transport controls
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
                        }
                    }

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

                        // File load status
                        if !self.load_status.text.is_empty() {
                            let is_active = matches!(self.file_load_state, FileLoadState::Loading { .. });
                            // Auto-clear success messages after 8 seconds
                            let age = self.load_status.timestamp.elapsed();
                            let show = self.load_status.is_error
                                || is_active
                                || age < std::time::Duration::from_secs(8);
                            if show {
                                ui.separator();
                                let color = if self.load_status.is_error {
                                    egui::Color32::from_rgb(220, 80, 80)
                                } else if is_active {
                                    egui::Color32::from_rgb(100, 180, 240)
                                } else {
                                    egui::Color32::from_rgb(80, 200, 120)
                                };
                                ui.label(
                                    egui::RichText::new(&self.load_status.text)
                                        .small()
                                        .color(color),
                                );
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
                            (BottomTab::PianoRoll, "Piano Roll"),
                            (BottomTab::Accuracy, "Accuracy"),
                            (BottomTab::Effects, "Effects"),
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
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new("Effects rack coming soon")
                                        .color(egui::Color32::from_rgb(100, 110, 120)),
                                );
                            });
                        }
                    }
                });
        }

        // ─── Sidebar ─────────────────────────────────────────────
        // Snapshot graph state with try_lock — never block the audio thread.
        // If we can't get the lock this frame, use stale data from last frame.
        let graph_arc = self.engine.lock().unwrap().graph().clone();
        let (num_stems, mut solos, mut mutes, mut volumes, mut master_vol) = {
            if let Ok(graph) = graph_arc.try_lock() {
                let snap = (
                    graph.stem_players.len(),
                    graph.stem_solos.clone(),
                    graph.stem_mutes.clone(),
                    graph.stem_volumes.clone(),
                    graph.master_volume,
                );
                self.sidebar_snapshot = Some((snap.0, snap.1.clone(), snap.2.clone(), snap.3.clone(), snap.4));
                snap
            } else if let Some(ref snap) = self.sidebar_snapshot {
                snap.clone()
            } else {
                (0, Vec::new(), Vec::new(), Vec::new(), 1.0)
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

                    ui.add_space(20.0);
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

        // Write back only if the user changed something (compare to snapshot)
        // Write back only if the user changed something
        let changed = self.sidebar_snapshot.as_ref().map_or(false, |snap| {
            snap.1 != solos || snap.2 != mutes || snap.3 != volumes || snap.4 != master_vol
        });
        if changed {
            if let Ok(mut graph) = graph_arc.try_lock() {
                graph.stem_solos = solos;
                graph.stem_mutes = mutes;
                graph.stem_volumes = volumes;
                graph.master_volume = master_vol;
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

                // Right-click to clear loop
                if response.secondary_clicked() {
                    let mut eng = self.engine.lock().unwrap();
                    eng.transport_mut().set_loop(None);
                }

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

            // ─── Waveform track lanes ───────────────────────────
            let track_area_height =
                available.y - RULER_HEIGHT - 10.0; // leave some margin
            let num_tracks = self.waveform_overviews.len().max(1);
            let lane_height =
                (track_area_height / num_tracks as f32).clamp(60.0, TRACK_LANE_HEIGHT);

            if self.waveform_overviews.is_empty() {
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
                    // Filter to rates supported by the selected output device
                    let available_rates = self.audio_settings.selected_output_sample_rates();
                    ui.horizontal(|ui| {
                        let rates = [44100u32, 48000, 96000];
                        for &rate in &rates {
                            let supported = available_rates.is_empty()
                                || available_rates.contains(&rate);
                            let label = format!("{} Hz", rate);
                            let mut btn = ui.add_enabled(
                                supported,
                                egui::SelectableLabel::new(
                                    self.audio_settings.sample_rate == rate,
                                    &label,
                                ),
                            );
                            if !supported {
                                btn = btn.on_disabled_hover_text("Not supported by selected device");
                            }
                            if btn.clicked() {
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
